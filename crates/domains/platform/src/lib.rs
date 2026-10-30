//! The `platform` domain: policies, change sets, tasks, the action log, the data dictionary,
//! and the word lists. Specified in `docs/api/platform.md`.
//!
//! Layout: one module per resource; each operation is one type implementing `ReadOp`,
//! `WriteOp`, or `PersonOp` against `acom_store::Cx`. The change-set engine itself lives in
//! `acom-changeset`; this crate exposes its reads as tools.

mod action_log;
mod change_set;
mod enums;
mod policy;
mod schema;
mod task;

use acom_contract::RegistryBuilder;
use acom_store::Cx;

pub use task::TASK_NOT_FOUND;

/// Adds every platform operation to the registry.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    b.read(policy::GetPolicy)
        .read(policy::ListPolicies)
        .read(change_set::GetChangeSet)
        .read(change_set::SearchChangeSets)
        .read(change_set::VerifyChangeSet)
        .read(task::ListTasks)
        .read(task::GetTask)
        .person(task::DismissTask)
        .read(action_log::SearchActionLog)
        .read(action_log::GetActionLog)
        .read(schema::DescribeData)
        .read(enums::ListEnums);
}

/// Shared by the reads: a time input in store time.
pub(crate) fn parse_time(
    cx: &Cx,
    field: &str,
    s: Option<&str>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, acom_contract::ToolError> {
    s.map(|s| {
        cx.time.parse(s).map_err(|e| {
            acom_contract::ToolError::invalid_input(e.to_string())
                .detail(serde_json::json!({"field": field, "reason": "format"}))
        })
    })
    .transpose()
}
