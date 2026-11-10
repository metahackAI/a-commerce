//! `get_customer_bot_status` (`platform.customer_bot.get`): Ailism Customer Bot now: its
//! settings, the run working, the last run, when the next one starts, and what it has done.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_policy::CustomerBotMode;
use acom_store::Cx;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{RUN_SELECT, RunRow, db, run_json, settings_json, settings_of};
use crate::bot::fresh_policy;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCustomerBotStatusInput {}

pub struct GetCustomerBotStatus;

impl Operation for GetCustomerBotStatus {
    const META: OpMeta = OpMeta {
        address: "platform.customer_bot.get",
        tool: "get_customer_bot_status",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/customer-bot",
        summary: "Ailism Customer Bot now: its settings, the run working, the last run, when the next one starts, and how much its shoppers have done.",
        consumes: &[],
        produces: &[Slot::one("customer_bot_run_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCustomerBotStatusInput;
    type Output = Value;
}

#[derive(Debug, sqlx::FromRow)]
struct Totals {
    runs: i64,
    sessions: i64,
    failed_sessions: i64,
    accounts_created: i64,
    personas: i64,
    chat_turns: i64,
    orders: i64,
    reviews: i64,
    spent: Option<rust_decimal::Decimal>,
}

impl ReadOp<Cx> for GetCustomerBotStatus {
    async fn read(&self, cx: &Cx, _: GetCustomerBotStatusInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let policy = fresh_policy(&mut tx).await?;
        let settings = settings_of(&policy)?;
        let working: Option<RunRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{RUN_SELECT} WHERE r.tenant_id = $1 AND r.run_state IN ('queued', 'running')
             ORDER BY r.created_at DESC LIMIT 1"
        )))
        .bind(cx.tenant.as_str())
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let last: Option<RunRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{RUN_SELECT} WHERE r.tenant_id = $1 AND r.run_state IN ('done', 'failed')
             ORDER BY r.created_at DESC LIMIT 1"
        )))
        .bind(cx.tenant.as_str())
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        // The schedule counts real minutes from the last run that started.
        let next: (Option<DateTime<Utc>>, Option<f64>) = sqlx::query_as(
            "SELECT max(created_at) + make_interval(mins => $2),
                    extract(epoch FROM max(created_at) + make_interval(mins => $2) - now())::float8
             FROM platform.customer_bot_run WHERE tenant_id = $1",
        )
        .bind(cx.tenant.as_str())
        .bind(settings.every_minutes as i32)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let totals: Totals = sqlx::query_as(
            "SELECT (SELECT count(*) FROM platform.customer_bot_run WHERE tenant_id = $1) AS runs,
                    count(*) AS sessions,
                    count(*) FILTER (WHERE session_state = 'failed') AS failed_sessions,
                    count(*) FILTER (WHERE account_origin = 'created') AS accounts_created,
                    (SELECT count(*) FROM platform.shopper_persona WHERE tenant_id = $1) AS personas,
                    coalesce(sum((outcome->>'chat_turns')::bigint), 0)::bigint AS chat_turns,
                    coalesce(sum((outcome->>'orders')::bigint), 0)::bigint AS orders,
                    coalesce(sum((outcome->>'reviews')::bigint), 0)::bigint AS reviews,
                    sum((outcome->'spent'->>'value')::numeric) AS spent
             FROM platform.customer_bot_session WHERE tenant_id = $1",
        )
        .bind(cx.tenant.as_str())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let on = settings.mode == CustomerBotMode::On;
        let (next_at, next_in) = if on {
            match next {
                (Some(at), Some(secs)) => {
                    (Some(cx.time.format(at)), Some(secs.max(0.0).round() as i64))
                }
                _ => (None, Some(0)),
            }
        } else {
            (None, None)
        };
        let spent = acom_core::Money::new(totals.spent.unwrap_or_default(), cx.currency);
        Ok(json!({
            "settings": settings_json(&settings),
            "working": working.as_ref().map(|r| run_json(&cx.time, r)),
            "last_run": last.as_ref().map(|r| run_json(&cx.time, r)),
            "next_run_at": next_at, "next_run_in_seconds": next_in,
            "totals": {"runs": totals.runs, "sessions": totals.sessions,
                       "failed_sessions": totals.failed_sessions,
                       "accounts_created": totals.accounts_created, "personas": totals.personas,
                       "chat_turns": totals.chat_turns, "orders": totals.orders,
                       "reviews": totals.reviews, "spent": spent}
        }))
    }
}
