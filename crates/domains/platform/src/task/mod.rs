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
