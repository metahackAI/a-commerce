//! The `action_log` resource: the append-only record of every action, as schema.org `Action`s.
//! Shared here: the row and how it reads as an `Action`.

pub mod get_action_log;
pub mod search_action_log;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_store::{Cx, StoreError};
use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

pub const ACTION_NOT_FOUND: ErrorCode = ErrorCode::new("action_not_found", ErrorClass::NotFound);

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: String,
    action_type: String,
    action_status: String,
    actor_kind: String,
    agent_id: Option<String>,
    agent_name: Option<String>,
    instrument: String,
    object_type: Option<String>,
    object_id: Option<String>,
    result: Option<Value>,
    error: Option<Value>,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    change_set_id: Option<String>,
    preview_id: Option<String>,
    policy_version: Option<String>,
    request_id: Option<String>,
    usage: Option<Value>,
    latency_ms: Option<i32>,
}

const COLUMNS: &str = "id, action_type, action_status, actor_kind, agent_id, agent_name,
    instrument, object_type, object_id, result, error, start_time, end_time, change_set_id,
    preview_id, policy_version, request_id, usage, latency_ms";

/// The schema.org type of an object type word (`unit_price_specification` → `UnitPriceSpecification`).
fn type_name(word: &str) -> String {
    word.split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}

fn action(cx: &Cx, r: &Row) -> Value {
    let agent = match r.actor_kind.as_str() {
        "system" => {
            json!({"@type": "SoftwareApplication", "id": r.agent_id, "name": r.agent_name.as_deref().unwrap_or("system")})
        }
        "agent_on_behalf" | "external_agent" => json!({"@type": "SoftwareApplication",
            "name": r.agent_name, "acom:on_behalf_of": r.agent_id}),
        _ => json!({"@type": "Person", "id": r.agent_id, "name": r.agent_name}),
    };
    let mut out = Map::new();
    out.insert("@type".into(), json!(r.action_type));
    out.insert("id".into(), json!(r.id));
    out.insert("action_status".into(), json!(r.action_status));
    out.insert("agent".into(), agent);
    out.insert("instrument".into(), json!(r.instrument));
    if let (Some(t), Some(id)) = (&r.object_type, &r.object_id) {
        out.insert("object".into(), json!({"@type": type_name(t), "id": id}));
    }
    if let Some(v) = &r.result {
        out.insert("result".into(), v.clone());
    }
    if let Some(e) = &r.error {
        out.insert("error".into(), e.clone());
    }
    out.insert("start_time".into(), json!(cx.time.format(r.start_time)));
    out.insert(
        "end_time".into(),
        json!(r.end_time.map(|t| cx.time.format(t))),
    );
    out.insert("acom:change_set_id".into(), json!(r.change_set_id));
    out.insert("acom:preview_id".into(), json!(r.preview_id));
    out.insert("acom:request_id".into(), json!(r.request_id));
    if let Some(u) = &r.usage {
        out.insert("acom:tokens".into(), u.clone());
    }
    if let Some(ms) = r.latency_ms {
        out.insert("acom:latency_ms".into(), json!(ms));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_object_types_like_schema_org() {
        assert_eq!(
            super::type_name("unit_price_specification"),
            "UnitPriceSpecification"
        );
        assert_eq!(super::type_name("change_set"), "ChangeSet");
    }
}
