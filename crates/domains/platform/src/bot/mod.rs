//! The `bot` resource (docs/api/platform.md#bot): Ailism Bot, the back-office manager AI. Its
//! cycles (`bot_cycle`), the proposals it opened as tasks (`bot_decision`), and what people told
//! it (`bot_feedback`); the reads over them, the person-only endpoints that run it now, change its
//! settings and give it feedback, the scheduled cycle, and the report a cycle ends with.
//!
//! The bot never writes the store: it opens tasks, the operations agent drafts their change
//! sets, and a person decides. A decision's state is the state of its task and change set,
//! computed when read.

pub mod close_bot_feedback;
pub mod get_bot_cycle;
pub mod get_bot_decision;
pub mod get_bot_status;
pub mod give_bot_feedback;
pub mod list_bot_cycles;
pub mod list_bot_decisions;
pub mod list_bot_feedback;
pub mod report;
pub mod rerequest_bot_decision;
pub mod run_bot_cycle;
pub mod update_bot_settings;
pub mod watch;

#[cfg(test)]
mod flow;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_core::enums::BotTrigger;
use acom_policy::watchers::AilismBotWatch;
use acom_policy::{Bot, PolicySet};
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

pub const BOT_CYCLE_NOT_FOUND: ErrorCode =
    ErrorCode::new("bot_cycle_not_found", ErrorClass::NotFound);
pub const BOT_DECISION_NOT_FOUND: ErrorCode =
    ErrorCode::new("bot_decision_not_found", ErrorClass::NotFound);
pub const BOT_FEEDBACK_NOT_FOUND: ErrorCode =
    ErrorCode::new("bot_feedback_not_found", ErrorClass::NotFound);

/// A cycle still `observing` after this long lost its report (the agent or gateway stopped):
/// it is closed as failed when the next one opens. Technical pacing, not a business rule.
const STALE_CYCLE_MINUTES: i64 = 30;

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

/// The store's bot settings and its schedule, typed.
pub(crate) fn settings_of(policy: &PolicySet) -> Result<(Bot, AilismBotWatch), ToolError> {
    let bot = policy.bot().map_err(|e| internal(e.to_string()))?;
    let watch = policy
        .watchers()
        .map_err(|e| internal(e.to_string()))?
        .ailism_bot;
    Ok((bot, watch))
}

/// The settings as `get_bot_status` and the settings endpoint answer them.
pub(crate) fn settings_json(bot: &Bot, watch: &AilismBotWatch) -> Value {
    let areas: serde_json::Map<String, Value> = bot
        .areas
        .all()
        .iter()
        .map(|(a, on)| ((*a).to_owned(), json!(on)))
        .collect();
    json!({"mode": bot.mode, "acts_as": bot.acts_as, "model": bot.model,
           "every_minutes": watch.every_minutes, "areas": areas,
           "max_proposals_per_cycle": bot.max_proposals_per_cycle,
           "max_open_decisions": bot.max_open_decisions, "cooldown_days": bot.cooldown_days,
           "directives": bot.directives})
}

/// The store's policy with its latest `platform` override applied again, so a setting saved a
/// moment ago reads back at once (the store's policy is cached for a short while).
pub(crate) async fn fresh_policy(tx: &mut TxCx) -> Result<PolicySet, ToolError> {
    let partial: Option<Value> = sqlx::query_scalar(
        "SELECT partial FROM platform.policy_override WHERE tenant_id = $1 AND file = 'platform'",
    )
    .bind(tx.tenant.as_str())
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    match partial {
        Some(p) => tx
            .policy
            .with_overrides([("platform", p.to_string().as_str())])
            .map_err(|e| internal(e.to_string())),
        None => Ok((*tx.policy).clone()),
    }
}

/// Opens a cycle, or returns the one still observing. A cycle left observing longer than
/// [`STALE_CYCLE_MINUTES`] is closed as failed first. `(id, created)`.
pub(crate) async fn open_cycle(
    tx: &mut TxCx,
    trigger: BotTrigger,
    requested_by: Option<&str>,
    mode: &str,
) -> Result<(String, bool), ToolError> {
    let now = tx.clock.now();
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('bot_cycle:' || $1))")
        .bind(tx.tenant.as_str())
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    let open: Option<(String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, start_time FROM platform.bot_cycle
         WHERE tenant_id = $1 AND cycle_state = 'observing' ORDER BY start_time DESC LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    if let Some((id, at)) = open {
        if now - at < Duration::minutes(STALE_CYCLE_MINUTES) {
            return Ok((id, false));
        }
        sqlx::query(
            "UPDATE platform.bot_cycle SET cycle_state = 'failed', end_time = $3,
               error = 'No report came back.', updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&id)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        cycle_event(tx, &id, "failed", None).await?;
    }
    let id = format!("bcy_{}", acom_core::id::new_suffix());
    sqlx::query(
        "INSERT INTO platform.bot_cycle (tenant_id, id, bot_trigger, requested_by, cycle_state,
           bot_mode, start_time)
         VALUES ($1, $2, $3, $4, 'observing', $5, $6)",
    )
    .bind(tx.tenant.as_str())
    .bind(&id)
    .bind(trigger.as_str())
    .bind(requested_by)
    .bind(mode)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    cycle_event(tx, &id, "observing", Some(trigger.as_str())).await?;
    Ok((id, true))
}

/// `bot.cycle.updated {cycle_id, state, trigger}`, sent when the transaction commits.
pub(crate) async fn cycle_event(
    tx: &mut TxCx,
    id: &str,
    state: &str,
    trigger: Option<&str>,
) -> Result<(), ToolError> {
    acom_changeset::notify(
        tx,
        "bot.cycle.updated",
        json!({"cycle_id": id, "state": state, "trigger": trigger}),
    )
    .await
}

/// The SQL of a decision's state from its task (`t`) and change set (`c`), as
/// `acom:bot_decision_state`.
pub(crate) const DECISION_STATE: &str = "CASE
      WHEN t.task_state = 'dismissed' THEN 'dismissed'
      WHEN c.change_set_state IS NULL THEN
        CASE WHEN t.conversation_id IS NULL OR t.task_state = 'working' THEN 'drafting'
             ELSE 'needs_you' END
      WHEN c.change_set_state = 'drafting' THEN 'drafting'
      WHEN c.change_set_state = 'waiting_for_approval' THEN 'waiting'
      WHEN c.change_set_state IN ('running', 'verifying') THEN 'running'
      ELSE c.change_set_state END";

/// A decision with what its task and change set say now.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct DecisionRow {
    pub id: String,
    pub cycle_id: String,
    pub task_id: String,
    pub bot_area: String,
    pub title: String,
    pub request_text: String,
    pub reason: String,
    pub evidence: Value,
    pub expected_effect: Value,
    pub bot_risk: String,
    pub related_entities: Value,
    pub revises_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub state: String,
    pub summary: Option<String>,
    pub change_set_id: Option<String>,
    pub change_set_number: Option<i32>,
    pub conversation_id: Option<String>,
    pub feedback: i64,
}

/// The decisions with their state, as a subquery `d` the reads filter.
pub(crate) fn decisions_sql() -> String {
    format!(
        "(SELECT d.tenant_id, d.id, d.cycle_id, d.task_id, d.bot_area, d.title, d.request_text,
            d.reason, d.evidence, d.expected_effect, d.bot_risk, d.related_entities, d.cause_key,
            d.revises_id, d.created_at,
            greatest(d.updated_at, t.updated_at, coalesce(c.updated_at, t.updated_at)) AS updated_at,
            {DECISION_STATE} AS state, t.summary, t.change_set_id, c.number AS change_set_number,
            t.conversation_id,
            (SELECT count(*) FROM platform.bot_feedback f
              WHERE f.tenant_id = d.tenant_id AND f.decision_id = d.id) AS feedback
         FROM platform.bot_decision d
         JOIN platform.task t ON t.tenant_id = d.tenant_id AND t.id = d.task_id
         LEFT JOIN platform.change_set c ON c.tenant_id = t.tenant_id AND c.id = t.change_set_id) d"
    )
}

pub(crate) const DECISION_COLUMNS: &str = "id, cycle_id, task_id, bot_area, title, request_text,
    reason, evidence, expected_effect, bot_risk, related_entities, revises_id, created_at, updated_at, state,
    summary, change_set_id, change_set_number, conversation_id, feedback";

pub(crate) fn decision_json(time: &acom_core::StoreTime, d: &DecisionRow) -> Value {
    json!({"id": d.id, "cycle_id": d.cycle_id, "task_id": d.task_id,
           "change_set_id": d.change_set_id, "change_set_number": d.change_set_number,
           "conversation_id": d.conversation_id, "area": d.bot_area, "title": d.title,
           "request": d.request_text, "reason": d.reason, "evidence": d.evidence,
           "expected_effect": d.expected_effect, "risk": d.bot_risk, "about": d.related_entities,
           "revises": d.revises_id, "state": d.state, "summary": d.summary, "feedback": d.feedback,
           "at": time.format(d.created_at), "updated_at": time.format(d.updated_at)})
}

/// One feedback row.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct FeedbackRow {
    pub id: String,
    pub feedback_kind: String,
    pub text: String,
    pub decision_id: Option<String>,
    pub feedback_state: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

pub(crate) const FEEDBACK_COLUMNS: &str =
    "id, feedback_kind, text, decision_id, feedback_state, created_by, created_at, closed_at";

pub(crate) fn feedback_json(time: &acom_core::StoreTime, f: &FeedbackRow) -> Value {
    json!({"id": f.id, "kind": f.feedback_kind, "text": f.text, "decision_id": f.decision_id,
           "state": f.feedback_state, "by": f.created_by, "at": time.format(f.created_at),
           "closed_at": f.closed_at.map(|t| time.format(t))})
}

/// What makes two proposals about the same thing: their area and the entities they name, or
/// their title's words when they name none.
pub(crate) fn cause_key(area: &str, about: &[(String, String)], title: &str) -> String {
    let mut key = if about.is_empty() {
        title
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        let mut ids: Vec<String> = about.iter().map(|(e, id)| format!("{e}:{id}")).collect();
        ids.sort();
        ids.dedup();
        ids.join(",")
    };
    key.truncate(380);
    format!("{area}|{key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cause_is_the_area_and_what_it_names() {
        let about = vec![
            ("product".to_owned(), "prd_b".to_owned()),
            ("product".to_owned(), "prd_a".to_owned()),
        ];
        assert_eq!(
            cause_key("pricing", &about, "Anything"),
            "pricing|product:prd_a,product:prd_b"
        );
        assert_eq!(
            cause_key("community", &[], "Hide the 2 reported posts!"),
            "community|hide the 2 reported posts"
        );
    }
}
