//! Person-only `POST /v1/platform/customer-bot/runs` (`platform.customer_bot.run`): runs Ailism
//! Customer Bot now.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot, ToolError,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{db, run_event, settings_of};
use crate::bot::fresh_policy;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunCustomerBotInput {}

/// Person-only: an operator runs the shoppers now ("Run now").
pub struct RunCustomerBot;

impl Operation for RunCustomerBot {
    const META: OpMeta = OpMeta {
        address: "platform.customer_bot.run",
        tool: "run_customer_bot",
        group: Group::Operate,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/platform/customer-bot/runs",
        summary: "Run Ailism Customer Bot now: queue a run whose shoppers start as soon as the gateway takes it. A run already queued or working is returned instead of a second.",
        consumes: &[],
        produces: &[Slot::one("customer_bot_run_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "human:bot",
    };
    type Input = RunCustomerBotInput;
    type Output = Value;
}

impl PersonOp<Cx> for RunCustomerBot {
    async fn run(&self, tx: &mut TxCx, _: RunCustomerBotInput) -> Result<Value, ToolError> {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtext('customer_bot_run:' || $1))")
            .bind(tx.tenant.as_str())
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        let open: Option<(String, String)> = sqlx::query_as(
            "SELECT id, run_state FROM platform.customer_bot_run
             WHERE tenant_id = $1 AND run_state IN ('queued', 'running')
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(tx.tenant.as_str())
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        if let Some((id, state)) = open {
            return Ok(json!({"ok": true, "id": id, "state": state, "created": false}));
        }
        let policy = fresh_policy(tx).await?;
        let settings = settings_of(&policy)?;
        let id = format!("cbr_{}", acom_core::id::new_suffix());
        let by = tx.principal.subject.clone();
        sqlx::query(
            "INSERT INTO platform.customer_bot_run (tenant_id, id, bot_trigger, requested_by,
               run_state, workers, start_time)
             VALUES ($1, $2, 'manual', $3, 'queued', $4, $5)",
        )
        .bind(tx.tenant.as_str())
        .bind(&id)
        .bind(&by)
        .bind(settings.workers.clamp(1, 20) as i32)
        .bind(tx.clock.now())
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        run_event(tx, &id, "queued").await?;
        acom_changeset::log_action(
            tx,
            acom_changeset::LogEntry {
                object: Some(("customer_bot_run", &id)),
                result: Some(json!({"state": "queued", "trigger": "manual"})),
                ..acom_changeset::LogEntry::new(
                    ActionType::CREATE.0,
                    acom_core::enums::ActionStatus::Completed,
                    "run_customer_bot",
                )
            },
        )
        .await?;
        Ok(json!({"ok": true, "id": id, "state": "queued", "created": true}))
    }
}
