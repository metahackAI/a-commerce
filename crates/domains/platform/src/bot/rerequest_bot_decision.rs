//! Person-only `POST /v1/platform/bot/decisions/{bot_decision_id}/rerequest`
//! (`platform.bot.rerequest`): ask a decision again with a correction. The change set still
//! waiting is rejected with the correction as its reason, the correction is kept as feedback,
//! and a new decision with the same proposal and the correction opens a new task for the
//! operations agent to draft again.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot,
    ToolError,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::report::{Proposal, open_decision};
use super::{BOT_DECISION_NOT_FOUND, db, decisions_sql, fresh_policy, not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RerequestBotDecisionInput {
    /// The decision to ask again.
    #[schemars(regex(pattern = r"^bdc_[a-z0-9_]+$"), example = &"bdc_1106_0800_1")]
    pub bot_decision_id: String,
    /// What to change, 1–1000 characters.
    #[schemars(length(min = 1, max = 1000), example = &"Order 30 of each size, not 80.")]
    pub text: String,
}

/// Person-only: an operator asks a decision again with a correction.
pub struct RerequestBotDecision;

impl Operation for RerequestBotDecision {
    const META: OpMeta = OpMeta {
        address: "platform.bot.rerequest",
        tool: "rerequest_bot_decision",
        group: Group::Operate,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/platform/bot/decisions/{bot_decision_id}/rerequest",
        summary: "Ask an Ailism Bot decision again with a correction: its waiting change set is rejected with the correction, and a new draft is made with it.",
        consumes: &[Slot::one("bot_decision_id")],
        produces: &[Slot::one("bot_decision_id"), Slot::one("task_id")],
        checks: &[],
        errors: &[
            BOT_DECISION_NOT_FOUND,
            ErrorCode::INVALID_STATE,
            ErrorCode::INVALID_INPUT,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "human:bot",
    };
    type Input = RerequestBotDecisionInput;
    type Output = Value;
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    cycle_id: String,
    task_id: String,
    bot_area: String,
    title: String,
    request_text: String,
    reason: String,
    evidence: Value,
    expected_effect: Value,
    bot_risk: String,
    related_entities: Value,
    cause_key: String,
    state: String,
    change_set_id: Option<String>,
}

impl PersonOp<Cx> for RerequestBotDecision {
    async fn run(
        &self,
        tx: &mut TxCx,
        input: RerequestBotDecisionInput,
    ) -> Result<Value, ToolError> {
        let text = input.text.trim().to_owned();
        if text.is_empty() || text.chars().count() > 1000 {
            return Err(ToolError::new(
                ErrorCode::INVALID_INPUT,
                "Say what to change in 1 to 1000 characters.",
            )
            .detail(json!({"field": "text", "reason": "length"})));
        }
        let row: Option<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT d.cycle_id, d.task_id, d.bot_area, d.title, d.request_text, d.reason,
                    d.evidence, d.expected_effect, d.bot_risk, d.related_entities, d.cause_key,
                    d.state, d.change_set_id
             FROM {} WHERE d.tenant_id = $1 AND d.id = $2",
            decisions_sql()
        )))
        .bind(tx.tenant.as_str())
        .bind(&input.bot_decision_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let d = row.ok_or_else(|| {
            not_found(
                BOT_DECISION_NOT_FOUND,
                "bot_decision",
                &input.bot_decision_id,
                "list_bot_decisions",
            )
        })?;
        if matches!(d.state.as_str(), "running" | "live") {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                "This change already ran. Undo it from its change set, or leave feedback.",
            )
            .detail(json!({"entity": "bot_decision", "from": d.state})));
        }
        let waiting =
            matches!(d.state.as_str(), "drafting" | "waiting") && d.change_set_id.is_some();
        if waiting && !tx.principal.has_scope("human:reject") {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                "Asking again rejects the waiting change set, which needs the permission to reject.",
            )
            .detail(json!({"reason": "scope", "required_scope": "human:reject"})));
        }
        let by = tx.principal.subject.clone();
        let now = tx.clock.now();
        let feedback = format!("bfb_{}", acom_core::id::new_suffix());
        sqlx::query(
            "INSERT INTO platform.bot_feedback (tenant_id, id, decision_id, feedback_kind, text,
               feedback_state, created_by, created_at, updated_at)
             VALUES ($1, $2, $3, 'correction', $4, 'open', $5, $6, $6)",
        )
        .bind(tx.tenant.as_str())
        .bind(&feedback)
        .bind(&input.bot_decision_id)
        .bind(&text)
        .bind(&by)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        let reason = format!("Asked again: {text}");
        let mut rejected = None;
        if waiting && let Some(cs) = &d.change_set_id {
            acom_changeset::change_set::reject_in(tx, cs, &reason, true).await?;
            rejected = Some(cs.clone());
        }
        // The old task is closed either way: the new one carries the work.
        sqlx::query(
            "UPDATE platform.task SET task_state = 'dismissed', end_time = $3, summary = $4,
               updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2 AND task_state IN ('needs_you', 'working')",
        )
        .bind(tx.tenant.as_str())
        .bind(&d.task_id)
        .bind(now)
        .bind(&reason)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;

        let policy = fresh_policy(tx).await?;
        let origin = policy
            .watchers()
            .map(|w| w.ailism_bot.origin.as_str())
            .unwrap_or("agent_noticed");
        let about: Vec<(String, String)> = d
            .related_entities
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|a| {
                Some((
                    a.get("entity")?.as_str()?.to_owned(),
                    a.get("id")?.as_str()?.to_owned(),
                ))
            })
            .collect();
        let proposal = Proposal {
            area: d.bot_area,
            title: d.title,
            request: format!(
                "{}\nThe operator asked for a change: {text}",
                d.request_text
            )
            .chars()
            .take(1000)
            .collect(),
            reason: d.reason,
            evidence: d.evidence,
            expected_effect: d.expected_effect,
            risk: d.bot_risk,
            about,
        };
        let fresh = acom_core::id::new_suffix();
        let suffix = format!(
            "{}_r{}",
            input.bot_decision_id.trim_start_matches("bdc_"),
            &fresh[fresh.len().saturating_sub(6)..]
        );
        let (decision, task) = open_decision(
            tx,
            &d.cycle_id,
            &suffix,
            &proposal,
            &d.cause_key,
            origin,
            Some(&input.bot_decision_id),
        )
        .await?;
        acom_changeset::log_action(
            tx,
            acom_changeset::LogEntry {
                object: Some(("bot_decision", &input.bot_decision_id)),
                result: Some(json!({"asked_again_as": decision, "task_id": task,
                                    "rejected_change_set": rejected, "feedback_id": feedback})),
                ..acom_changeset::LogEntry::new(
                    ActionType::UPDATE.0,
                    acom_core::enums::ActionStatus::Completed,
                    "rerequest_bot_decision",
                )
            },
        )
        .await?;
        Ok(
            json!({"ok": true, "id": decision, "task_id": task, "revises": input.bot_decision_id,
                  "rejected_change_set": rejected, "feedback_id": feedback}),
        )
    }
}
