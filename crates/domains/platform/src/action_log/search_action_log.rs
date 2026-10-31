//! `search_action_log` (`platform.action_log.search`): actions by who, tool, record, set, state
//! and time.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::enums::ActionStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{COLUMNS, Row, action, db};
use crate::parse_time;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchActionLogInput {
    /// Who acted: a model name, or a person or system id.
    #[schemars(example = &"adm_sj")]
    pub agent: Option<String>,
    /// The tool used.
    #[schemars(example = &"set_sale_prices")]
    pub instrument: Option<String>,
    /// The record acted on.
    #[schemars(example = &"prd_windbreaker")]
    pub object_id: Option<String>,
    /// The change set the action belongs to.
    #[schemars(regex(pattern = r"^cs_[a-z0-9_]+$"), example = &"cs_1042")]
    pub change_set_id: Option<String>,
    /// Only actions in this state.
    #[schemars(example = &"completed")]
    pub action_status: Option<ActionStatus>,
    /// Started at or after this time (store time).
    #[schemars(example = &"2026-11-06T00:00")]
    pub start_time_from: Option<String>,
    /// Started at or before this time (store time).
    #[schemars(example = &"2026-11-06T23:59")]
    pub start_time_through: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchActionLog;

impl Operation for SearchActionLog {
    const META: OpMeta = OpMeta {
        address: "platform.action_log.search",
        tool: "search_action_log",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/action-logs",
        summary: "Find actions in the log by who, which tool, which record, which change set, state and time. For audits, disputes, and usage numbers.",
        consumes: &[],
        produces: &[Slot::many("action_id")],
        checks: &[],
        errors: &[ErrorCode::FORBIDDEN],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchActionLogInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchActionLog {
    async fn read(&self, cx: &Cx, input: SearchActionLogInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let from = parse_time(cx, "start_time_from", input.start_time_from.as_deref())?;
        let through = parse_time(
            cx,
            "start_time_through",
            input.start_time_through.as_deref(),
        )?;
        let filter = "tenant_id = $1
            AND ($2::text IS NULL OR agent_id = $2 OR agent_name = $2)
            AND ($3::text IS NULL OR instrument = $3)
            AND ($4::text IS NULL OR object_id = $4)
            AND ($5::text IS NULL OR change_set_id = $5)
            AND ($6::text IS NULL OR action_status = $6)
            AND ($7::timestamptz IS NULL OR start_time >= $7)
            AND ($8::timestamptz IS NULL OR start_time <= $8)";
        let status = input.action_status.map(|s| s.as_str());
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM platform.action_log WHERE {filter}"
        )))
        .bind(cx.tenant.as_str())
        .bind(input.agent.as_deref())
        .bind(input.instrument.as_deref())
        .bind(input.object_id.as_deref())
        .bind(input.change_set_id.as_deref())
        .bind(status)
        .bind(from)
        .bind(through)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM platform.action_log WHERE {filter}
             ORDER BY start_time DESC, id DESC LIMIT $9 OFFSET $10"
        )))
        .bind(cx.tenant.as_str())
        .bind(input.agent.as_deref())
        .bind(input.instrument.as_deref())
        .bind(input.object_id.as_deref())
        .bind(input.change_set_id.as_deref())
        .bind(status)
        .bind(from)
        .bind(through)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|r| action(cx, r)).collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
