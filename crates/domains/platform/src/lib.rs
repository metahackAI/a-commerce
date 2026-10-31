//! The `platform` domain: policies, change sets, tasks, the action log, the data dictionary,
//! and the word lists. Specified in `docs/api/platform.md`.
//!
//! Layout follows the API hierarchy `platform.{resource}.{operation}`: one folder per resource,
//! one file per operation (`src/{resource}/{tool}.rs`), each operation one type implementing
//! `ReadOp`, `WriteOp`, or `PersonOp` against `acom_store::Cx`; what a resource's operations
//! share is in its `mod.rs`. The change-set engine itself lives in `acom-changeset`; this crate
//! exposes its reads as tools.

mod action_log;
mod change_set;
mod r#enum;
mod policy;
mod schema;
mod task;

use acom_contract::RegistryBuilder;
use acom_store::Cx;

pub use task::TASK_NOT_FOUND;

/// Adds every platform operation to the registry.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    b.read(policy::get_policy::GetPolicy)
        .read(policy::list_policies::ListPolicies)
        .read(change_set::get_change_set::GetChangeSet)
        .read(change_set::search_change_sets::SearchChangeSets)
        .read(change_set::verify_change_set::VerifyChangeSet)
        .read(task::list_tasks::ListTasks)
        .read(task::get_task::GetTask)
        .person(task::dismiss_task::DismissTask)
        .read(action_log::search_action_log::SearchActionLog)
        .read(action_log::get_action_log::GetActionLog)
        .read(schema::describe_data::DescribeData)
        .read(r#enum::list_enums::ListEnums);
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
