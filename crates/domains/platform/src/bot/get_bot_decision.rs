//! `get_bot_decision` (`platform.bot.get_decision`): one decision with all the feedback left on it.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    BOT_DECISION_NOT_FOUND, DECISION_COLUMNS, DecisionRow, FEEDBACK_COLUMNS, FeedbackRow, db,
    decision_json, decisions_sql, feedback_json, not_found,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetBotDecisionInput {
    /// The decision.
    #[schemars(regex(pattern = r"^bdc_[a-z0-9_]+$"), example = &"bdc_1106_0800_1")]
    pub bot_decision_id: String,
}

pub struct GetBotDecision;

impl Operation for GetBotDecision {
    const META: OpMeta = OpMeta {
        address: "platform.bot.get_decision",
        tool: "get_bot_decision",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/bot/decisions/{bot_decision_id}",
        summary: "One Ailism Bot decision: what it proposed and why, where its change is now, and the feedback people left on it.",
        consumes: &[Slot::one("bot_decision_id")],
        produces: &[Slot::one("task_id"), Slot::one("change_set_id")],
        checks: &[],
        errors: &[BOT_DECISION_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetBotDecisionInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetBotDecision {
    async fn read(&self, cx: &Cx, input: GetBotDecisionInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row: Option<DecisionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DECISION_COLUMNS} FROM {} WHERE d.tenant_id = $1 AND d.id = $2",
            decisions_sql()
        )))
        .bind(cx.tenant.as_str())
        .bind(&input.bot_decision_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let feedback: Vec<FeedbackRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {FEEDBACK_COLUMNS} FROM platform.bot_feedback
             WHERE tenant_id = $1 AND decision_id = $2 ORDER BY created_at, id"
        )))
        .bind(cx.tenant.as_str())
        .bind(&input.bot_decision_id)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let d = row.ok_or_else(|| {
            not_found(
                BOT_DECISION_NOT_FOUND,
                "bot_decision",
                &input.bot_decision_id,
                "list_bot_decisions",
            )
        })?;
        let mut out = decision_json(&cx.time, &d);
        out["feedback"] = json!(
            feedback
                .iter()
                .map(|f| feedback_json(&cx.time, f))
                .collect::<Vec<_>>()
        );
        Ok(out)
    }
}
