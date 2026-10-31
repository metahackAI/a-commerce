//! `get_task` (`platform.task.get`): one task in full.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{COLUMNS, TASK_NOT_FOUND, TaskRow, db, not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetTaskInput {
    /// The task.
    #[schemars(regex(pattern = r"^task_[a-z0-9_]+$"), example = &"task_returns_1106")]
    pub task_id: String,
}

pub struct GetTask;

impl Operation for GetTask {
    const META: OpMeta = OpMeta {
        address: "platform.task.get",
        tool: "get_task",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/tasks/{task_id}",
        summary: "One task in full: why it exists (the watcher's threshold and what it observed), its history, and the linked conversation, change set and records.",
        consumes: &[Slot::one("task_id")],
        produces: &[Slot::one("change_set_id")],
        checks: &[],
        errors: &[TASK_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetTaskInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetTask {
    async fn read(&self, cx: &Cx, input: GetTaskInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let t = sqlx::query_as::<_, TaskRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM platform.task WHERE tenant_id = $1 AND id = $2"
        )))
        .bind(cx.tenant.as_str())
        .bind(&input.task_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?
        .ok_or_else(|| not_found(&input.task_id))?;
        // The history: when the task started, what its change set went through, how it ended.
        #[allow(clippy::type_complexity)]
        let events: Vec<(DateTime<Utc>, Option<String>, String, Option<i32>)> =
            match &t.change_set_id {
                Some(cs) => sqlx::query_as(
                    "SELECT e.start_time, e.to_state, e.action_type, c.number
                 FROM platform.change_set_event e
                 JOIN platform.change_set c ON c.tenant_id = e.tenant_id AND c.id = e.change_set_id
                 WHERE e.tenant_id = $1 AND e.change_set_id = $2 ORDER BY e.position",
                )
                .bind(cx.tenant.as_str())
                .bind(cs)
                .fetch_all(&mut *tx.tx)
                .await
                .map_err(db)?,
                None => Vec::new(),
            };
        tx.commit().await.map_err(ToolError::from)?;
        let mut history = vec![
            json!({"at": cx.time.format(t.start_time), "state": "started",
            "note": match t.task_origin.as_str() {
                "you_asked" => "You asked for this",
                "agent_noticed" => "A watcher noticed this and the agent took it",
                _ => "An automation handled this",
            }}),
        ];
        for (at, to, action, number) in events {
            history.push(json!({"at": cx.time.format(at), "state": to,
                "note": format!("Change set #{} {}", number.unwrap_or(0),
                    change_set_note(to.as_deref(), &action))}));
        }
        if let Some(end) = t.end_time {
            history.push(json!({"at": cx.time.format(end), "state": t.task_state,
                                "note": format!("Task {}", t.task_state.replace('_', " "))}));
        }
        let watcher = t.watcher.as_ref().map(|key| {
            let mut w = t.watcher_observation.clone().unwrap_or_else(|| json!({}));
            w["key"] = json!(key);
            w
        });
        Ok(json!({
            "id": t.id, "title": t.title, "state": t.task_state, "origin": t.task_origin,
            "tag": t.tag, "summary": t.summary, "request": t.request_text, "watcher": watcher,
            "related": {"change_set_id": t.change_set_id, "conversation_id": t.conversation_id,
                        "entities": t.related_entities},
            "history": history,
            // When the task appeared on the store's clock; the row's own `created_at` is the
            // database's wall clock (a seed load, or real time under the demo clock).
            "created_at": cx.time.format(t.start_time), "updated_at": cx.time.format(t.updated_at),
        }))
    }
}

fn change_set_note(to: Option<&str>, action: &str) -> String {
    match (to, action) {
        (Some("drafting"), "CreateAction") => "drafted".into(),
        (Some("drafting"), _) => "changed".into(),
        (Some("waiting_for_approval"), _) => "ready for your approval".into(),
        (Some("running"), _) => "approved and running".into(),
        (Some("verifying"), _) => "checking the results".into(),
        (Some("live"), _) => "live, verified".into(),
        (Some(other), _) => other.replace('_', " "),
        (None, a) => a.to_owned(),
    }
}
