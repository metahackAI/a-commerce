//! `undo_memory`.

use crate::repo::{self};
use crate::{MEMORY_NOT_FOUND, NOT_SIGNED_IN};
use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot,
    ToolError,
};
use acom_core::enums::ActionStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

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
    fn undo_is_person_only() {
        let s = closed_schema_for::<UndoMemoryInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(UndoMemory::META.kind, Kind::Person);
        assert_eq!(UndoMemory::META.person_scope, "human:undo");
    }
}
