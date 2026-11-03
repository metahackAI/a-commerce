//! Person-only `POST /v1/platform/bot/cycles` (`platform.bot.run`): runs Ailism Bot now.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot,
    ToolError,
};
use acom_core::enums::BotTrigger;
use acom_policy::BotMode;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{fresh_policy, open_cycle, settings_of};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunBotCycleInput {}

/// Person-only: an operator runs a cycle now ("Run now").
pub struct RunBotCycle;

impl Operation for RunBotCycle {
    const META: OpMeta = OpMeta {
        address: "platform.bot.run",
        tool: "run_bot_cycle",
        group: Group::Operate,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/platform/bot/cycles",
        summary: "Run Ailism Bot now: open a cycle that reads the store and proposes what to fix. The one already running is returned instead of a second.",
        consumes: &[],
        produces: &[Slot::one("bot_cycle_id")],
        checks: &[],
        errors: &[ErrorCode::INVALID_STATE],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "human:bot",
    };
    type Input = RunBotCycleInput;
    type Output = Value;
}

impl PersonOp<Cx> for RunBotCycle {
    async fn run(&self, tx: &mut TxCx, _: RunBotCycleInput) -> Result<Value, ToolError> {
        let policy = fresh_policy(tx).await?;
        let (bot, _) = settings_of(&policy)?;
        if bot.mode == BotMode::Off {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                "Ailism Bot is off. Turn it on in its settings first.",
            )
            .detail(json!({"entity": "bot", "from": "off"})));
        }
        let by = tx.principal.subject.clone();
        let mode = serde_json::to_value(bot.mode)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "propose".to_owned());
        let (id, created) = open_cycle(tx, BotTrigger::Manual, Some(&by), &mode).await?;
        if created {
            acom_changeset::log_action(
                tx,
                acom_changeset::LogEntry {
                    object: Some(("bot_cycle", &id)),
                    result: Some(json!({"state": "observing", "trigger": "manual"})),
                    ..acom_changeset::LogEntry::new(
                        ActionType::CREATE.0,
                        acom_core::enums::ActionStatus::Completed,
                        "run_bot_cycle",
                    )
                },
            )
            .await?;
        }
        Ok(json!({"ok": true, "id": id, "state": "observing"}))
    }
}
