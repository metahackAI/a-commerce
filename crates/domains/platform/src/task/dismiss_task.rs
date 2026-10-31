//! Person-only `POST /v1/platform/tasks/{task_id}/dismiss` (`platform.task.dismiss`).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot,
    ToolError,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{TASK_NOT_FOUND, db, not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DismissTaskInput {
    /// The task to put away.
    #[schemars(regex(pattern = r"^task_[a-z0-9_]+$"), example = &"task_linen_reorder")]
    pub task_id: String,
    /// Why, in a sentence. Also closes the task's change set if it was still a draft.
    #[schemars(length(max = 500), example = &"Already handled by phone.")]
    pub reason: Option<String>,
}

/// Person-only: an operator puts a task away from the task list.
pub struct DismissTask;

impl Operation for DismissTask {
    const META: OpMeta = OpMeta {
        address: "platform.task.dismiss",
        tool: "dismiss_task",
        group: Group::Operate,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/platform/tasks/{task_id}/dismiss",
        summary: "Put a task away. Its change set, if still waiting, is rejected with the same reason.",
        consumes: &[Slot::one("task_id")],
        produces: &[],
        checks: &[],
        errors: &[TASK_NOT_FOUND, ErrorCode::INVALID_STATE],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "human:task",
    };
    type Input = DismissTaskInput;
    type Output = Value;
}

impl PersonOp<Cx> for DismissTask {
    async fn run(&self, tx: &mut TxCx, input: DismissTaskInput) -> Result<Value, ToolError> {
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT task_state, change_set_id FROM platform.task
             WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(tx.tenant.as_str())
        .bind(&input.task_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let (state, cs) = row.ok_or_else(|| not_found(&input.task_id))?;
        match state.as_str() {
            "dismissed" => {
                return Ok(json!({"ok": true, "id": input.task_id, "state": "dismissed"}));
            }
            "done" => {
                return Err(
                    ToolError::new(ErrorCode::INVALID_STATE, "This task is already done.")
                        .detail(json!({"entity": "task", "from": "done"})),
                );
            }
            _ => {}
        }
        let reason = input
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .unwrap_or("Dismissed from the task list.");
        let now = tx.clock.now();
        sqlx::query(
            "UPDATE platform.task SET task_state = 'dismissed', end_time = $3,
               summary = coalesce(summary, '') || CASE WHEN summary IS NULL THEN '' ELSE ' · ' END || $4,
               updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&input.task_id)
        .bind(now)
        .bind(format!("Dismissed: {reason}"))
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        let mut closed = None;
        if let Some(cs) = cs {
            let open: Option<String> = sqlx::query_scalar(
                "SELECT change_set_state FROM platform.change_set
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tx.tenant.as_str())
            .bind(&cs)
            .fetch_optional(&mut *tx.tx)
            .await
            .map_err(db)?;
            if matches!(open.as_deref(), Some("drafting" | "waiting_for_approval")) {
                acom_changeset::change_set::reject_in(tx, &cs, reason, true).await?;
                closed = Some(cs);
            }
        }
        acom_changeset::log_action(
            tx,
            acom_changeset::LogEntry {
                object: Some(("task", &input.task_id)),
                result: Some(json!({"state": "dismissed", "reason": reason,
                                    "rejected_change_set": closed})),
                ..acom_changeset::LogEntry::new(
                    ActionType::UPDATE.0,
                    acom_core::enums::ActionStatus::Completed,
                    "dismiss_task",
                )
            },
        )
        .await?;
        let note = json!({"type": "task.updated", "tenant": tx.tenant.as_str(),
                          "task_id": input.task_id, "state": "dismissed"});
        sqlx::query("SELECT pg_notify('acom_events', $1)")
            .bind(note.to_string())
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        Ok(
            json!({"ok": true, "id": input.task_id, "state": "dismissed",
                  "rejected_change_set": closed}),
        )
    }
}
