//! Person-only `POST /v1/platform/bot/feedback` (`platform.bot.give_feedback`): an opinion, a
//! standing directive, or a correction asked of a decision. Open feedback goes into every
//! analysis until a person closes it.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot,
    ToolError,
};
use acom_core::enums::BotFeedbackKind;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{BOT_DECISION_NOT_FOUND, db, not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GiveBotFeedbackInput {
    /// feedback: an opinion; directive: something to keep doing or never do; correction: what
    /// to change in a decision (also said in its conversation).
    #[schemars(example = &"feedback")]
    pub kind: BotFeedbackKind,
    /// What the person says, 1–1000 characters.
    #[schemars(length(min = 1, max = 1000), example = &"Reorders under 50 units are not worth a change set.")]
    pub text: String,
    /// The decision it is about; none for the bot as a whole.
    #[schemars(regex(pattern = r"^bdc_[a-z0-9_]+$"), example = &"bdc_1106_0800_1")]
    pub bot_decision_id: Option<String>,
}

/// Person-only: an operator tells the bot something.
pub struct GiveBotFeedback;

impl Operation for GiveBotFeedback {
    const META: OpMeta = OpMeta {
        address: "platform.bot.give_feedback",
        tool: "give_bot_feedback",
        group: Group::Operate,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/platform/bot/feedback",
        summary: "Tell Ailism Bot something: an opinion on a decision, a directive to follow from now on, or a correction asked of a decision.",
        consumes: &[Slot::one("bot_decision_id")],
        produces: &[Slot::one("bot_feedback_id")],
        checks: &[],
        errors: &[BOT_DECISION_NOT_FOUND, ErrorCode::INVALID_INPUT],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "human:bot",
    };
    type Input = GiveBotFeedbackInput;
    type Output = Value;
}

impl PersonOp<Cx> for GiveBotFeedback {
    async fn run(&self, tx: &mut TxCx, input: GiveBotFeedbackInput) -> Result<Value, ToolError> {
        let text = input.text.trim();
        if text.is_empty() || text.chars().count() > 1000 {
            return Err(ToolError::new(
                ErrorCode::INVALID_INPUT,
                "Write between 1 and 1000 characters.",
            )
            .detail(json!({"field": "text", "reason": "length"})));
        }
        if let Some(d) = &input.bot_decision_id {
            let known: Option<String> = sqlx::query_scalar(
                "SELECT id FROM platform.bot_decision WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tx.tenant.as_str())
            .bind(d)
            .fetch_optional(&mut *tx.tx)
            .await
            .map_err(db)?;
            if known.is_none() {
                return Err(not_found(
                    BOT_DECISION_NOT_FOUND,
                    "bot_decision",
                    d,
                    "list_bot_decisions",
                ));
            }
        }
        let id = format!("bfb_{}", acom_core::id::new_suffix());
        let by = tx.principal.subject.clone();
        let now = tx.clock.now();
        sqlx::query(
            "INSERT INTO platform.bot_feedback (tenant_id, id, decision_id, feedback_kind, text,
               feedback_state, created_by, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, 'open', $6, $7, $7)",
        )
        .bind(tx.tenant.as_str())
        .bind(&id)
        .bind(input.bot_decision_id.as_deref())
        .bind(input.kind.as_str())
        .bind(text)
        .bind(&by)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        acom_changeset::log_action(
            tx,
            acom_changeset::LogEntry {
                object: Some(("bot_feedback", &id)),
                result: Some(
                    json!({"kind": input.kind.as_str(), "decision_id": input.bot_decision_id}),
                ),
                ..acom_changeset::LogEntry::new(
                    ActionType::CREATE.0,
                    acom_core::enums::ActionStatus::Completed,
                    "give_bot_feedback",
                )
            },
        )
        .await?;
        Ok(json!({"ok": true, "id": id, "state": "open"}))
    }
}
