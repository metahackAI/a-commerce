//! The `customer_bot` resource (docs/api/platform.md#customer_bot): Ailism Customer Bot, the
//! shoppers that keep a demo store busy. Its runs (`customer_bot_run`), each run's shopping
//! sessions (`customer_bot_session`) with every step they took (`customer_bot_step`), and the
//! persona each member shops as (`shopper_persona`); the reads over them, the person-only
//! endpoints that run it now and change its settings, and what the gateway's workers record.
//!
//! The bot never writes the store itself: a session shops through the storefront like a browser
//! (sign-in, chat, the person-only buttons), and only what it did is recorded here.

pub mod get_customer_bot_run;
pub mod get_customer_bot_session;
pub mod get_customer_bot_status;
pub mod list_customer_bot_runs;
pub mod list_customer_bot_sessions;
pub mod list_shopper_personas;
pub mod record;
pub mod run_customer_bot;
pub mod update_customer_bot_settings;

#[cfg(test)]
mod flow;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_policy::{CustomerBot, PolicySet};
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

pub const CUSTOMER_BOT_RUN_NOT_FOUND: ErrorCode =
    ErrorCode::new("customer_bot_run_not_found", ErrorClass::NotFound);
pub const CUSTOMER_BOT_SESSION_NOT_FOUND: ErrorCode =
    ErrorCode::new("customer_bot_session_not_found", ErrorClass::NotFound);

/// A run still running this long after the gateway took it lost its workers (a restart, a
/// deploy): it is closed as failed before the next one starts. Technical pacing, not a rule.
pub(crate) const STALE_RUN_MINUTES: i64 = 45;

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

fn not_found(code: ErrorCode, entity: &str, id: &str, next: &'static str) -> ToolError {
    ToolError::new(
        code,
        format!("There is no {} {id}.", entity.replace('_', " ")),
    )
    .detail(json!({"entity": entity, "id": id}))
    .next(next)
}

fn internal(message: String) -> ToolError {
    ToolError::new(ErrorCode::INTERNAL, message)
}

/// The store's customer bot settings, typed.
pub(crate) fn settings_of(policy: &PolicySet) -> Result<CustomerBot, ToolError> {
    policy.customer_bot().map_err(|e| internal(e.to_string()))
}

/// The settings as `get_customer_bot_status` and the settings endpoint answer them.
pub(crate) fn settings_json(c: &CustomerBot) -> Value {
    json!({"mode": c.mode, "every_minutes": c.every_minutes, "workers": c.workers,
           "new_account_percent": c.new_account_percent, "max_actions": c.max_actions,
           "reviews_per_session": c.reviews_per_session, "model": c.model,
           "never_shop_as": c.never_shop_as})
}

/// A run as the reads list it.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct RunRow {
    pub id: String,
    pub bot_trigger: String,
    pub requested_by: Option<String>,
    pub run_state: String,
    pub workers: i32,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub summary: Value,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub sessions: i64,
}

pub(crate) const RUN_SELECT: &str = "SELECT r.id, r.bot_trigger, r.requested_by, r.run_state,
    r.workers, r.start_time, r.end_time, r.summary, r.error, r.created_at, r.finished_at,
    (SELECT count(*) FROM platform.customer_bot_session s
      WHERE s.tenant_id = r.tenant_id AND s.run_id = r.id) AS sessions
    FROM platform.customer_bot_run r";

pub(crate) fn run_json(time: &acom_core::StoreTime, r: &RunRow) -> Value {
    json!({"id": r.id, "trigger": r.bot_trigger, "requested_by": r.requested_by,
           "state": r.run_state, "workers": r.workers, "at": time.format(r.start_time),
           "end_time": r.end_time.map(|t| time.format(t)),
           "started": time.format(r.created_at),
           "finished": r.finished_at.map(|t| time.format(t)),
           "sessions": r.sessions, "summary": r.summary, "error": r.error})
}

/// A session as the reads list it.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct SessionRow {
    pub id: String,
    pub run_id: String,
    pub customer_id: Option<String>,
    pub account_origin: String,
    pub session_state: String,
    pub persona_built: bool,
    pub goal: Option<String>,
    pub outcome: Value,
    pub conversation_ids: Value,
    pub error: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub steps: i64,
    pub persona: Option<String>,
}

pub(crate) const SESSION_SELECT: &str = "SELECT s.id, s.run_id, s.customer_id,
    s.account_origin, s.session_state, s.persona_built, s.goal, s.outcome, s.conversation_ids,
    s.error, s.start_time, s.end_time, s.created_at, s.finished_at,
    (SELECT count(*) FROM platform.customer_bot_step t
      WHERE t.tenant_id = s.tenant_id AND t.session_id = s.id) AS steps,
    (SELECT p.description FROM platform.shopper_persona p
      WHERE p.tenant_id = s.tenant_id AND p.customer_id = s.customer_id) AS persona
    FROM platform.customer_bot_session s";

pub(crate) fn session_json(time: &acom_core::StoreTime, s: &SessionRow) -> Value {
    json!({"id": s.id, "run_id": s.run_id, "customer_id": s.customer_id,
           "account": s.account_origin, "state": s.session_state,
           "persona_built": s.persona_built, "persona": s.persona, "goal": s.goal,
           "outcome": s.outcome, "conversation_ids": s.conversation_ids, "error": s.error,
           "steps": s.steps, "at": time.format(s.start_time),
           "end_time": s.end_time.map(|t| time.format(t)),
           "started": time.format(s.created_at),
           "finished": s.finished_at.map(|t| time.format(t))})
}

/// A persona as the reads list it.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct PersonaRow {
    pub id: String,
    pub customer_id: String,
    pub description: String,
    pub persona: Value,
    pub built_by_session_id: Option<String>,
    pub session_count: i32,
    pub last_session_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

pub(crate) const PERSONA_COLUMNS: &str = "id, customer_id, description, persona,
    built_by_session_id, session_count, last_session_at, created_at";

pub(crate) fn persona_json(time: &acom_core::StoreTime, p: &PersonaRow) -> Value {
    json!({"id": p.id, "customer_id": p.customer_id, "description": p.description,
           "persona": p.persona, "built_by_session_id": p.built_by_session_id,
           "sessions": p.session_count,
           "last_session": p.last_session_at.map(|t| time.format(t)),
           "created": time.format(p.created_at)})
}

/// `customer_bot.run.updated {run_id, state}`, sent when the transaction commits.
pub(crate) async fn run_event(tx: &mut TxCx, id: &str, state: &str) -> Result<(), ToolError> {
    acom_changeset::notify(
        tx,
        "customer_bot.run.updated",
        json!({"run_id": id, "state": state}),
    )
    .await
}
