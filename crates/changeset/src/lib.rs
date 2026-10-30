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
//! Every step writes a schema.org `Action` row to `platform.action_log`, and every change-set
//! transition a `platform.change_set_event` row plus a `NOTIFY acom_events`.

pub mod bind;
pub mod change_set;
pub mod preview;
pub mod read;

use acom_contract::{DynOp, ErrorClass, ErrorCode, Registry, ToolError};
use acom_core::enums::ActionStatus;
use acom_store::{Cx, TxCx};
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

pub(crate) fn db_err(e: sqlx::Error) -> ToolError {
    ToolError::from(acom_store::StoreError::from(e))
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
