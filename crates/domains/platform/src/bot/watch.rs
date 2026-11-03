//! The scheduled cycle (`watcher.ailism_bot`, `policies/platform.yml watchers.ailism_bot`): opens
//! a cycle every `every_minutes` and announces it (`bot.cycle.updated`); the gateway hands it to
//! the agent, which reads the store, analyses, and reports back ([`super::report`]).

use acom_contract::{Job, JobMeta, JobOutcome, ToolError};
use acom_core::enums::BotTrigger;
use acom_policy::BotMode;
use acom_store::{Cx, TxCx};
use serde_json::{Value, json};

use super::{open_cycle, settings_of};

#[derive(Debug)]
pub struct AilismBotCycle;

impl Job<Cx> for AilismBotCycle {
    const META: JobMeta = JobMeta {
        kind: "watcher.ailism_bot",
        summary: "Opens an Ailism Bot cycle: the agent reads the store, analyses it, and proposes what to fix.",
        watcher: Some("ailism_bot"),
    };

    async fn run(&self, tx: &mut TxCx, _: &Value) -> Result<JobOutcome, ToolError> {
        let (bot, _) = settings_of(&tx.policy)?;
        let mode = match bot.mode {
            BotMode::Off => return Ok(JobOutcome::done(json!({"skipped": "off"}))),
            BotMode::Observe => "observe",
            BotMode::Propose => "propose",
        };
        let (id, created) = open_cycle(tx, BotTrigger::Schedule, None, mode).await?;
        Ok(JobOutcome::done(
            json!({"cycle_id": id, "created": created}),
        ))
    }
}
