//! `get_preferences`, `update_my_preferences`, and the person-only
//! `POST /v1/customer/me/memories/{memory_id}/undo`: what the agent remembers about a shopper,
//! always visible to them and undoable with one tap. Only facts about the shopper themselves
//! are kept; never a gift recipient's, never payment or health data.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, PersonOp, Planned, ReadOp, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_core::enums::{ActionStatus, PreferenceKey};
use acom_core::ids::MemoryId;
use acom_store::{Cx, Page, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::access::{customer_for, policy_words, signed_in_shopper};
use crate::person::customer_not_found;
use crate::repo::{self, PreferenceRow};
use crate::wording::{about_someone_else, memory_texts, sensitive_reason};
use crate::{
    CUSTOMER_NOT_FOUND, GIFT_RECIPIENT_DATA, MEMORY_NOT_FOUND, NOT_SIGNED_IN, NOTHING_TO_CHANGE,
    SENSITIVE_DATA,
};

/// How many memories one call may add or remove.
const MAX_ITEMS: usize = 5;

// ── get_preferences ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPreferencesInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
    /// Only memories about this.
    #[schemars(example = &"travel_priority")]
    pub key: Option<PreferenceKey>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct GetPreferences;

impl Operation for GetPreferences {
    const META: OpMeta = OpMeta {
        address: "customer.preference.get",
        tool: "get_preferences",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/{customer_id}/preferences",
        summary: "Everything the agent remembers about a member, newest first, each as the line the shopper sees.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::many("memory_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPreferencesInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetPreferences {
    async fn read(&self, cx: &Cx, input: GetPreferencesInput) -> Result<Value, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if repo::person(&mut tx, &customer_id).await?.is_none() {
            return Err(customer_not_found(&customer_id));
        }
        let key = input.key.map(|k| k.as_str());
        let (total, rows) =
            repo::preferences(&mut tx, &customer_id, key, page.limit, page.offset).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|p| {
                json!({"id": p.id, "key": p.preference_key, "value": p.value,
                       "text": p.description, "source": p.source,
                       "created_at": cx.time.format(p.learned_at)})
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

// ── update_my_preferences ───────────────────────────────────────────────────────────────

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

// ── POST /v1/customer/me/memories/{memory_id}/undo ──────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UndoMemoryInput {
    /// The memory to forget (the `memory_note` block's Undo button).
    #[schemars(regex(pattern = r"^mem_[a-z0-9_]+$"), example = &"mem_12")]
    pub memory_id: String,
}

/// Person-only: the shopper forgets one remembered preference.
pub struct UndoMemory;

impl Operation for UndoMemory {
    const META: OpMeta = OpMeta {
        address: "customer.preference.undo_memory",
        tool: "undo_memory",
        group: Group::Engage,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/customer/me/memories/{memory_id}/undo",
        summary: "Forget one remembered preference. The shopper's Undo button on a memory note.",
        consumes: &[Slot::one("memory_id")],
        produces: &[],
        checks: &[],
        errors: &[MEMORY_NOT_FOUND, NOT_SIGNED_IN],
        audience: Audience::Shopper,
        action_type: ActionType::DELETE,
        person_scope: "human:undo",
    };
    type Input = UndoMemoryInput;
    type Output = Value;
}

impl PersonOp<Cx> for UndoMemory {
    async fn run(&self, tx: &mut TxCx, input: UndoMemoryInput) -> Result<Value, ToolError> {
        let me = tx
            .principal
            .customer_id()
            .map(str::to_owned)
            .ok_or_else(crate::access::not_signed_in)?;
        let memory = repo::preference(tx, &input.memory_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    MEMORY_NOT_FOUND,
                    format!(
                        "There is no memory {}; it may already be forgotten.",
                        input.memory_id
                    ),
                )
                .detail(json!({"entity": "preference", "id": input.memory_id}))
            })?;
        if memory.customer_id != me {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                "That memory belongs to someone else.",
            )
            .detail(json!({"reason": "owner"})));
        }
        repo::delete_preference(tx, &me, &memory.id).await?;
        acom_changeset::log_action(
            tx,
            acom_changeset::LogEntry {
                object: Some(("preference", &memory.id)),
                result: Some(json!({"forgotten": {"key": memory.preference_key,
                                                  "value": memory.value}})),
                ..acom_changeset::LogEntry::new(
                    ActionType::DELETE.0,
                    ActionStatus::Completed,
                    "undo_memory",
                )
            },
        )
        .await?;
        Ok(json!({"ok": true, "id": memory.id, "undo": null}))
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn inputs_are_closed_and_flat() {
        for s in [
            closed_schema_for::<GetPreferencesInput>(),
            closed_schema_for::<UpdateMyPreferencesInput>(),
            closed_schema_for::<UndoMemoryInput>(),
        ] {
            assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        }
        let s = closed_schema_for::<UpdateMyPreferencesInput>();
        let remove = &s["properties"]["remove"]["items"]["properties"];
        assert_eq!(remove["match"]["enum"], json!(["memory_id", "key_value"]));
        assert_eq!(
            s["properties"]["add"]["items"]["properties"]["source"]["enum"],
            json!(["conversation"])
        );
    }

    #[test]
    fn undo_is_person_only() {
        assert_eq!(UndoMemory::META.kind, Kind::Person);
        assert_eq!(UndoMemory::META.person_scope, "human:undo");
        assert_eq!(
            UpdateMyPreferences::META.path,
            "/v1/customer/people/me/preferences"
        );
    }

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn remembers_and_forgets() {
        let cx = crate::testdb::cx("cus_jamie").await;
        let planned = UpdateMyPreferences
            .preview(
                &cx,
                UpdateMyPreferencesInput {
                    add: Some(vec![AddMemory {
                        key: PreferenceKey::TravelPriority,
                        value: "weight and packed size".into(),
                        source: MemorySource::Conversation,
                    }]),
                    remove: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            planned.plan.add[0].text,
            "Remembered: weight and packed size matter to you when traveling"
        );
        let mut tx = cx.begin().await.unwrap();
        let done = UpdateMyPreferences
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap();
        let again = UpdateMyPreferences
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap();
        assert_eq!(done.id, again.id);
        let out = UndoMemory
            .run(
                &mut tx,
                UndoMemoryInput {
                    memory_id: done.id.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(out["ok"], true);
        let e = UndoMemory
            .run(&mut tx, UndoMemoryInput { memory_id: done.id })
            .await
            .unwrap_err();
        assert_eq!(e.code, "memory_not_found");
    }
}
