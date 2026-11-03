//! `get_bot_status` (`platform.bot.get`): Ailism Bot now — its settings, next and last cycle,
//! how many decisions are in each state, and how much feedback is open.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{db, decisions_sql, fresh_policy, settings_json, settings_of};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetBotStatusInput {}

pub struct GetBotStatus;

impl Operation for GetBotStatus {
    const META: OpMeta = OpMeta {
        address: "platform.bot.get",
        tool: "get_bot_status",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/bot",
        summary: "Ailism Bot now: its settings, when it runs next, its last cycle, how many of its decisions wait or ran, and the feedback still open.",
        consumes: &[],
        produces: &[Slot::one("bot_cycle_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetBotStatusInput;
    type Output = Value;
}

/// The operator the bot's drafts are made for: its own setting, else the task pickup's.
pub(crate) fn acts_as(policy: &acom_policy::PolicySet, own: Option<&str>) -> Option<String> {
    own.map(str::to_owned).or_else(|| {
        policy
            .raw("platform")
            .and_then(|p| p.pointer("/task_pickup/acts_as"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    })
}

/// The last cycle: id, state, trigger, start, report, error.
type LastCycle = (
    String,
    String,
    String,
    DateTime<Utc>,
    Option<Value>,
    Option<String>,
);

impl ReadOp<Cx> for GetBotStatus {
    async fn read(&self, cx: &Cx, _: GetBotStatusInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let policy = fresh_policy(&mut tx).await?;
        let (bot, watch) = settings_of(&policy)?;
        let next: Option<DateTime<Utc>> = sqlx::query_scalar(
            "SELECT min(run_at) FROM platform.job
             WHERE tenant_id = $1 AND kind = 'watcher.ailism_bot' AND done_at IS NULL",
        )
        .bind(cx.tenant.as_str())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let last: Option<LastCycle> = sqlx::query_as(
            "SELECT id, cycle_state, bot_trigger, start_time, report, error
                 FROM platform.bot_cycle WHERE tenant_id = $1
                 ORDER BY start_time DESC, id DESC LIMIT 1",
        )
        .bind(cx.tenant.as_str())
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let counts: Vec<(String, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT state, count(*) FROM {} WHERE d.tenant_id = $1 GROUP BY state",
            decisions_sql()
        )))
        .bind(cx.tenant.as_str())
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        let open_feedback: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.bot_feedback
             WHERE tenant_id = $1 AND feedback_state = 'open'",
        )
        .bind(cx.tenant.as_str())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut decisions: Map<String, Value> = acom_core::enums::BotDecisionState::WORDS
            .iter()
            .map(|w| ((*w).to_owned(), json!(0)))
            .collect();
        for (state, n) in counts {
            decisions.insert(state, json!(n));
        }
        let running = watch.enabled && bot.mode != acom_policy::BotMode::Off;
        Ok(json!({
            "settings": settings_json(&bot, &watch),
            "acts_as": acts_as(&policy, bot.acts_as.as_deref()),
            "next_run_at": next.filter(|_| running).map(|t| cx.time.format(t)),
            "last_cycle": last.map(|(id, state, trigger, at, report, error)| json!({
                "id": id, "state": state, "trigger": trigger, "at": cx.time.format(at),
                "summary": report.as_ref().and_then(|r| r.get("summary")).cloned(),
                "error": error})),
            "decisions": decisions,
            "open_feedback": open_feedback,
        }))
    }
}
