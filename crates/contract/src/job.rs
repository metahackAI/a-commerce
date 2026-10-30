//! Scheduled work (`platform.job`): what a domain registers next to its operations.
//!
//! A job kind (`promotion.start`, `watcher.stock_out_forecast`) has one handler in the domain
//! that owns its data. The engine (`acom-changeset::jobs`) takes due jobs in `run_at` order, runs
//! each handler inside its own store-scoped transaction, and records the outcome. A watcher is a
//! job that looks at the store and reports what crosses a policy threshold as task drafts; the
//! engine writes `platform.task` from them, so no domain writes another domain's table.
//! Design: `docs/backend/commerce.md` §5.3, `docs/api/platform.md#task`.

use std::future::Future;

use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::Value;

use crate::error::ToolError;
use crate::op::Ctx;

/// Static description of one kind of job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct JobMeta {
    /// What `platform.job.kind` holds: `{domain or watcher}.{verb}`, for example
    /// `promotion.start` or `watcher.price_below_cost`.
    pub kind: &'static str,
    /// One line on what a run does.
    pub summary: &'static str,
    /// For a watcher: its key under `platform.yml watchers`. The engine reads `enabled`,
    /// `every_minutes`, `origin` and `run_fix` there and schedules the next run itself.
    pub watcher: Option<&'static str>,
}

/// One entity a task is about, as `platform.task.related_entities` stores it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntityRef {
    /// The data-model entity name (`claim`, `product`).
    pub entity: String,
    pub id: String,
}

impl EntityRef {
    pub fn new(entity: &str, id: impl Into<String>) -> Self {
        Self {
            entity: entity.to_owned(),
            id: id.into(),
        }
    }
}

/// One write a watcher proposes, previewed by the engine as the system.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FixCall {
    /// The tool name (`set_price`).
    pub tool: &'static str,
    /// Its input, as the model would send it.
    pub input: Value,
}

/// The change set a watcher proposes to fix what it found. The engine drafts it and submits it
/// for approval; it never approves it. Only a standing approval in the store's policy
/// (`watchers.<key>.run_fix: standing_approval`) lets it run without a person.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FixDraft {
    pub title: String,
    /// The sentence the change set records as its request.
    pub request_text: String,
    /// `[{slot, text, value}]`, as a change set's `interpretation`.
    pub interpretation: Vec<Value>,
    /// The writes, in the order they run.
    pub calls: Vec<FixCall>,
    /// What the task says once the fix ran, before the time ("Raised to the price floor").
    pub done_text: String,
}

/// What a watcher found for one cause: the engine creates a task from it, or refreshes the open
/// task of the same watcher and cause.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskDraft {
    /// What the finding is about when a watcher reports several at once (one product of
    /// `stock_out_forecast`); `None` for one task per watcher. Matched against the open task's
    /// related entities.
    pub cause: Option<EntityRef>,
    /// One line for the home list.
    pub title: String,
    /// The current status line of a new task.
    pub summary: String,
    /// Area label (`returns`, `inventory`, `pricing`).
    pub tag: String,
    /// The sentence the agent starts a turn with when the task is opened.
    pub request_text: String,
    /// `{threshold: {policy_key, value}, observed: {...}}`, stored as `watcher_observation`.
    pub observation: Value,
    /// The entities the task is about.
    pub related: Vec<EntityRef>,
    /// A change set that would fix it, when the watcher's policy names a fix.
    pub fix: Option<FixDraft>,
}

/// A domain event a job produced, for `platform.outbox` (topic `promotion.started`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DomainEvent {
    pub topic: String,
    /// The aggregate it is about (`promo_denim_bundle`).
    pub object_id: String,
    pub payload: Value,
}

/// What one run of a job did.
#[derive(Debug, Clone, PartialEq)]
pub enum JobOutcome {
    /// The work is done. `result` is what the runner reports (`{promotion_id, status}`);
    /// `events` go to the outbox in the job's transaction.
    Done {
        result: Value,
        events: Vec<DomainEvent>,
    },
    /// A watcher looked: one draft per cause over its threshold, empty when nothing is.
    Watched(Vec<TaskDraft>),
}

impl JobOutcome {
    /// Done, with a result and no events.
    pub fn done(result: Value) -> Self {
        Self::Done {
            result,
            events: Vec::new(),
        }
    }
}

/// A job handler. Runs inside the job's transaction, with the transaction's clock at the time
/// the job came due; must be safe to run again after a failure (the engine retries).
pub trait Job<C: Ctx>: Send + Sync + 'static {
    const META: JobMeta;

    fn run(
        &self,
        tx: &mut C::Tx,
        payload: &Value,
    ) -> impl Future<Output = Result<JobOutcome, ToolError>> + Send;
}

/// A job handler with its type erased.
pub trait DynJob<C: Ctx>: Send + Sync {
    fn meta(&self) -> &'static JobMeta;
    fn run<'a>(
        &'a self,
        tx: &'a mut C::Tx,
        payload: &'a Value,
    ) -> BoxFuture<'a, Result<JobOutcome, ToolError>>;
}

pub(crate) struct JobAdapter<J>(pub J);

impl<C: Ctx, J: Job<C>> DynJob<C> for JobAdapter<J> {
    fn meta(&self) -> &'static JobMeta {
        &J::META
    }
    fn run<'a>(
        &'a self,
        tx: &'a mut C::Tx,
        payload: &'a Value,
    ) -> BoxFuture<'a, Result<JobOutcome, ToolError>> {
        Box::pin(self.0.run(tx, payload))
    }
}

/// Whether a kind is well formed: lowercase words joined by `_`, two parts joined by `.`.
pub(crate) fn kind_is_valid(kind: &str) -> bool {
    let mut parts = kind.split('.');
    let ok = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                && !p.starts_with('_')
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_two_words_joined_by_a_dot() {
        assert!(kind_is_valid("promotion.start"));
        assert!(kind_is_valid("watcher.stock_out_forecast"));
        assert!(!kind_is_valid("promotion"));
        assert!(!kind_is_valid("Promotion.start"));
        assert!(!kind_is_valid("a.b.c"));
        assert!(!kind_is_valid("watcher._x"));
    }
}
