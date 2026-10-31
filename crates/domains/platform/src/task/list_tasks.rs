//! `list_tasks` (`platform.task.list`): the operator's task list.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::{TaskOrigin, TaskState};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{COLUMNS, TaskRow, db};
use crate::parse_time;

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
