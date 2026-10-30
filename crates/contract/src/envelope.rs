//! Response shapes shared by every operation (docs/api/README.md §3.3).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A read. Lists return items and the total in one call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReadResult<T> {
    pub items: Vec<T>,
    pub total: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl<T> ReadResult<T> {
    pub fn all(items: Vec<T>) -> Self {
        let total = items.len() as u64;
        Self {
            items,
            total,
            next_cursor: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeOp {
    Create,
    Update,
    Delete,
    Schedule,
    Send,
    Grant,
    Revoke,
    /// Left out of the request (for example below the stock threshold). Recorded, not run.
    Exclude,
}

/// One row of "what will change".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Change {
    pub op: ChangeOp,
    /// schema.org type in snake_case, for example `unit_price_specification`.
    pub entity: String,
    /// The record's id; empty for a record the write will create (shown as `null`).
    #[serde(serialize_with = "empty_as_null", deserialize_with = "null_as_empty")]
    #[schemars(with = "Option<String>")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub from: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub to: Value,
    /// Short human sentence when the from/to pair is not self-explanatory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn empty_as_null<S: serde::Serializer>(id: &str, s: S) -> Result<S::Ok, S::Error> {
    if id.is_empty() {
        s.serialize_none()
    } else {
        s.serialize_str(id)
    }
}

fn null_as_empty<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// The result of one policy check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Check {
    /// Policy key, for example `max_discount`.
    pub rule: String,
    pub passed: bool,
    pub detail: Option<String>,
    pub fix_tool: Option<String>,
}

/// A write with `preview=true`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Preview {
    pub preview_id: String,
    /// RFC 3339 with the store offset. `None` for a preview in a change set: those do not
    /// expire and are previewed again when a person approves.
    pub expires_at: Option<String>,
    pub changes: Vec<Change>,
    pub checks: Vec<Check>,
    pub needs_approval: bool,
    pub approval_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_set_id: Option<String>,
}

/// Undo is always a person's action, on a whole change set or on one remembered preference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Undo {
    /// Always `"person"`: no tool can undo.
    pub by: String,
    /// The change set (`cs_…`) or memory (`mem_…`) a person can undo.
    pub target: String,
    /// RFC 3339 deadline, or `None` when there is none.
    pub until: Option<String>,
}

/// A write run with a `preview_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommitResult {
    pub ok: bool,
    /// The main id the write produced or changed.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<Undo>,
    /// What the write produced, by slot name (`promotion_id`, `cart`, `memory`…), as the
    /// operation's spec shows its run result.
    #[serde(flatten)]
    pub produced: serde_json::Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_to_a_new_record_has_a_null_id() {
        let c = Change {
            op: ChangeOp::Create,
            entity: "coupon".into(),
            id: String::new(),
            from: Value::Null,
            to: serde_json::json!({"code": "VIPWEEKEND20"}),
            note: None,
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["id"], Value::Null);
        let back: Change = serde_json::from_value(v).unwrap();
        assert_eq!(back, c);
    }
}
