//! Previews and change sets (docs/backend/commerce.md §2–3, docs/api/platform.md#change_set).
//!
//! - [`preview`] stores what a write would do (plan, changes, checks, the row versions it read,
//!   the policy version) and returns it; nothing in the store changes.
//! - [`commit`] runs a stored preview on its own: only a write that needs no approval and is not
//!   part of a change set, while the preview is open and still right. The same preview twice
//!   returns the first result.
//! - [`change_set`] groups previews. A person approves exactly what will run: approval previews
//!   every item again and refuses (`estimate_changed`, back to drafting) if anything differs. The
//!   runner commits the items in order, binding `$item[n].slot` placeholders to what earlier
//!   items produced, reads each back (`verify`), and moves the set to `live` or `failed`. Undo
//!   takes every item back in reverse order until the deadline.
//!
//! - [`jobs`] runs scheduled work (`platform.job`) with the handlers domains register, writes
//!   the tasks watchers find, and drafts their fixes as change sets.
//! - [`clock`] reads and moves a store's demo clock; moving it forward runs what comes due.
//!
//! Every step writes a schema.org `Action` row to `platform.action_log`, and every change-set
//! transition a `platform.change_set_event` row plus a `NOTIFY acom_events`.

pub mod bind;
pub mod change_set;
pub mod clock;
pub mod jobs;
pub mod preview;
pub mod read;

use acom_contract::{DynOp, ErrorClass, ErrorCode, Registry, ToolError};
use acom_core::enums::ActionStatus;
use acom_store::{Cx, TxCx};
use rust_decimal::Decimal;
use serde_json::{Value, json};

pub use change_set::{ItemRef, NewChangeSet};
pub use preview::{ClaimedPreview, claim_preview, commit, mark_committed, preview};

pub const CHANGE_SET_NOT_FOUND: ErrorCode =
    ErrorCode::new("change_set_not_found", ErrorClass::NotFound);
pub const CHANGE_SET_NOT_RUN: ErrorCode =
    ErrorCode::new("change_set_not_run", ErrorClass::InvalidState);
pub const IRREVERSIBLE_CHANGE: ErrorCode =
    ErrorCode::new("irreversible_change", ErrorClass::InvalidState);
pub const ALREADY_EXISTS: ErrorCode = ErrorCode::new("already_exists", ErrorClass::AlreadyExists);

/// The system actor the change-set runner acts as.
pub const RUNNER: &str = "sys_runner";

/// The slot of `Committed::produced` that a run shows its caller once and never keeps: a new API
/// key's secret, a client's tokens. It is taken out before the result is stored in the preview,
/// the change set or the action log, and answers only the request that ran the write (the run
/// response, or the approval response for a change set). Running the same preview again answers
/// without it.
pub const SHOWN_ONCE: &str = "shown_once";

/// What a stored input holds in place of a field its schema marks `writeOnly` (a secret).
pub const NOT_STORED: &str = "(not stored)";

/// Takes the [`SHOWN_ONCE`] values out of a run's result.
pub(crate) fn take_shown_once(
    done: &mut acom_contract::Committed,
) -> Option<serde_json::Map<String, Value>> {
    match done.produced.remove(SHOWN_ONCE) {
        Some(Value::Object(m)) if !m.is_empty() => Some(m),
        _ => None,
    }
}

/// The input as the preview store keeps it: every top-level field the operation's input schema
/// marks `"writeOnly": true` (a client secret, a refresh token) is replaced by [`NOT_STORED`].
/// The operation's preview saw the real value; nothing stored or logged holds it.
pub fn storable_input(schema: &Value, input: &Value) -> Value {
    let mut out = input.clone();
    let props = schema.get("properties").and_then(Value::as_object);
    if let (Some(props), Some(fields)) = (props, out.as_object_mut()) {
        for (name, value) in fields.iter_mut() {
            let secret = props
                .get(name)
                .and_then(|p| p.get("writeOnly"))
                .and_then(Value::as_bool)
                == Some(true);
            if secret {
                *value = Value::String(NOT_STORED.to_owned());
            }
        }
    }
    out
}

pub(crate) fn db_err(e: sqlx::Error) -> ToolError {
    ToolError::from(acom_store::StoreError::from(e))
}

/// Sends one progress event (`change_set.updated`, `task.updated`, `job.updated`) on the
/// `acom_events` channel. Postgres delivers it when the transaction commits, and not at all if
/// it rolls back; `GET /v1/platform/events` passes it on to the store's listeners.
pub async fn notify(tx: &mut TxCx, kind: &str, body: Value) -> Result<(), ToolError> {
    let mut note = body;
    if let Value::Object(o) = &mut note {
        o.insert("type".into(), json!(kind));
        o.insert("tenant".into(), json!(tx.tenant.as_str()));
    }
    sqlx::query("SELECT pg_notify('acom_events', $1)")
        .bind(note.to_string())
        .execute(&mut *tx.tx)
        .await
        .map_err(db_err)?;
    Ok(())
}

pub(crate) fn op<'r>(
    registry: &'r Registry<Cx>,
    tool: &str,
) -> Result<&'r std::sync::Arc<dyn DynOp<Cx>>, ToolError> {
    registry.get(tool).ok_or_else(|| {
        ToolError::new(
            ErrorCode::NOT_FOUND,
            format!("There is no tool named `{tool}`."),
        )
    })
}

/// The operations whose effects cannot be taken back (`approval.change_set.irreversible`).
pub(crate) fn irreversible(cx: &Cx, tool: &str) -> bool {
    cx.policy
        .raw("approval")
        .and_then(|p| p.pointer("/change_set/irreversible"))
        .and_then(Value::as_array)
        .is_some_and(|list| list.iter().any(|t| t.as_str() == Some(tool)))
}

/// Whether a fresh preview's changes say the same as the stored ones. Numbers are equal by value,
/// so a price stored as `26.00` and read back from a seed file as `26.0` is the same price; with
/// `arbitrary_precision`, `serde_json` would compare their text.
pub(crate) fn same_changes(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            x == y
                || matches!(
                    (x.to_string().parse::<Decimal>(), y.to_string().parse::<Decimal>()),
                    (Ok(x), Ok(y)) if x == y
                )
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(x, y)| same_changes(x, y))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| same_changes(v, w)))
        }
        _ => a == b,
    }
}

/// The record a completed write is about: the entity of its first change and the id it produced.
pub(crate) fn main_object<'a>(changes: &'a Value, id: &'a str) -> Option<(&'a str, &'a str)> {
    let entity = changes.as_array()?.first()?.get("entity")?.as_str()?;
    (!id.is_empty()).then_some((entity, id))
}

/// One action-log row (schema.org `Action`).
#[derive(Debug)]
pub struct LogEntry<'a> {
    pub action_type: &'a str,
    pub status: ActionStatus,
    pub instrument: &'a str,
    pub object: Option<(&'a str, &'a str)>,
    pub result: Option<Value>,
    pub error: Option<&'a ToolError>,
    pub change_set_id: Option<&'a str>,
    pub preview_id: Option<&'a str>,
}

impl<'a> LogEntry<'a> {
    pub fn new(action_type: &'a str, status: ActionStatus, instrument: &'a str) -> Self {
        Self {
            action_type,
            status,
            instrument,
            object: None,
            result: None,
            error: None,
            change_set_id: None,
            preview_id: None,
        }
    }
}

/// Appends one action to `platform.action_log`, inside the caller's transaction.
pub async fn log_action(tx: &mut TxCx, e: LogEntry<'_>) -> Result<(), ToolError> {
    let now = tx.clock.now();
    let p = tx.principal.clone();
    sqlx::query(
        "INSERT INTO platform.action_log (tenant_id, id, action_type, action_status, actor_kind,
           agent_id, agent_name, instrument, object_type, object_id, result, error, start_time,
           end_time, change_set_id, preview_id, policy_version, request_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$13,$14,$15,$16,$17)",
    )
    .bind(tx.tenant.as_str())
    .bind(acom_core::ids::ActionId::generate().as_str())
    .bind(e.action_type)
    .bind(e.status.as_str())
    .bind(p.kind.as_str())
    .bind(&p.subject)
    .bind(p.name.as_deref())
    .bind(e.instrument)
    .bind(e.object.map(|o| o.0))
    .bind(e.object.map(|o| o.1))
    .bind(e.result)
    .bind(
        e.error
            .map(|x| json!({"code": x.code, "class": x.class, "message": x.message})),
    )
    .bind(now)
    .bind(e.change_set_id)
    .bind(e.preview_id)
    .bind(tx.policy.version())
    .bind(&tx.request_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_compare_numbers_by_value() {
        let stored: Value =
            serde_json::from_str(r#"[{"to": {"price": 26.00, "units": 240}}]"#).unwrap();
        let seeded: Value =
            serde_json::from_str(r#"[{"to": {"price": 26.0, "units": 240}}]"#).unwrap();
        let raised: Value =
            serde_json::from_str(r#"[{"to": {"price": 26.5, "units": 240}}]"#).unwrap();
        let more: Value =
            serde_json::from_str(r#"[{"to": {"price": 26.00, "units": 240, "note": null}}]"#)
                .unwrap();
        assert_ne!(stored, seeded, "serde_json compares the text");
        assert!(same_changes(&stored, &seeded));
        assert!(!same_changes(&stored, &raised));
        assert!(!same_changes(&stored, &more));
    }

    #[test]
    fn secrets_are_neither_stored_nor_kept() {
        let schema = json!({"type": "object", "properties": {
            "client_id": {"type": "string"},
            "client_secret_ref": {"type": "string", "writeOnly": true}}});
        let input = json!({"client_id": "key_31", "client_secret_ref": "ak_live_3f9c"});
        assert_eq!(
            storable_input(&schema, &input),
            json!({"client_id": "key_31", "client_secret_ref": NOT_STORED})
        );
        let mut done = acom_contract::Committed {
            id: "key_31".into(),
            produced: serde_json::from_value(
                json!({"api_key_id": "key_31", "shown_once": {"secret": "ak_live_3f9c"}}),
            )
            .unwrap(),
        };
        assert_eq!(
            take_shown_once(&mut done).map(Value::Object),
            Some(json!({"secret": "ak_live_3f9c"}))
        );
        assert_eq!(json!(done.produced), json!({"api_key_id": "key_31"}));
        assert_eq!(take_shown_once(&mut done), None);
    }
}
