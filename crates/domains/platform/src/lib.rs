//! The `platform` domain: policies, change sets, tasks, the action log, the data dictionary,
//! the word lists, and who may do what (stores, roles, accounts, organizations, menus, API keys,
//! and the sessions of outside clients). Specified in `docs/api/platform.md`.
//!
//! Layout follows the API hierarchy `platform.{resource}.{operation}`: one folder per resource,
//! one file per operation (`src/{resource}/{tool}.rs`), each operation one type implementing
//! `ReadOp`, `WriteOp`, or `PersonOp` against `acom_store::Cx`; what a resource's operations
//! share is in its `mod.rs`, domain-wide SQL in `repo.rs`, policy values in `rules.rs`. The
//! change-set engine itself lives in `acom-changeset`; this crate exposes its reads as tools.
//!
//! People's browser sign-in is the gateway's (`/api/auth/*`); the `auth` tools here are for
//! outside agents and integration clients that sign in with an API key.

pub mod rules;
pub mod service;

mod action_log;
mod admin_user;
mod api_key;
mod auth;
mod change_set;
mod r#enum;
mod menu;
mod org;
mod policy;
mod repo;
mod role;
mod schema;
mod task;
mod tenant;

#[cfg(test)]
mod testdb;

use acom_contract::{ApprovalNeed, ErrorCode, RegistryBuilder, ToolError};
use acom_store::{Cx, Page};
use serde_json::{Value, json};

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
        .read(r#enum::list_enums::ListEnums)
        .write(auth::login::Login)
        .write(auth::refresh_token::RefreshToken)
        .write(auth::logout::Logout)
        .read(tenant::get_tenant::GetTenant)
        .read(tenant::list_tenants::ListTenants)
        .read(role::list_roles::ListRoles)
        .read(role::get_role::GetRole)
        .write(role::create_role::CreateRole)
        .write(role::assign_role::AssignRole)
        .read(role::list_permissions::ListPermissions)
        .read(menu::list_menus::ListMenus)
        .write(menu::set_menu::SetMenu)
        .write(menu::set_favorite::SetFavorite)
        .read(admin_user::search_admin_users::SearchAdminUsers)
        .write(admin_user::create_admin_user::CreateAdminUser)
        .write(admin_user::set_admin_status::SetAdminStatus)
        .read(org::list_orgs::ListOrgs)
        .write(org::set_org::SetOrg)
        .read(api_key::list_api_keys::ListApiKeys)
        .write(api_key::issue_api_key::IssueApiKey)
        .write(api_key::revoke_api_key::RevokeApiKey);
}

/// Shared by the reads: a time input in store time.
pub(crate) fn parse_time(
    cx: &Cx,
    field: &str,
    s: Option<&str>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, ToolError> {
    s.map(|s| {
        cx.time.parse(s).map_err(|e| {
            ToolError::invalid_input(e.to_string())
                .detail(json!({"field": field, "reason": "format"}))
        })
    })
    .transpose()
}

/// The scope that manages the platform: roles, accounts, organizations, menus, API keys and the
/// store list. Only the platform administrator role carries it (`platform.role_scope`).
pub(crate) const ADMIN_SCOPE: &str = "tools:write:platform";

/// The administrator-only reads and writes ("관리자 역할만"): the caller, or the person the agent
/// acts for, must hold [`ADMIN_SCOPE`]; the change-set runner (a system actor) always may.
pub(crate) fn require_admin(cx: &Cx, next: Option<&'static str>) -> Result<(), ToolError> {
    let p = &cx.principal;
    if p.kind == acom_core::enums::ActorKind::System || p.has_scope(ADMIN_SCOPE) {
        return Ok(());
    }
    let e = ToolError::new(
        ErrorCode::FORBIDDEN,
        "Only a platform administrator can see or change who may do what in this store.",
    )
    .detail(json!({"reason": "role", "required_scope": ADMIN_SCOPE}));
    Err(match next {
        Some(tool) => e.next(tool),
        None => e,
    })
}

/// Lowercase words joined by `_`, for ids made from names: `Returns desk` → `returns_desk`.
pub(crate) fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() { "x".into() } else { out }
}

/// A list envelope `{items, total, next_cursor?}`.
pub(crate) fn list(items: Vec<Value>, total: i64, page: &Page) -> Value {
    let returned = items.len();
    let mut out = json!({"items": items, "total": total});
    if let Some(c) = page.next_cursor(returned, total) {
        out["next_cursor"] = json!(c);
    }
    out
}

/// What `approval.yml` says about a write of kind W, from the facts its preview computed.
pub(crate) fn policy_approval(cx: &Cx, tool: &str, facts: &Value) -> ApprovalNeed {
    let a = cx.policy.approval_for(tool, facts);
    match a.reason {
        Some(reason) if a.needed => ApprovalNeed::Needed { reason },
        _ if a.needed => ApprovalNeed::Needed {
            reason: format!("The store asks a person to approve {tool}"),
        },
        _ => ApprovalNeed::NotNeeded,
    }
}

/// The text of a required field, trimmed; `invalid_input` when it is empty.
pub(crate) fn required_text(field: &str, value: &str) -> Result<String, ToolError> {
    let t = value.trim();
    if t.is_empty() {
        return Err(
            ToolError::invalid_input(format!("`{field}` cannot be empty."))
                .detail(json!({"field": field, "reason": "missing"})),
        );
    }
    Ok(t.to_owned())
}
