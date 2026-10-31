//! `update_my_preferences`.

use super::MAX_ITEMS;
use crate::access::{policy_words, signed_in_shopper};
use crate::person::customer_not_found;
use crate::repo::{self, PreferenceRow};
use crate::wording::{about_someone_else, memory_texts, sensitive_reason};
use crate::{GIFT_RECIPIENT_DATA, NOT_SIGNED_IN, NOTHING_TO_CHANGE, SENSITIVE_DATA};
use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::PreferenceKey;
use acom_core::ids::MemoryId;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Where a memory was learned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    /// Learned in this conversation.
    Conversation,
}

impl MemorySource {
    fn word(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
        }
    }
}

/// One memory to add.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddMemory {
    /// What it is about: size; fit_note (fit and layering habits); fabric_preference;
    /// compares_specs (likes to compare specs); layout_preference (how they like results shown);
    /// travel_priority (what matters when traveling); other.
    #[schemars(example = &"travel_priority")]
    pub key: PreferenceKey,
    /// The value in the shopper's words, 1–200 characters.
    #[schemars(length(min = 1, max = 200), example = &"weight and packed size")]
    pub value: String,
    /// Where it was learned.
    #[schemars(example = &"conversation")]
    pub source: MemorySource,
}

/// How a memory to remove is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RemoveMatch {
    /// By `memory_id`.
    MemoryId,
    /// The memory with this `key` and `value`.
    KeyValue,
}

/// One memory to remove.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveMemory {
    /// memory_id: needs `memory_id`; key_value: needs `key` and `value`.
    #[serde(rename = "match")]
    #[schemars(example = &"memory_id")]
    pub match_by: RemoveMatch,
    /// The memory (`get_preferences`). With `match: memory_id`.
    #[schemars(regex(pattern = r"^mem_[a-z0-9_]+$"), example = &"mem_07")]
    pub memory_id: Option<String>,
    /// What the memory is about. With `match: key_value`.
    #[schemars(example = &"layout_preference")]
    pub key: Option<PreferenceKey>,
    /// The remembered value. With `match: key_value`.
    #[schemars(length(min = 1, max = 200), example = &"detailed lists")]
    pub value: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateMyPreferencesInput {
    /// Memories to add, at most 5.
    #[schemars(length(max = 5), example = add_example())]
    pub add: Option<Vec<AddMemory>>,
    /// Memories to forget, at most 5.
    #[schemars(length(max = 5), example = remove_example())]
    pub remove: Option<Vec<RemoveMemory>>,
}

fn add_example() -> Value {
    json!([{"key": "travel_priority", "value": "weight and packed size", "source": "conversation"}])
}

fn remove_example() -> Value {
    json!([{"match": "memory_id", "memory_id": "mem_07"}])
}

/// The memories to add (ids made now) and the ones to forget (kept whole, so undo can restore
/// them).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreferencePlan {
    pub customer_id: String,
    pub add: Vec<PlannedMemory>,
    pub remove: Vec<PlannedMemory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedMemory {
    pub id: String,
    pub key: String,
    pub value: String,
    /// The line kept on the profile.
    pub description: String,
    /// The note shown when it is remembered.
    pub text: String,
    pub source: String,
    pub learned_at: chrono::DateTime<chrono::Utc>,
}

impl PlannedMemory {
    fn row(&self, customer_id: &str) -> PreferenceRow {
        PreferenceRow {
            id: self.id.clone(),
            customer_id: customer_id.to_owned(),
            preference_key: self.key.clone(),
            value: self.value.clone(),
            description: self.description.clone(),
            source: self.source.clone(),
            conversation_id: None,
            learned_at: self.learned_at,
            version: 1,
        }
    }
}

pub struct UpdateMyPreferences;

impl Operation for UpdateMyPreferences {
    const META: OpMeta = OpMeta {
        address: "customer.preference.update_my_preferences",
        tool: "update_my_preferences",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/people/me/preferences",
        summary: "Remember or forget something the shopper said about themselves (size, fit, fabrics, how they like results shown, travel priorities). The shopper sees the memory and can undo it.",
        consumes: &[],
        produces: &[Slot::one("memory_id")],
        checks: &[
            "preference.about_shopper_only",
            "preference.no_sensitive_data",
        ],
        errors: &[
            GIFT_RECIPIENT_DATA,
            SENSITIVE_DATA,
            NOT_SIGNED_IN,
            NOTHING_TO_CHANGE,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateMyPreferencesInput;
    type Output = Value;
}

fn bad_input(message: &str, field: &str) -> ToolError {
    ToolError::invalid_input(message)
        .detail(json!({"field": field, "reason": "missing"}))
        .next("update_my_preferences")
}

impl WriteOp<Cx> for UpdateMyPreferences {
    type Plan = PreferencePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateMyPreferencesInput,
    ) -> Result<Planned<PreferencePlan>, ToolError> {
        let me = signed_in_shopper(cx)?;
        let add = input.add.unwrap_or_default();
        let remove = input.remove.unwrap_or_default();
        if add.is_empty() && remove.is_empty() {
            return Err(bad_input(
                "Give at least one memory to add or remove.",
                "add",
            ));
        }
        if add.len() > MAX_ITEMS || remove.len() > MAX_ITEMS {
            return Err(ToolError::invalid_input(format!(
                "At most {MAX_ITEMS} memories can be added or removed at once."
            ))
            .detail(json!({"field": "add", "reason": "range"})));
        }
        let keys = policy_words(cx, "customer.preference.keys");
        let sources = policy_words(cx, "customer.preference.sources");
        let now = cx.clock.now();

        // Screen what would be remembered before reading anything.
        let mut wanted = Vec::new();
        for a in &add {
            let (key, source) = (a.key.as_str(), a.source.word());
            let value = a.value.trim();
            if value.is_empty() || value.chars().count() > 200 {
                return Err(ToolError::invalid_input("A memory is 1–200 characters.")
                    .detail(json!({"field": "add[].value", "reason": "range"})));
            }
            if !keys.iter().any(|k| k == key) || !sources.iter().any(|s| s == source) {
                return Err(ToolError::invalid_input(format!(
                    "This store does not remember `{key}` from `{source}`."
                ))
                .detail(json!({"field": "add[].key", "reason": "enum", "allowed": keys})));
            }
            if let Some(why) = sensitive_reason(value) {
                return Err(ToolError::new(
                    SENSITIVE_DATA,
                    format!("Not remembered: {why} Use it in this conversation only."),
                )
                .detail(json!({"rule": "preference.no_sensitive_data"})));
            }
            if about_someone_else(value) {
                return Err(ToolError::new(
                    GIFT_RECIPIENT_DATA,
                    "Not remembered: it is about someone else. Use it in this conversation only.",
                )
                .detail(json!({"rule": "preference.about_shopper_only"})));
            }
            let (description, text) = memory_texts(key, value);
            wanted.push(PlannedMemory {
                id: MemoryId::generate().to_string(),
                key: key.to_owned(),
                value: value.to_owned(),
                description,
                text,
                source: source.to_owned(),
                learned_at: now,
            });
        }

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let who = repo::person(&mut tx, &me)
            .await?
            .ok_or_else(|| customer_not_found(&me))?;
        let existing = repo::all_preferences(&mut tx, &me).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let mut gone: Vec<&PreferenceRow> = Vec::new();
        for r in &remove {
            let found = match r.match_by {
                RemoveMatch::MemoryId => {
                    let id = r.memory_id.as_deref().ok_or_else(|| {
                        bad_input(
                            "`match: memory_id` needs `memory_id`.",
                            "remove[].memory_id",
                        )
                    })?;
                    existing.iter().find(|p| p.id == id)
                }
                RemoveMatch::KeyValue => {
                    let (Some(k), Some(v)) = (r.key, r.value.as_deref()) else {
                        return Err(bad_input(
                            "`match: key_value` needs `key` and `value`.",
                            "remove[].key",
                        ));
                    };
                    existing
                        .iter()
                        .find(|p| p.preference_key == k.as_str() && p.value == v.trim())
                }
            };
            if let Some(p) = found
                && !gone.iter().any(|g| g.id == p.id)
            {
                gone.push(p);
            }
        }
        // The same key and value stay one memory.
        let mut unique: Vec<PlannedMemory> = Vec::new();
        for w in wanted {
            let known = existing
                .iter()
                .any(|p| p.preference_key == w.key && p.value == w.value)
                || unique.iter().any(|u| u.key == w.key && u.value == w.value);
            if !known {
                unique.push(w);
            }
        }
        let wanted = unique;
        if wanted.is_empty() && gone.is_empty() {
            return Err(ToolError::new(
                NOTHING_TO_CHANGE,
                "Already remembered, or nothing to forget.",
            )
            .next("get_preferences"));
        }

        let mut changes: Vec<Change> = wanted
            .iter()
            .map(|w| Change {
                op: ChangeOp::Create,
                entity: "preference".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"key": w.key, "value": w.value, "source": w.source}),
                note: Some(w.text.clone()),
            })
            .collect();
        changes.extend(gone.iter().map(|p| Change {
            op: ChangeOp::Delete,
            entity: "preference".into(),
            id: p.id.clone(),
            from: json!({"key": p.preference_key, "value": p.value}),
            to: Value::Null,
            note: None,
        }));
        let checks = vec![
            Check {
                rule: "preference.about_shopper_only".into(),
                passed: true,
                detail: Some("About the shopper, not a gift recipient".into()),
                fix_tool: None,
            },
            Check {
                rule: "preference.no_sensitive_data".into(),
                passed: true,
                detail: Some("No payment or health data".into()),
                fix_tool: None,
            },
        ];
        let mut basis = vec![Basis {
            entity: "customer.person".into(),
            id: who.id.clone(),
            version: i64::from(who.version),
        }];
        basis.extend(gone.iter().map(|p| Basis {
            entity: "customer.preference".into(),
            id: p.id.clone(),
            version: i64::from(p.version),
        }));
        let remove: Vec<PlannedMemory> = gone
            .iter()
            .map(|p| PlannedMemory {
                id: p.id.clone(),
                key: p.preference_key.clone(),
                value: p.value.clone(),
                description: p.description.clone(),
                text: p.description.clone(),
                source: p.source.clone(),
                learned_at: p.learned_at,
            })
            .collect();
        let facts = json!({"customer_id": me, "add": wanted.len(), "remove": remove.len()});
        Ok(Planned {
            plan: PreferencePlan {
                customer_id: me,
                add: wanted,
                remove,
            },
            changes,
            checks,
            basis,
            approval: crate::access::approval(cx, Self::META.tool, &facts),
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &PreferencePlan) -> Result<Committed, ToolError> {
        let mut memories = Vec::new();
        for m in &plan.add {
            // A memory that already existed keeps its first id.
            let id = repo::insert_preference(tx, &m.row(&plan.customer_id)).await?;
            memories.push(json!({"memory_id": id, "text": m.text}));
        }
        for m in &plan.remove {
            repo::delete_preference(tx, &plan.customer_id, &m.id).await?;
        }
        let mut produced = Map::new();
        let id = match memories.first() {
            Some(first) => {
                produced.insert("memory_id".into(), first["memory_id"].clone());
                produced.insert("memory".into(), first.clone());
                produced.insert("memories".into(), json!(memories));
                first["memory_id"].as_str().unwrap_or_default().to_owned()
            }
            None => plan
                .remove
                .first()
                .map(|m| m.id.clone())
                .unwrap_or_default(),
        };
        Ok(Committed { id, produced })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &PreferencePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let now = repo::all_preferences(&mut tx, &plan.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        for m in &plan.add {
            if !now
                .iter()
                .any(|p| p.preference_key == m.key && p.value == m.value)
            {
                mismatches.push(format!("{} \"{}\" was not remembered", m.key, m.value));
            }
        }
        for m in &plan.remove {
            if now.iter().any(|p| p.id == m.id) {
                mismatches.push(format!("{} is still remembered", m.id));
            }
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &PreferencePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        for m in &plan.add {
            repo::delete_preference(tx, &plan.customer_id, &m.id).await?;
        }
        for m in &plan.remove {
            repo::insert_preference(tx, &m.row(&plan.customer_id)).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_flat() {
        let s = closed_schema_for::<UpdateMyPreferencesInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        let remove = &s["properties"]["remove"]["items"]["properties"];
        assert_eq!(remove["match"]["enum"], json!(["memory_id", "key_value"]));
        assert_eq!(
            s["properties"]["add"]["items"]["properties"]["source"]["enum"],
            json!(["conversation"])
        );
        assert_eq!(
            UpdateMyPreferences::META.path,
            "/v1/customer/people/me/preferences"
        );
    }
}
