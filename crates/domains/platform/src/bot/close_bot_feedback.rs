//! Person-only `POST /v1/platform/bot/feedback/{bot_feedback_id}/close`
//! (`platform.bot.close_feedback`): the bot no longer reads it.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot, ToolError,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{BOT_FEEDBACK_NOT_FOUND, db, not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseBotFeedbackInput {
    /// The feedback to close.
    #[schemars(regex(pattern = r"^bfb_[a-z0-9_]+$"), example = &"bfb_01")]
    pub bot_feedback_id: String,
}

/// Person-only: an operator closes feedback the bot has taken in.
pub struct CloseBotFeedback;

impl Operation for CloseBotFeedback {
    const META: OpMeta = OpMeta {
        address: "platform.bot.close_feedback",
        tool: "close_bot_feedback",
        group: Group::Operate,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/platform/bot/feedback/{bot_feedback_id}/close",
        summary: "Close feedback left for Ailism Bot, so its analyses no longer read it.",
        consumes: &[Slot::one("bot_feedback_id")],
        produces: &[],
        checks: &[],
        errors: &[BOT_FEEDBACK_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "human:bot",
    };
    type Input = CloseBotFeedbackInput;
    type Output = Value;
}

impl PersonOp<Cx> for CloseBotFeedback {
    async fn run(&self, tx: &mut TxCx, input: CloseBotFeedbackInput) -> Result<Value, ToolError> {
        let state: Option<String> = sqlx::query_scalar(
            "SELECT feedback_state FROM platform.bot_feedback
             WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(tx.tenant.as_str())
        .bind(&input.bot_feedback_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let state = state.ok_or_else(|| {
            not_found(
                BOT_FEEDBACK_NOT_FOUND,
                "bot_feedback",
                &input.bot_feedback_id,
                "list_bot_feedback",
            )
        })?;
        if state == "open" {
            let by = tx.principal.subject.clone();
            let now = tx.clock.now();
            sqlx::query(
                "UPDATE platform.bot_feedback SET feedback_state = 'closed', closed_by = $3,
                   closed_at = $4, updated_at = now(), version = version + 1
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tx.tenant.as_str())
            .bind(&input.bot_feedback_id)
            .bind(&by)
            .bind(now)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
            acom_changeset::log_action(
                tx,
                acom_changeset::LogEntry {
                    object: Some(("bot_feedback", &input.bot_feedback_id)),
                    result: Some(json!({"state": "closed"})),
                    ..acom_changeset::LogEntry::new(
                        ActionType::UPDATE.0,
                        acom_core::enums::ActionStatus::Completed,
                        "close_bot_feedback",
                    )
                },
            )
            .await?;
        }
        Ok(json!({"ok": true, "id": input.bot_feedback_id, "state": "closed"}))
    }
}
