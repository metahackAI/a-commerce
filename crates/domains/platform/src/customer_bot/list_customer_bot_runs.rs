//! `list_customer_bot_runs` (`platform.customer_bot.list_runs`): the runs Ailism Customer Bot
//! made, newest first.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{RUN_SELECT, RunRow, db, run_json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListCustomerBotRunsInput {
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListCustomerBotRuns;

impl Operation for ListCustomerBotRuns {
    const META: OpMeta = OpMeta {
        address: "platform.customer_bot.list_runs",
        tool: "list_customer_bot_runs",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/customer-bot/runs",
        summary: "The runs Ailism Customer Bot made, newest first: when, how many shoppers, and what they did together.",
        consumes: &[],
        produces: &[Slot::many("customer_bot_run_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListCustomerBotRunsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListCustomerBotRuns {
    async fn read(&self, cx: &Cx, input: ListCustomerBotRunsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.customer_bot_run WHERE tenant_id = $1",
        )
        .bind(cx.tenant.as_str())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows: Vec<RunRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{RUN_SELECT} WHERE r.tenant_id = $1 ORDER BY r.created_at DESC, r.id DESC
             LIMIT $2 OFFSET $3"
        )))
        .bind(cx.tenant.as_str())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|r| run_json(&cx.time, r)).collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
