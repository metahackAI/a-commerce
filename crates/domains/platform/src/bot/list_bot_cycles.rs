//! `list_bot_cycles` (`platform.bot.list_cycles`): the cycles Ailism Bot ran, newest first.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::db;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListBotCyclesInput {
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListBotCycles;

impl Operation for ListBotCycles {
    const META: OpMeta = OpMeta {
        address: "platform.bot.list_cycles",
        tool: "list_bot_cycles",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/bot/cycles",
        summary: "The cycles Ailism Bot ran, newest first: when, why, what it found and how many proposals it opened.",
        consumes: &[],
        produces: &[Slot::many("bot_cycle_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListBotCyclesInput;
    type Output = Value;
}

#[derive(Debug, sqlx::FromRow)]
struct CycleRow {
    id: String,
    bot_trigger: String,
    cycle_state: String,
    bot_mode: String,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    report: Option<Value>,
    anomalies: Value,
    error: Option<String>,
    opened: i64,
}

impl ReadOp<Cx> for ListBotCycles {
    async fn read(&self, cx: &Cx, input: ListBotCyclesInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM platform.bot_cycle WHERE tenant_id = $1")
                .bind(cx.tenant.as_str())
                .fetch_one(&mut *tx.tx)
                .await
                .map_err(db)?;
        let rows: Vec<CycleRow> = sqlx::query_as(
            "SELECT c.id, c.bot_trigger, c.cycle_state, c.bot_mode, c.start_time, c.end_time,
                    c.report, c.anomalies, c.error,
                    (SELECT count(*) FROM platform.bot_decision d
                      WHERE d.tenant_id = c.tenant_id AND d.cycle_id = c.id) AS opened
             FROM platform.bot_cycle c WHERE c.tenant_id = $1
             ORDER BY c.start_time DESC, c.id DESC LIMIT $2 OFFSET $3",
        )
        .bind(cx.tenant.as_str())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|c| {
                let proposals = c
                    .report
                    .as_ref()
                    .and_then(|r| r.get("proposals"))
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                json!({"id": c.id, "trigger": c.bot_trigger, "state": c.cycle_state,
                       "mode": c.bot_mode, "at": cx.time.format(c.start_time),
                       "end_time": c.end_time.map(|t| cx.time.format(t)),
                       "summary": c.report.as_ref().and_then(|r| r.get("summary")).cloned(),
                       "proposals": proposals, "opened": c.opened,
                       "anomalies": c.anomalies.as_array().map_or(0, Vec::len),
                       "error": c.error})
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
