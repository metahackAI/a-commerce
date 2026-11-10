//! Person-only `PUT /v1/platform/customer-bot/settings` (`platform.customer_bot.update_settings`):
//! how Ailism Customer Bot runs. The values live in the store's `platform` policy override
//! (`customer_bot`), so every reader of the policy sees the same settings.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, ToolError,
};
use acom_core::enums::CustomerBotMode;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{db, settings_json, settings_of};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCustomerBotSettingsInput {
    /// on: a run every `every_minutes`; off: only when a person runs it now.
    #[schemars(example = &"on")]
    pub mode: Option<CustomerBotMode>,
    /// Real minutes between scheduled runs, 5–1440.
    #[schemars(range(min = 5, max = 1440), example = 10)]
    pub every_minutes: Option<u32>,
    /// Sessions that shop at the same time in one run, 1–10.
    #[schemars(range(min = 1, max = 10), example = 2)]
    pub workers: Option<u32>,
    /// Share of sessions, in percent, that sign up a new account first, 0–100.
    #[schemars(range(min = 0, max = 100), example = 20)]
    pub new_account_percent: Option<u32>,
    /// The most actions one session takes (chat turns, orders, reviews), 1–20.
    #[schemars(range(min = 1, max = 20), example = 8)]
    pub max_actions: Option<u32>,
    /// The most reviews one session writes for delivered items, 0–5.
    #[schemars(range(min = 0, max = 5), example = 2)]
    pub reviews_per_session: Option<u32>,
    /// The model's alias (agents/models.yml).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"glimmer")]
    pub model: Option<String>,
}

pub struct UpdateCustomerBotSettings;

impl Operation for UpdateCustomerBotSettings {
    const META: OpMeta = OpMeta {
        address: "platform.customer_bot.update_settings",
        tool: "update_customer_bot_settings",
        group: Group::Operate,
        kind: Kind::Person,
        method: Method::Put,
        path: "/v1/platform/customer-bot/settings",
        summary: "Change how Ailism Customer Bot runs: on or off, how often, how many shoppers at once, how many new accounts, how much each does, and with which model.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[ErrorCode::INVALID_INPUT],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "human:bot",
    };
    type Input = UpdateCustomerBotSettingsInput;
    type Output = Value;
}

fn invalid(field: &str, message: &str) -> ToolError {
    ToolError::new(ErrorCode::INVALID_INPUT, message.to_owned())
        .detail(json!({"field": field, "reason": "out_of_range"}))
}

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

impl UpdateCustomerBotSettingsInput {
    fn patch(self) -> Result<(Value, Vec<String>), ToolError> {
        let mut bot = Map::new();
        let mut keys = Vec::new();
        if let Some(m) = self.mode {
            bot.insert("mode".into(), json!(m.as_str()));
            keys.push("mode".to_owned());
        }
        for (field, value, range) in [
            ("every_minutes", self.every_minutes, 5..=1440),
            ("workers", self.workers, 1..=10),
            ("new_account_percent", self.new_account_percent, 0..=100),
            ("max_actions", self.max_actions, 1..=20),
            ("reviews_per_session", self.reviews_per_session, 0..=5),
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
        Ok((json!({"customer_bot": bot}), keys))
    }
}

impl PersonOp<Cx> for UpdateCustomerBotSettings {
    async fn run(
        &self,
        tx: &mut TxCx,
        input: UpdateCustomerBotSettingsInput,
    ) -> Result<Value, ToolError> {
        let (patch, keys) = input.patch()?;
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
        let settings = settings_of(&policy)?;
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
        let out = settings_json(&settings);
        acom_changeset::log_action(
            tx,
            acom_changeset::LogEntry {
                object: Some(("customer_bot", "settings")),
                result: Some(json!({"changed": keys, "settings": out})),
                ..acom_changeset::LogEntry::new(
                    ActionType::UPDATE.0,
                    acom_core::enums::ActionStatus::Completed,
                    "update_customer_bot_settings",
                )
            },
        )
        .await?;
        Ok(json!({"ok": true, "settings": out, "changed": keys}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(v: Value) -> UpdateCustomerBotSettingsInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn only_the_settings_given_change_and_out_of_range_ones_are_refused() {
        let (patch, keys) = input(json!({"workers": 4, "mode": "off"})).patch().unwrap();
        assert_eq!(
            patch,
            json!({"customer_bot": {"mode": "off", "workers": 4}})
        );
        assert_eq!(keys, vec!["mode", "workers"]);
        let e = input(json!({"workers": 30})).patch().unwrap_err();
        assert_eq!(e.detail.as_ref().unwrap()["field"], "workers");
        assert!(input(json!({"model": "Not An Alias"})).patch().is_err());
    }
}
