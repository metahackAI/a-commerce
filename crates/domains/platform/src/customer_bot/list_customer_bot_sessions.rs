//! `list_customer_bot_sessions` (`platform.customer_bot.list_sessions`): shopping sessions,
//! newest first, of one run, one member or one state.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::CustomerBotSessionState;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{SESSION_SELECT, SessionRow, db, session_json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListCustomerBotSessionsInput {
    /// Only this run's sessions.
    #[schemars(regex(pattern = r"^cbr_[a-z0-9_]+$"), example = &"cbr_01jcb0")]
    pub customer_bot_run_id: Option<String>,
    /// Only sessions that shopped as this member.
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_m000042")]
    pub customer_id: Option<String>,
    /// Only sessions in this state.
    #[schemars(example = &"done")]
    pub state: Option<CustomerBotSessionState>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListCustomerBotSessions;

impl Operation for ListCustomerBotSessions {
    const META: OpMeta = OpMeta {
        address: "platform.customer_bot.list_sessions",
        tool: "list_customer_bot_sessions",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/customer-bot/sessions",
        summary: "Ailism Customer Bot's shopping sessions, newest first: who shopped, what for, and what they did. Filter by run, member or state.",
        consumes: &[],
        produces: &[Slot::many("customer_bot_session_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListCustomerBotSessionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListCustomerBotSessions {
    async fn read(&self, cx: &Cx, input: ListCustomerBotSessionsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let state = input.state.map(|s| s.as_str());
        let filter = "s.tenant_id = $1 AND ($2::text IS NULL OR s.run_id = $2)
             AND ($3::text IS NULL OR s.customer_id = $3)
             AND ($4::text IS NULL OR s.session_state = $4)";
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM platform.customer_bot_session s WHERE {filter}"
        )))
        .bind(cx.tenant.as_str())
        .bind(input.customer_bot_run_id.as_deref())
        .bind(input.customer_id.as_deref())
        .bind(state)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows: Vec<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{SESSION_SELECT} WHERE {filter} ORDER BY s.created_at DESC, s.id DESC
             LIMIT $5 OFFSET $6"
        )))
        .bind(cx.tenant.as_str())
        .bind(input.customer_bot_run_id.as_deref())
        .bind(input.customer_id.as_deref())
        .bind(state)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|s| session_json(&cx.time, s)).collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
