//! `list_bot_feedback` (`platform.bot.list_feedback`): what people told Ailism Bot, newest
//! first. Each analysis reads the open ones.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::BotFeedbackState;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{FEEDBACK_COLUMNS, FeedbackRow, db, feedback_json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListBotFeedbackInput {
    /// Only open or only closed feedback. All when left out.
    #[schemars(example = &"open")]
    pub state: Option<BotFeedbackState>,
    /// Only what was left on this decision.
    #[schemars(regex(pattern = r"^bdc_[a-z0-9_]+$"), example = &"bdc_1106_0800_1")]
    pub bot_decision_id: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListBotFeedback;

impl Operation for ListBotFeedback {
    const META: OpMeta = OpMeta {
        address: "platform.bot.list_feedback",
        tool: "list_bot_feedback",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/bot/feedback",
        summary: "What people told Ailism Bot — opinions, standing directives and corrections — newest first.",
        consumes: &[Slot::one("bot_decision_id")],
        produces: &[Slot::many("bot_feedback_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListBotFeedbackInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListBotFeedback {
    async fn read(&self, cx: &Cx, input: ListBotFeedbackInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let state = input.state.map(|s| s.as_str());
        let filter = "tenant_id = $1 AND ($2::text IS NULL OR feedback_state = $2)
            AND ($3::text IS NULL OR decision_id = $3)";
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM platform.bot_feedback WHERE {filter}"
        )))
        .bind(cx.tenant.as_str())
        .bind(state)
        .bind(input.bot_decision_id.as_deref())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows: Vec<FeedbackRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {FEEDBACK_COLUMNS} FROM platform.bot_feedback WHERE {filter}
             ORDER BY created_at DESC, id DESC LIMIT $4 OFFSET $5"
        )))
        .bind(cx.tenant.as_str())
        .bind(state)
        .bind(input.bot_decision_id.as_deref())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|f| feedback_json(&cx.time, f)).collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
