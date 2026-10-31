//! The job runner (docs/backend/commerce.md §5.3, docs/api/platform.md#task).
//!
//! Scheduled work lives in `platform.job`. [`run_due`] takes the store's due jobs
//! (`done_at IS NULL AND run_at <= until`) one at a time, in `run_at` order, each under
//! `FOR UPDATE SKIP LOCKED` in its own store-scoped transaction, so two runners (the worker and a
//! demo-clock move) never run the same job. A job runs with its transaction's clock at the time
//! it came due. Success sets `done_at` and `attempts`; a failure rolls the handler's writes back,
//! records `attempts` and `last_error`, and moves `run_at` later with a growing delay.
//!
//! Watchers repeat: after a run the next one is scheduled `every_minutes` later
//! (`platform.yml watchers`). Runs a watcher missed while the clock jumped (or the worker was
//! down) are merged into one, at the latest time it would have run. What a watcher finds comes
//! back as task drafts, and this runner writes `platform.task`: it refreshes the open task of the
//! same watcher and cause, or creates one; it never makes a second. When a draft carries a fix,
//! the runner drafts a change set as `sys_watcher` and submits it for approval. It never approves
//! one: only a standing approval in the store's policy (`run_fix: standing_approval`) runs it
//! without a person, and that run can be undone until its deadline.
//!
//! Every job that runs sends `job.updated`, every task written `task.updated`.

use std::sync::Arc;

use acom_contract::{EntityRef, ErrorCode, FixDraft, JobOutcome, Registry, TaskDraft, ToolError};
use acom_core::FixedClock;
use acom_core::enums::{TaskOrigin, TaskState};
use acom_core::ids::TaskId;
use acom_policy::{RunFix, Schedule, Watchers};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::{Value, json};

use crate::change_set::{self, NewChangeSet};
use crate::{db_err, notify, preview};

/// The system actor that runs scheduled jobs.
pub const SCHEDULER: &str = "sys_scheduler";
/// The system actor that drafts a watcher's fix (`created_by` of the change set).
pub const WATCHER: &str = "sys_watcher";

/// The first retry waits this long; each further one twice as long, up to the cap. Technical
/// pacing for a failing job, not a business rule.
const RETRY_FIRST_MINUTES: i64 = 1;
const RETRY_MAX_MINUTES: i64 = 60;

/// Where a job is (`job.updated {state}`), derived from its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Waiting for `run_at`.
    Scheduled,
    /// The last run failed; it runs again at `run_at`.
    Retrying,
    Done,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Retrying => "retrying",
            Self::Done => "done",
        }
    }
}

/// A task a watcher run created or refreshed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TouchedTask {
    pub task_id: String,
    pub state: String,
    /// `true` when this run created it.
    pub created: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_set_id: Option<String>,
}

/// One job a runner took, and what came of it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RanJob {
    pub job_id: String,
    pub kind: String,
    /// When it was due, in store time; the clock it ran with.
    pub run_at: String,
    pub state: JobState,
    pub attempts: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What a non-watcher job reported (`{promotion_id, status}`).
    #[serde(skip_serializing_if = "Value::is_null")]
    pub result: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<TouchedTask>,
    /// A watcher's next run, in store time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Scheduling (pure)

/// The latest `run_at + k·every` (k ≥ 0) that is not after `until`: the one run a watcher makes
/// for every run it missed.
pub fn latest_slot(run_at: DateTime<Utc>, until: DateTime<Utc>, every: Duration) -> DateTime<Utc> {
    let step = every.num_seconds();
    if until <= run_at || step <= 0 {
        return run_at;
    }
    let missed = (until - run_at).num_seconds() / step;
    run_at + Duration::seconds(missed.saturating_mul(step))
}

/// A watcher's next run after it ran at `ran_at`: the first slot after `until`.
pub fn next_run(ran_at: DateTime<Utc>, until: DateTime<Utc>, every: Duration) -> DateTime<Utc> {
    latest_slot(ran_at, until, every) + every
}

/// How long to wait before attempt `attempts + 1` after `attempts` failed: 1, 2, 4, … minutes,
/// at most an hour.
pub fn retry_delay(attempts: i32) -> Duration {
    let doublings = u32::try_from(attempts.saturating_sub(1).clamp(0, 16)).unwrap_or(16);
    let minutes = RETRY_FIRST_MINUTES
        .saturating_mul(1_i64 << doublings)
        .min(RETRY_MAX_MINUTES);
    Duration::minutes(minutes)
}

fn minutes(every: u32) -> Duration {
    Duration::minutes(i64::from(every.max(1)))
}

fn internal(message: String) -> ToolError {
    ToolError::new(ErrorCode::INTERNAL, message)
}

// ---------------------------------------------------------------------------------------------
// Running

#[derive(Debug, sqlx::FromRow)]
struct JobRow {
    id: String,
    kind: String,
    run_at: DateTime<Utc>,
    payload: Value,
    attempts: i32,
}

/// Runs every job of `cx`'s store that is due at `until`, in `run_at` order, and returns what
/// ran. Each job is tried at most once per call, so a failing job waits for its retry time.
pub async fn run_due(
    registry: &Registry<Cx>,
    cx: &Cx,
    until: DateTime<Utc>,
) -> Result<Vec<RanJob>, ToolError> {
    let cx = cx.as_system(SCHEDULER);
    let watchers = match cx.policy.watchers() {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::error!(store = %cx.tenant, error = %e, "watcher policy is not readable");
            None
        }
    };
    prepare_watchers(registry, &cx, watchers.as_ref(), until).await?;
    let mut tried: Vec<String> = Vec::new();
    let mut ran = Vec::new();
    loop {
        let mut tx = cx.begin().await.map_err(ToolError::from)?;
        let job: Option<JobRow> = sqlx::query_as(
            "SELECT id, kind, run_at, payload, attempts FROM platform.job
             WHERE tenant_id = $1 AND done_at IS NULL AND run_at <= $2 AND NOT (id = ANY($3))
             ORDER BY run_at, id LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .bind(cx.tenant.as_str())
        .bind(until)
        .bind(&tried)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db_err)?;
        let Some(job) = job else {
            tx.commit().await.map_err(ToolError::from)?;
            break;
        };
        tried.push(job.id.clone());
        ran.push(run_one(registry, &cx, watchers.as_ref(), tx, job, until).await?);
    }
    Ok(ran)
}

/// Every enabled watcher has one pending job (a store without one gets it now), and a watcher
/// that missed runs is moved to the latest of them.
async fn prepare_watchers(
    registry: &Registry<Cx>,
    cx: &Cx,
    watchers: Option<&Watchers>,
    until: DateTime<Utc>,
) -> Result<(), ToolError> {
    let Some(watchers) = watchers else {
        return Ok(());
    };
    for meta in registry.jobs() {
        let Some(s) = meta.watcher.and_then(|k| watchers.schedule(k)) else {
            continue;
        };
        if !s.enabled {
            continue;
        }
        let mut tx = cx.begin().await.map_err(ToolError::from)?;
        lock_kind(&mut tx, meta.kind).await?;
        // A job another runner holds is still pending (done_at is set when it commits).
        let pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.job
             WHERE tenant_id = $1 AND kind = $2 AND done_at IS NULL",
        )
        .bind(cx.tenant.as_str())
        .bind(meta.kind)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db_err)?;
        if pending == 0 {
            insert_job(&mut tx, meta.kind, until, &json!({})).await?;
            tx.commit().await.map_err(ToolError::from)?;
            continue;
        }
        // Only the due ones nobody is running now; a running one keeps its time.
        let due: Vec<(String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT id, run_at FROM platform.job
             WHERE tenant_id = $1 AND kind = $2 AND done_at IS NULL AND run_at <= $3
             FOR UPDATE SKIP LOCKED",
        )
        .bind(cx.tenant.as_str())
        .bind(meta.kind)
        .bind(until)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db_err)?;
        for (id, run_at) in due {
            let latest = latest_slot(run_at, until, minutes(s.every_minutes));
            if latest != run_at {
                sqlx::query(
                    "UPDATE platform.job SET run_at = $3, updated_at = now(), version = version + 1
                     WHERE tenant_id = $1 AND id = $2",
                )
                .bind(cx.tenant.as_str())
                .bind(&id)
                .bind(latest)
                .execute(&mut *tx.tx)
                .await
                .map_err(db_err)?;
            }
        }
        tx.commit().await.map_err(ToolError::from)?;
    }
    Ok(())
}

/// One writer per job kind and store while deciding whether a kind needs its next job, so two
/// runners never schedule the same watcher twice.
async fn lock_kind(tx: &mut TxCx, kind: &str) -> Result<(), ToolError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('job_kind:' || $1 || ':' || $2))")
        .bind(tx.tenant.as_str())
        .bind(kind)
        .execute(&mut *tx.tx)
        .await
        .map_err(db_err)?;
    Ok(())
}

/// Books one run of `kind` at `run_at` under a fixed `id`, so booking the same run again does
/// nothing: a job that books its own next run (`loyalty.expire` tomorrow at midnight), or a
/// domain scheduling follow-up work. The engine owns `platform.job`; domains schedule through
/// this rather than writing the table. Returns whether the run was new.
pub async fn book(
    tx: &mut TxCx,
    id: &str,
    kind: &str,
    run_at: DateTime<Utc>,
    payload: &Value,
) -> Result<bool, ToolError> {
    let added = sqlx::query(
        "INSERT INTO platform.job (tenant_id, id, kind, run_at, payload) VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(kind)
    .bind(run_at)
    .bind(payload)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?
    .rows_affected()
        > 0;
    if added {
        job_event(tx, id, kind, JobState::Scheduled, run_at).await?;
    }
    Ok(added)
}

async fn insert_job(
    tx: &mut TxCx,
    kind: &str,
    run_at: DateTime<Utc>,
    payload: &Value,
) -> Result<String, ToolError> {
    let id = format!("job_{}", acom_core::id::new_suffix());
    sqlx::query(
        "INSERT INTO platform.job (tenant_id, id, kind, run_at, payload) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(tx.tenant.as_str())
    .bind(&id)
    .bind(kind)
    .bind(run_at)
    .bind(payload)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    job_event(tx, &id, kind, JobState::Scheduled, run_at).await?;
    Ok(id)
}

async fn job_event(
    tx: &mut TxCx,
    id: &str,
    kind: &str,
    state: JobState,
    run_at: DateTime<Utc>,
) -> Result<(), ToolError> {
    let run_at = tx.time.format(run_at);
    notify(
        tx,
        "job.updated",
        json!({"job_id": id, "kind": kind, "state": state.as_str(), "run_at": run_at}),
    )
    .await
}

async fn savepoint(tx: &mut TxCx, sql: &'static str) -> Result<(), ToolError> {
    sqlx::query(sql)
        .execute(&mut *tx.tx)
        .await
        .map_err(db_err)
        .map(|_| ())
}

/// Runs one claimed job inside its transaction and records the outcome.
async fn run_one(
    registry: &Registry<Cx>,
    cx: &Cx,
    watchers: Option<&Watchers>,
    mut tx: TxCx,
    job: JobRow,
    until: DateTime<Utc>,
) -> Result<RanJob, ToolError> {
    let clock = Arc::new(FixedClock::at(job.run_at));
    tx.clock = clock.clone();
    let job_cx = Cx {
        clock,
        ..cx.clone()
    };
    let attempts = job.attempts.saturating_add(1);
    let handler = registry.job(&job.kind).cloned();
    let watcher = handler.as_ref().and_then(|h| h.meta().watcher);
    let schedule: Option<Schedule> = watcher.and_then(|k| watchers?.schedule(k));

    savepoint(&mut tx, "SAVEPOINT job_run").await?;
    let outcome = match (&handler, watcher, schedule) {
        (None, _, _) => Err(internal(format!(
            "No handler is registered for job kind `{}`.",
            job.kind
        ))),
        (Some(_), Some(key), None) => Err(internal(format!(
            "The store policy has no readable settings for watcher `{key}` (platform.yml watchers)."
        ))),
        (Some(_), Some(_), Some(s)) if !s.enabled => {
            Ok(JobOutcome::done(json!({"skipped": "disabled"})))
        }
        (Some(h), _, _) => h.run(&mut tx, &job.payload).await,
    };
    let base = RanJob {
        job_id: job.id.clone(),
        kind: job.kind.clone(),
        run_at: cx.time.format(job.run_at),
        state: JobState::Done,
        attempts,
        error: None,
        result: Value::Null,
        tasks: Vec::new(),
        next_run_at: None,
    };

    let outcome = match outcome {
        Ok(o) => o,
        Err(e) => {
            savepoint(&mut tx, "ROLLBACK TO SAVEPOINT job_run").await?;
            let retry_at = until.max(job.run_at) + retry_delay(attempts);
            sqlx::query(
                "UPDATE platform.job SET attempts = $3, last_error = $4, run_at = $5,
                   locked_at = NULL, updated_at = now(), version = version + 1
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(cx.tenant.as_str())
            .bind(&job.id)
            .bind(attempts)
            .bind(format!("{}: {}", e.code, e.message))
            .bind(retry_at)
            .execute(&mut *tx.tx)
            .await
            .map_err(db_err)?;
            job_event(&mut tx, &job.id, &job.kind, JobState::Retrying, retry_at).await?;
            tx.commit().await.map_err(ToolError::from)?;
            tracing::warn!(store = %cx.tenant, job = %job.id, kind = %job.kind, attempts,
                           error = %e.message, "job failed; it will run again");
            return Ok(RanJob {
                state: JobState::Retrying,
                error: Some(e.message),
                next_run_at: Some(cx.time.format(retry_at)),
                ..base
            });
        }
    };
    savepoint(&mut tx, "RELEASE SAVEPOINT job_run").await?;

    let (result, drafts) = match outcome {
        JobOutcome::Done { result, events } => {
            for e in &events {
                sqlx::query(
                    "INSERT INTO platform.outbox (tenant_id, topic, object_id, payload)
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(cx.tenant.as_str())
                .bind(&e.topic)
                .bind(&e.object_id)
                .bind(&e.payload)
                .execute(&mut *tx.tx)
                .await
                .map_err(db_err)?;
            }
            (result, Vec::new())
        }
        JobOutcome::Watched(d) => (Value::Null, d),
    };
    let mut tasks = Vec::new();
    let mut fixes: Vec<(usize, FixDraft)> = Vec::new();
    if let (Some(key), Some(s)) = (watcher, schedule)
        && s.enabled
    {
        for d in &drafts {
            let (t, fix_needed) = upsert_task(&mut tx, key, d, s).await?;
            if fix_needed && let Some(f) = &d.fix {
                fixes.push((tasks.len(), f.clone()));
            }
            tasks.push(t);
        }
    }
    sqlx::query(
        "UPDATE platform.job SET done_at = $3, attempts = $4, last_error = NULL, locked_at = NULL,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(cx.tenant.as_str())
    .bind(&job.id)
    .bind(job.run_at)
    .bind(attempts)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    job_event(&mut tx, &job.id, &job.kind, JobState::Done, job.run_at).await?;
    let mut next_run_at = None;
    if let Some(s) = schedule {
        let next = next_run(job.run_at, until, minutes(s.every_minutes));
        lock_kind(&mut tx, &job.kind).await?;
        let pending: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.job
             WHERE tenant_id = $1 AND kind = $2 AND done_at IS NULL",
        )
        .bind(cx.tenant.as_str())
        .bind(&job.kind)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db_err)?;
        if pending == 0 {
            insert_job(&mut tx, &job.kind, next, &job.payload).await?;
            next_run_at = Some(cx.time.format(next));
        }
    }
    tx.commit().await.map_err(ToolError::from)?;

    // A fix drafts and maybe runs a change set; previews and runs open their own transactions,
    // so this happens after the job's work is committed. A fix that fails leaves the task
    // without a change set, and the next run tries again.
    let mut error = None;
    if let (Some(key), Some(s)) = (watcher, schedule) {
        for (i, fix) in fixes {
            let t = &mut tasks[i];
            match draft_fix(registry, &job_cx, key, s.run_fix, &t.task_id, &fix).await {
                Ok((cs, state)) => {
                    t.change_set_id = Some(cs);
                    t.state = state;
                }
                Err(e) => {
                    tracing::warn!(store = %cx.tenant, task = %t.task_id, error = %e.message,
                                   "could not draft the watcher's fix");
                    error = Some(format!(
                        "The fix for {} was not drafted: {}",
                        t.task_id, e.message
                    ));
                }
            }
        }
    }
    Ok(RanJob {
        result,
        tasks,
        next_run_at,
        error,
        ..base
    })
}

// ---------------------------------------------------------------------------------------------
// Tasks

/// `[{entity, id}]`, as `related_entities` stores entity references.
fn entities(list: &[EntityRef]) -> Value {
    Value::Array(
        list.iter()
            .map(|e| json!({"entity": e.entity, "id": e.id}))
            .collect(),
    )
}

/// Writes one finding: refreshes the open task of this watcher and cause, or creates one.
/// Returns the task and whether it still needs its fix drafted.
async fn upsert_task(
    tx: &mut TxCx,
    watcher: &str,
    d: &TaskDraft,
    s: Schedule,
) -> Result<(TouchedTask, bool), ToolError> {
    let now = tx.clock.now();
    // One writer per watcher and store: two runs never both decide the task is missing.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('task_watcher:' || $1 || ':' || $2))")
        .bind(tx.tenant.as_str())
        .bind(watcher)
        .execute(&mut *tx.tx)
        .await
        .map_err(db_err)?;
    let cause = d.cause.as_ref().map(|c| entities(std::slice::from_ref(c)));
    let open: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, task_state, change_set_id FROM platform.task
         WHERE tenant_id = $1 AND watcher = $2 AND task_state IN ('needs_you', 'working')
           AND ($3::jsonb IS NULL OR related_entities @> $3::jsonb)
         ORDER BY start_time DESC, id LIMIT 1 FOR UPDATE",
    )
    .bind(tx.tenant.as_str())
    .bind(watcher)
    .bind(&cause)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    let related = entities(&d.related);
    if let Some((id, state, cs)) = open {
        sqlx::query(
            "UPDATE platform.task SET title = $3, watcher_observation = $4, related_entities = $5,
               updated_at = $6, version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&id)
        .bind(&d.title)
        .bind(&d.observation)
        .bind(&related)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db_err)?;
        task_event(tx, &id, &state, cs.as_deref()).await?;
        let fix_needed = d.fix.is_some() && cs.is_none();
        return Ok((
            TouchedTask {
                task_id: id,
                state,
                created: false,
                change_set_id: cs,
            },
            fix_needed,
        ));
    }
    let id = TaskId::generate().to_string();
    let state = if d.fix.is_some() && s.run_fix == RunFix::StandingApproval {
        TaskState::Working
    } else {
        TaskState::NeedsYou
    };
    let origin: TaskOrigin = s.origin;
    sqlx::query(
        "INSERT INTO platform.task (tenant_id, id, title, task_state, task_origin, summary, tag,
           request_text, watcher, watcher_observation, related_entities, start_time, created_at,
           updated_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$12,$12)",
    )
    .bind(tx.tenant.as_str())
    .bind(&id)
    .bind(&d.title)
    .bind(state.as_str())
    .bind(origin.as_str())
    .bind(&d.summary)
    .bind(&d.tag)
    .bind(&d.request_text)
    .bind(watcher)
    .bind(&d.observation)
    .bind(&related)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    task_event(tx, &id, state.as_str(), None).await?;
    Ok((
        TouchedTask {
            task_id: id,
            state: state.as_str().to_owned(),
            created: true,
            change_set_id: None,
        },
        d.fix.is_some(),
    ))
}

pub(crate) async fn task_event(
    tx: &mut TxCx,
    id: &str,
    state: &str,
    change_set_id: Option<&str>,
) -> Result<(), ToolError> {
    let mut body = json!({"task_id": id, "state": state});
    if let Some(cs) = change_set_id {
        body["change_set_id"] = json!(cs);
    }
    notify(tx, "task.updated", body).await
}

/// Sets a task's state (unless it was closed meanwhile) and status line.
async fn set_task(
    cx: &Cx,
    task_id: &str,
    change_set_id: &str,
    state: Option<TaskState>,
    summary: &str,
) -> Result<String, ToolError> {
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let now = tx.clock.now();
    let row: Option<String> = sqlx::query_scalar(
        "UPDATE platform.task SET change_set_id = $3, summary = $4,
           task_state = CASE WHEN task_state IN ('needs_you', 'working') AND $5::text IS NOT NULL
                             THEN $5 ELSE task_state END,
           updated_at = $6, version = version + 1
         WHERE tenant_id = $1 AND id = $2 RETURNING task_state",
    )
    .bind(cx.tenant.as_str())
    .bind(task_id)
    .bind(change_set_id)
    .bind(summary)
    .bind(state.map(TaskState::as_str))
    .bind(now)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    let state = row.ok_or_else(|| internal(format!("Task {task_id} disappeared.")))?;
    task_event(&mut tx, task_id, &state, Some(change_set_id)).await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(state)
}

/// Drafts the change set a watcher proposes, links it to the task, and submits it. With a
/// standing approval it also runs it. Returns the change set and the task's state.
async fn draft_fix(
    registry: &Registry<Cx>,
    job_cx: &Cx,
    watcher: &str,
    run_fix: RunFix,
    task_id: &str,
    fix: &FixDraft,
) -> Result<(String, String), ToolError> {
    let cx = job_cx.as_system(WATCHER);
    let created = change_set::create(
        &cx,
        NewChangeSet {
            title: fix.title.clone(),
            request_text: vec![fix.request_text.clone()],
            interpretation: fix.interpretation.clone(),
            conversation_id: None,
        },
    )
    .await?;
    let cs = created["id"]
        .as_str()
        .ok_or_else(|| internal("a new change set has no id".into()))?
        .to_owned();
    let number = created["number"].as_i64().unwrap_or_default();
    set_task(
        &cx,
        task_id,
        &cs,
        None,
        &format!("Drafting change set #{number}"),
    )
    .await?;
    let drafted = async {
        for call in &fix.calls {
            preview::preview(registry, &cx, call.tool, call.input.clone(), Some(&cs)).await?;
        }
        change_set::submit(&cx, &cs, 1).await
    }
    .await;
    if let Err(e) = drafted {
        let note = format!(
            "Could not draft the fix in change set #{number}: {}",
            e.message
        );
        set_task(&cx, task_id, &cs, Some(TaskState::NeedsYou), &note).await?;
        return Err(e);
    }
    match run_fix {
        RunFix::Ask => {
            let state = set_task(
                &cx,
                task_id,
                &cs,
                Some(TaskState::NeedsYou),
                &format!("Change set #{number} is waiting for your approval"),
            )
            .await?;
            Ok((cs, state))
        }
        RunFix::StandingApproval => {
            let key = format!("watchers.{watcher}.run_fix");
            let out = match change_set::run_with_standing_approval(registry, &cx, &cs, &key).await {
                Ok(out) => out,
                Err(e) => {
                    let note = format!("Change set #{number} could not run: {}", e.message);
                    set_task(&cx, task_id, &cs, Some(TaskState::NeedsYou), &note).await?;
                    return Err(e);
                }
            };
            let at = cx.clock.now().with_timezone(&cx.time.zone);
            let summary = if out["state"] == "live" {
                let until = out["undo_until"]
                    .as_str()
                    .and_then(|u| DateTime::parse_from_rfc3339(u).ok())
                    .map(|u| format!("; undo until {}", u.format("%b %-d")))
                    .unwrap_or_default();
                format!("{} at {}{until}", fix.done_text, at.format("%H:%M"))
            } else {
                format!("Change set #{number} did not go live; review what failed")
            };
            let state = (out["state"] != "live").then_some(TaskState::NeedsYou);
            let state = set_task(&cx, task_id, &cs, state, &summary).await?;
            Ok((cs, state))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn a_watcher_that_missed_runs_makes_only_the_latest() {
        let every = Duration::minutes(30);
        // Friday 11:10 → Saturday 10:00: 45 slots missed, one run at 09:40, the next at 10:10.
        let at = latest_slot(
            t("2026-11-06T11:10:00-05:00"),
            t("2026-11-07T10:00:00-05:00"),
            every,
        );
        assert_eq!(at, t("2026-11-07T09:40:00-05:00"));
        assert_eq!(
            next_run(at, t("2026-11-07T10:00:00-05:00"), every),
            t("2026-11-07T10:10:00-05:00")
        );
        // Due exactly now, or not yet due: it stays where it is.
        let due = t("2026-11-06T11:10:00-05:00");
        assert_eq!(latest_slot(due, due, every), due);
        assert_eq!(latest_slot(due, t("2026-11-06T11:00:00-05:00"), every), due);
        assert_eq!(latest_slot(due, t("2026-11-06T11:39:59-05:00"), every), due);
    }

    #[test]
    fn the_next_run_is_always_after_the_clock() {
        let every = Duration::minutes(60);
        let ran = t("2026-11-06T11:40:00-05:00");
        assert_eq!(next_run(ran, ran, every), t("2026-11-06T12:40:00-05:00"));
        let until = t("2026-11-09T09:00:00-05:00");
        let next = next_run(latest_slot(ran, until, every), until, every);
        assert!(next > until && next - until <= every);
        assert_eq!(next, t("2026-11-09T09:40:00-05:00"));
    }

    #[test]
    fn retries_wait_longer_each_time_up_to_an_hour() {
        let waits: Vec<i64> = (1..=9).map(|a| retry_delay(a).num_minutes()).collect();
        assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(retry_delay(0).num_minutes(), 1);
        assert_eq!(retry_delay(i32::MAX).num_minutes(), 60);
    }

    #[test]
    fn job_states_are_words() {
        assert_eq!(
            serde_json::to_value(JobState::Retrying).unwrap(),
            json!("retrying")
        );
        assert_eq!(JobState::Done.as_str(), "done");
    }
}
