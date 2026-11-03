//! `list_bot_decisions` (`platform.bot.list_decisions`): the proposals Ailism Bot opened, newest
//! first, each with the state its task and change set are in now.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::{BotArea, BotDecisionState};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{DECISION_COLUMNS, DecisionRow, db, decision_json, decisions_sql};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListBotDecisionsInput {
    /// Only these states. All when left out.
    #[schemars(example = ["waiting"])]
    pub state: Option<Vec<BotDecisionState>>,
    /// Only this area.
    #[schemars(example = &"inventory")]
    pub area: Option<BotArea>,
    /// Only what this cycle opened.
    #[schemars(regex(pattern = r"^bcy_[a-z0-9_]+$"), example = &"bcy_1106_0800")]
    pub bot_cycle_id: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListBotDecisions;

impl Operation for ListBotDecisions {
    const META: OpMeta = OpMeta {
        address: "platform.bot.list_decisions",
        tool: "list_bot_decisions",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/bot/decisions",
        summary: "The proposals Ailism Bot opened, newest first: why, what it expects, and whether the change waits for approval, ran, or was turned down.",
        consumes: &[Slot::one("bot_cycle_id")],
        produces: &[
            Slot::many("bot_decision_id"),
            Slot::many("task_id"),
            Slot::many("change_set_id"),
        ],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListBotDecisionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListBotDecisions {
    async fn read(&self, cx: &Cx, input: ListBotDecisionsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let states: Vec<&str> = input
            .state
            .unwrap_or_default()
            .iter()
            .map(|s| s.as_str())
            .collect();
        let area = input.area.map(|a| a.as_str());
        let filter = "d.tenant_id = $1 AND (cardinality($2::text[]) = 0 OR d.state = ANY($2))
            AND ($3::text IS NULL OR d.bot_area = $3) AND ($4::text IS NULL OR d.cycle_id = $4)";
        let from = decisions_sql();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {from} WHERE {filter}"
        )))
        .bind(cx.tenant.as_str())
        .bind(&states)
        .bind(area)
        .bind(input.bot_cycle_id.as_deref())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows: Vec<DecisionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {DECISION_COLUMNS} FROM {from} WHERE {filter}
             ORDER BY d.created_at DESC, d.id DESC LIMIT $5 OFFSET $6"
        )))
        .bind(cx.tenant.as_str())
        .bind(&states)
        .bind(area)
        .bind(input.bot_cycle_id.as_deref())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|d| decision_json(&cx.time, d)).collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
