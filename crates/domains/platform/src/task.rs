//! `list_tasks`, `get_task`, and the person-only `POST /v1/platform/tasks/{task_id}/dismiss`.

use acom_contract::{
    ActionType, Audience, ErrorClass, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp,
    ReadOp, Slot, ToolError,
};
use acom_core::enums::{TaskOrigin, TaskState};
use acom_store::{Cx, Page, StoreError, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::parse_time;

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
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

const COLUMNS: &str = "id, title, task_state, task_origin, summary, tag, request_text, watcher,
    watcher_observation, related_entities, change_set_id, conversation_id, start_time, end_time,
    created_at, updated_at";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTasksInput {
    /// Only these states. Default: needs_you, working, done.
    #[schemars(example = ["needs_you"])]
    pub state: Option<Vec<TaskState>>,
    /// Only tasks that came this way. All when left out.
    #[schemars(example = ["agent_noticed", "automation"])]
    pub origin: Option<Vec<TaskOrigin>>,
    /// Only tasks created or changed after this time (store time).
    #[schemars(example = &"2026-11-06T00:00")]
    pub since: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListTasks;

impl Operation for ListTasks {
    const META: OpMeta = OpMeta {
        address: "platform.task.list",
        tool: "list_tasks",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/tasks",
        summary: "The operator's task list: what needs them now, what the agent is working on, what finished on its own.",
        consumes: &[],
        produces: &[Slot::many("task_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListTasksInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListTasks {
    async fn read(&self, cx: &Cx, input: ListTasksInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let states: Vec<&str> = input
            .state
            .unwrap_or_else(|| vec![TaskState::NeedsYou, TaskState::Working, TaskState::Done])
            .iter()
            .map(|s| s.as_str())
            .collect();
        let origins: Vec<&str> = input
            .origin
            .unwrap_or_default()
            .iter()
            .map(|o| o.as_str())
            .collect();
        let since = parse_time(cx, "since", input.since.as_deref())?;
        let filter = "tenant_id = $1 AND task_state = ANY($2)
            AND (cardinality($3::text[]) = 0 OR task_origin = ANY($3))
            AND ($4::timestamptz IS NULL OR greatest(start_time, updated_at) >= $4)";
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM platform.task WHERE {filter}"
        )))
        .bind(cx.tenant.as_str())
        .bind(&states)
        .bind(&origins)
        .bind(since)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        // Needs you first, then working, then the rest; newest first inside each.
        let rows = sqlx::query_as::<_, TaskRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM platform.task WHERE {filter}
             ORDER BY CASE task_state WHEN 'needs_you' THEN 0 WHEN 'working' THEN 1 ELSE 2 END,
                      start_time DESC
             LIMIT $5 OFFSET $6"
        )))
        .bind(cx.tenant.as_str())
        .bind(&states)
        .bind(&origins)
        .bind(since)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|t| {
                json!({"id": t.id, "title": t.title, "state": t.task_state,
                       "origin": t.task_origin, "at": cx.time.format(t.start_time),
                       "summary": t.summary, "tag": t.tag, "change_set_id": t.change_set_id,
                       "watcher": t.watcher, "conversation_id": t.conversation_id,
                       "updated_at": cx.time.format(t.updated_at)})
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

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
            "created_at": cx.time.format(t.created_at), "updated_at": cx.time.format(t.updated_at),
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
