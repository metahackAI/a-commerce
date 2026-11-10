//! `get_customer_bot_run` (`platform.customer_bot.get_run`): one run with its sessions.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    CUSTOMER_BOT_RUN_NOT_FOUND, RUN_SELECT, RunRow, SESSION_SELECT, SessionRow, db, not_found,
    run_json, session_json,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCustomerBotRunInput {
    /// The run.
    #[schemars(regex(pattern = r"^cbr_[a-z0-9_]+$"), example = &"cbr_01jcb0")]
    pub customer_bot_run_id: String,
}

pub struct GetCustomerBotRun;

impl Operation for GetCustomerBotRun {
    const META: OpMeta = OpMeta {
        address: "platform.customer_bot.get_run",
        tool: "get_customer_bot_run",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/customer-bot/runs/{customer_bot_run_id}",
        summary: "One Ailism Customer Bot run with its sessions: who shopped, as which persona, and what each did.",
        consumes: &[Slot::one("customer_bot_run_id")],
        produces: &[Slot::many("customer_bot_session_id")],
        checks: &[],
        errors: &[CUSTOMER_BOT_RUN_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCustomerBotRunInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCustomerBotRun {
    async fn read(&self, cx: &Cx, input: GetCustomerBotRunInput) -> Result<Value, ToolError> {
        let id = input.customer_bot_run_id;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let run: Option<RunRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{RUN_SELECT} WHERE r.tenant_id = $1 AND r.id = $2"
        )))
        .bind(cx.tenant.as_str())
        .bind(&id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let Some(run) = run else {
            return Err(not_found(
                CUSTOMER_BOT_RUN_NOT_FOUND,
                "customer_bot_run",
                &id,
                "list_customer_bot_runs",
            ));
        };
        let sessions: Vec<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{SESSION_SELECT} WHERE s.tenant_id = $1 AND s.run_id = $2 ORDER BY s.created_at, s.id"
        )))
        .bind(cx.tenant.as_str())
        .bind(&id)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut out = run_json(&cx.time, &run);
        out["sessions"] = json!(
            sessions
                .iter()
                .map(|s| session_json(&cx.time, s))
                .collect::<Vec<_>>()
        );
        Ok(out)
    }
}
