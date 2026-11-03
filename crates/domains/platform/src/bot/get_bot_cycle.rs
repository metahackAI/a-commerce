//! `get_bot_cycle` (`platform.bot.get_cycle`): one cycle in full — the snapshot it read, the
//! warning signs, the report with each proposal's outcome, what it cost, the decisions it opened.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{BOT_CYCLE_NOT_FOUND, db, not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetBotCycleInput {
    /// The cycle.
    #[schemars(regex(pattern = r"^bcy_[a-z0-9_]+$"), example = &"bcy_1106_0800")]
    pub bot_cycle_id: String,
}

pub struct GetBotCycle;

impl Operation for GetBotCycle {
    const META: OpMeta = OpMeta {
        address: "platform.bot.get_cycle",
        tool: "get_bot_cycle",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/bot/cycles/{bot_cycle_id}",
        summary: "One Ailism Bot cycle in full: what it read, the warning signs, its report with what became of each proposal, and what it cost.",
        consumes: &[Slot::one("bot_cycle_id")],
        produces: &[Slot::many("bot_decision_id")],
        checks: &[],
        errors: &[BOT_CYCLE_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetBotCycleInput;
    type Output = Value;
}

#[derive(Debug, sqlx::FromRow)]
struct CycleRow {
    id: String,
    bot_trigger: String,
    requested_by: Option<String>,
    cycle_state: String,
    bot_mode: String,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    snapshot: Value,
    anomalies: Value,
    report: Option<Value>,
    usage: Value,
    error: Option<String>,
}

impl ReadOp<Cx> for GetBotCycle {
    async fn read(&self, cx: &Cx, input: GetBotCycleInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row: Option<CycleRow> = sqlx::query_as(
            "SELECT id, bot_trigger, requested_by, cycle_state, bot_mode, start_time, end_time,
                    snapshot, anomalies, report, usage, error
             FROM platform.bot_cycle WHERE tenant_id = $1 AND id = $2",
        )
        .bind(cx.tenant.as_str())
        .bind(&input.bot_cycle_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let decisions: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM platform.bot_decision WHERE tenant_id = $1 AND cycle_id = $2
             ORDER BY created_at, id",
        )
        .bind(cx.tenant.as_str())
        .bind(&input.bot_cycle_id)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let c = row.ok_or_else(|| {
            not_found(
                BOT_CYCLE_NOT_FOUND,
                "bot_cycle",
                &input.bot_cycle_id,
                "list_bot_cycles",
            )
        })?;
        Ok(
            json!({"id": c.id, "trigger": c.bot_trigger, "requested_by": c.requested_by,
                  "state": c.cycle_state, "mode": c.bot_mode, "at": cx.time.format(c.start_time),
                  "end_time": c.end_time.map(|t| cx.time.format(t)), "snapshot": c.snapshot,
                  "anomalies": c.anomalies, "report": c.report, "usage": c.usage,
                  "decisions": decisions, "error": c.error}),
        )
    }
}
