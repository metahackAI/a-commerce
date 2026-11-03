//! Person-only `PUT /v1/platform/bot/settings` (`platform.bot.update_settings`): how Ailism Bot
//! runs. The values live in the store's `platform` policy override (`bot` and
//! `watchers.ailism_bot`), so every reader of the policy sees the same settings.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, ToolError,
};
use acom_core::enums::BotMode;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{db, settings_json, settings_of};

/// The areas to turn on or off; the ones left out stay as they are.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BotAreasInput {
    /// Prices under cost or the floor, discounts too deep.
    pub pricing: Option<bool>,
    /// Stock running out, reorders.
    pub inventory: Option<bool>,
    /// Running and ending promotions, slow sellers.
    pub promotions: Option<bool>,
    /// Main page slots, banners and collections.
    pub merchandising: Option<bool>,
    /// Products missing a description or photos.
    pub catalog_content: Option<bool>,
    /// Return requests waiting.
    pub returns: Option<bool>,
    /// Unanswered product questions and low reviews.
    pub questions_reviews: Option<bool>,
    /// Reported community posts.
    pub community: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateBotSettingsInput {
    /// off: no cycle runs; observe: reports only; propose: tasks and change set drafts.
    #[schemars(example = &"propose")]
    pub mode: Option<BotMode>,
    /// Minutes between scheduled cycles, 30–1440.
    #[schemars(range(min = 30, max = 1440), example = 240)]
    pub every_minutes: Option<u32>,
    /// The active operator the drafts are made for; an empty text clears it (the task
    /// pickup's operator is used).
    #[schemars(example = &"adm_sj")]
    pub acts_as: Option<String>,
    /// The analysing model's alias (agents/models.yml).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"glimmer")]
    pub model: Option<String>,
    /// Areas to turn on or off; the ones left out stay as they are.
    pub areas: Option<BotAreasInput>,
    /// Proposals opened per cycle, 1–10.
    #[schemars(range(min = 1, max = 10), example = 3)]
    pub max_proposals_per_cycle: Option<u32>,
    /// No new proposal while this many wait for a decision, 1–50.
    #[schemars(range(min = 1, max = 50), example = 8)]
    pub max_open_decisions: Option<u32>,
    /// Days a turned-down proposal's cause is not proposed again, 0–60.
    #[schemars(range(min = 0, max = 60), example = 7)]
    pub cooldown_days: Option<u32>,
    /// Standing directives, up to 20 of up to 300 characters each.
    #[schemars(example = ["Never discount new arrivals."])]
    pub directives: Option<Vec<String>>,
}

/// Person-only: an operator changes how the bot runs.
pub struct UpdateBotSettings;

impl Operation for UpdateBotSettings {
    const META: OpMeta = OpMeta {
        address: "platform.bot.update_settings",
        tool: "update_bot_settings",
        group: Group::Operate,
        kind: Kind::Person,
        method: Method::Put,
        path: "/v1/platform/bot/settings",
        summary: "Change how Ailism Bot runs: its mode, how often, for which operator, with which model, in which areas, and how many proposals it may open.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[ErrorCode::INVALID_INPUT],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "human:bot",
    };
    type Input = UpdateBotSettingsInput;
    type Output = Value;
}

fn invalid(field: &str, message: &str) -> ToolError {
    ToolError::new(ErrorCode::INVALID_INPUT, message.to_owned())
        .detail(json!({"field": field, "reason": "out_of_range"}))
}

/// `patch` written over `base`, objects key by key.
fn merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                merge(b.entry(k).or_insert(Value::Null), v);
            }
        }
        (b, p) => *b = p,
    }
}

impl UpdateBotSettingsInput {
    /// The checked values as a policy patch, and the keys they change.
    fn patch(self) -> Result<(Value, Vec<String>), ToolError> {
        let mut bot = Map::new();
        let mut watch = Map::new();
        let mut keys = Vec::new();
        if let Some(m) = self.mode {
            bot.insert("mode".into(), json!(m.as_str()));
            watch.insert("enabled".into(), json!(m != BotMode::Off));
            keys.push("mode".to_owned());
        }
        if let Some(n) = self.every_minutes {
            if !(30..=1440).contains(&n) {
                return Err(invalid("every_minutes", "Run it every 30 to 1440 minutes."));
            }
            watch.insert("every_minutes".into(), json!(n));
            keys.push("every_minutes".to_owned());
        }
        if let Some(a) = self.acts_as {
            let a = a.trim();
            bot.insert(
                "acts_as".into(),
                if a.is_empty() { Value::Null } else { json!(a) },
            );
            keys.push("acts_as".to_owned());
        }
        if let Some(m) = self.model {
            let ok = m.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && m.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !ok {
                return Err(invalid(
                    "model",
                    "A model is named by its alias, like glimmer.",
                ));
            }
            bot.insert("model".into(), json!(m));
            keys.push("model".to_owned());
        }
        if let Some(a) = self.areas {
            let mut areas = Map::new();
            for (k, v) in [
                ("pricing", a.pricing),
                ("inventory", a.inventory),
                ("promotions", a.promotions),
                ("merchandising", a.merchandising),
                ("catalog_content", a.catalog_content),
                ("returns", a.returns),
                ("questions_reviews", a.questions_reviews),
                ("community", a.community),
            ] {
                if let Some(on) = v {
                    areas.insert(k.into(), json!(on));
                    keys.push(format!("areas.{k}"));
                }
            }
            bot.insert("areas".into(), Value::Object(areas));
        }
        for (field, value, range) in [
            (
                "max_proposals_per_cycle",
                self.max_proposals_per_cycle,
                1..=10,
            ),
            ("max_open_decisions", self.max_open_decisions, 1..=50),
            ("cooldown_days", self.cooldown_days, 0..=60),
        ] {
            if let Some(n) = value {
                if !range.contains(&n) {
                    return Err(invalid(
                        field,
                        &format!(
                            "{} must be {} to {}.",
                            field.replace('_', " "),
                            range.start(),
                            range.end()
                        ),
                    ));
                }
                bot.insert(field.into(), json!(n));
                keys.push(field.to_owned());
            }
        }
        if let Some(d) = self.directives {
            let d: Vec<String> = d
                .into_iter()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            if d.len() > 20 || d.iter().any(|s| s.chars().count() > 300) {
                return Err(invalid(
                    "directives",
                    "Up to 20 directives of up to 300 characters each.",
                ));
            }
            bot.insert("directives".into(), json!(d));
            keys.push("directives".to_owned());
        }
        Ok((json!({"bot": bot, "watchers": {"ailism_bot": watch}}), keys))
    }
}

impl PersonOp<Cx> for UpdateBotSettings {
    async fn run(&self, tx: &mut TxCx, input: UpdateBotSettingsInput) -> Result<Value, ToolError> {
        let acts_as = input.acts_as.clone().filter(|a| !a.trim().is_empty());
        let (patch, keys) = input.patch()?;
        if let Some(a) = &acts_as {
            let active: Option<String> = sqlx::query_scalar(
                "SELECT id FROM platform.admin_user
                 WHERE tenant_id = $1 AND id = $2 AND admin_status = 'active'",
            )
            .bind(tx.tenant.as_str())
            .bind(a.trim())
            .fetch_optional(&mut *tx.tx)
            .await
            .map_err(db)?;
            if active.is_none() {
                return Err(ToolError::new(
                    ErrorCode::INVALID_INPUT,
                    format!("{a} is not an active operator."),
                )
                .detail(json!({"field": "acts_as", "reason": "unknown_operator"})));
            }
        }
        let row: Option<(Value, i32)> = sqlx::query_as(
            "SELECT partial, policy_version FROM platform.policy_override
             WHERE tenant_id = $1 AND file = 'platform' FOR UPDATE",
        )
        .bind(tx.tenant.as_str())
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let (mut partial, policy_version) = row.unwrap_or((json!({}), 1));
        merge(&mut partial, patch);
        // The whole store policy must still read with it.
        let policy = tx
            .policy
            .with_overrides([("platform", partial.to_string().as_str())])
            .map_err(|e| ToolError::new(ErrorCode::INVALID_INPUT, e.to_string()))?;
        let (bot, watch) = settings_of(&policy)?;
        sqlx::query(
            "INSERT INTO platform.policy_override (tenant_id, file, partial, policy_version)
             VALUES ($1, 'platform', $2, $3)
             ON CONFLICT (tenant_id, file) DO UPDATE
               SET partial = EXCLUDED.partial, updated_at = now(),
                   version = platform.policy_override.version + 1",
        )
        .bind(tx.tenant.as_str())
        .bind(&partial)
        .bind(policy_version)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        let settings = settings_json(&bot, &watch);
        acom_changeset::log_action(
            tx,
            acom_changeset::LogEntry {
                object: Some(("policy_override", "platform")),
                result: Some(json!({"changed": keys, "settings": settings})),
                ..acom_changeset::LogEntry::new(
                    ActionType::UPDATE.0,
                    acom_core::enums::ActionStatus::Completed,
                    "update_bot_settings",
                )
            },
        )
        .await?;
        Ok(json!({"ok": true, "settings": settings}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_the_bot_off_also_stops_its_schedule_and_ranges_are_checked() {
        let input: UpdateBotSettingsInput =
            serde_json::from_value(json!({"mode": "off", "areas": {"community": false}})).unwrap();
        let (patch, keys) = input.patch().unwrap();
        assert_eq!(patch["bot"]["mode"], "off");
        assert_eq!(patch["watchers"]["ailism_bot"]["enabled"], false);
        assert_eq!(patch["bot"]["areas"], json!({"community": false}));
        assert_eq!(keys, ["mode", "areas.community"]);
        let too_often: UpdateBotSettingsInput =
            serde_json::from_value(json!({"every_minutes": 5})).unwrap();
        assert!(too_often.patch().is_err());
        let mut base = json!({"watchers": {"price_below_cost": {"run_fix": "standing_approval"}}});
        merge(
            &mut base,
            json!({"watchers": {"ailism_bot": {"every_minutes": 120}}}),
        );
        assert_eq!(
            base["watchers"]["price_below_cost"]["run_fix"],
            "standing_approval"
        );
        assert_eq!(base["watchers"]["ailism_bot"]["every_minutes"], 120);
    }
}
