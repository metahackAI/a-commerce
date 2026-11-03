//! The `task` resource: the operator's tasks (`list_tasks`, `get_task`) and the person-only
//! `POST /v1/platform/tasks/{task_id}/dismiss`. Shared here: the task row and its errors.

pub mod dismiss_task;
pub mod get_task;
pub mod list_tasks;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_store::StoreError;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

pub const TASK_NOT_FOUND: ErrorCode = ErrorCode::new("task_not_found", ErrorClass::NotFound);

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

fn not_found(id: &str) -> ToolError {
    ToolError::new(TASK_NOT_FOUND, format!("There is no task {id}."))
        .detail(json!({"entity": "task", "id": id}))
        .next("list_tasks")
}

#[derive(Debug, sqlx::FromRow)]
struct TaskRow {
    id: String,
    title: String,
    task_state: String,
    task_origin: String,
    summary: Option<String>,
    tag: Option<String>,
    request_text: Option<String>,
    watcher: Option<String>,
    watcher_observation: Option<Value>,
    related_entities: Value,
    change_set_id: Option<String>,
    conversation_id: Option<String>,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, title, task_state, task_origin, summary, tag, request_text, watcher,
    watcher_observation, related_entities, change_set_id, conversation_id, start_time, end_time,
    updated_at";

/// The gateway handed a task to the operations agent and the turn ended without a change set
/// (internal `POST /v1/platform/tasks/{task_id}/pickup-ended`, docs/api/platform.md): the task
/// keeps the conversation and says what the agent answered, so it shows as waiting for a person
/// instead of still being drafted. A task that is closed or has a change set is left alone.
pub async fn note_undrafted(
    tx: &mut acom_store::TxCx,
    task: &str,
    conversation: Option<&str>,
    reply: &str,
) -> Result<Value, ToolError> {
    let said: String = reply.split_whitespace().collect::<Vec<_>>().join(" ");
    let said: String = said.chars().take(300).collect();
    let summary = if said.is_empty() {
        "The agent did not draft a change; open it to continue.".to_owned()
    } else {
        format!("The agent did not draft a change: {said}")
    };
    let state: Option<String> = sqlx::query_scalar(
        "UPDATE platform.task SET summary = $3, conversation_id = coalesce(conversation_id, $4),
           task_state = 'needs_you', updated_at = $5, version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND change_set_id IS NULL
           AND task_state IN ('needs_you', 'working')
         RETURNING task_state",
    )
    .bind(tx.tenant.as_str())
    .bind(task)
    .bind(&summary)
    .bind(conversation)
    .bind(tx.clock.now())
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    if state.is_some() {
        acom_changeset::notify(
            tx,
            "task.updated",
            json!({"task_id": task, "state": "needs_you"}),
        )
        .await?;
    }
    Ok(json!({"task_id": task, "noted": state.is_some()}))
}
