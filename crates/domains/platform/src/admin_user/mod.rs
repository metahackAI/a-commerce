//! The `admin_user` resource: operator accounts (`platform.admin_user`, API `admin_user_id`).
//! Signing in is the gateway's; these tools find, create and suspend accounts.

pub mod create_admin_user;
pub mod search_admin_users;
pub mod set_admin_status;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::repo::db;

/// No operator account with that id.
pub const ADMIN_USER_NOT_FOUND: ErrorCode =
    ErrorCode::new("admin_user_not_found", ErrorClass::NotFound);
/// Another account already signs in with this email.
pub const DUPLICATE_EMAIL: ErrorCode = ErrorCode::new("duplicate_email", ErrorClass::AlreadyExists);
/// A role id that is not one of the store's roles.
pub const INVALID_ROLE: ErrorCode = ErrorCode::new("invalid_role", ErrorClass::InvalidInput);
/// The account is already in that state.
pub const ALREADY_IN_STATE: ErrorCode =
    ErrorCode::new("already_in_state", ErrorClass::InvalidState);

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct AdminRow {
    pub id: String,
    pub name: String,
    pub email: String,
    pub admin_status: String,
    pub organization_id: Option<String>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub version: i32,
}

pub(crate) async fn admin(tx: &mut TxCx, id: &str) -> Result<Option<AdminRow>, ToolError> {
    sqlx::query_as(
        "SELECT id, name, email, admin_status, organization_id, last_login_at, version
         FROM platform.admin_user WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) fn not_found(id: &str) -> ToolError {
    ToolError::new(
        ADMIN_USER_NOT_FOUND,
        format!("There is no operator account {id}."),
    )
    .detail(json!({"entity": "admin_user", "id": id}))
    .next("search_admin_users")
}

/// An email with the name part hidden after its first letter: `s***@northwind.example`.
/// Lists show accounts without their personal data by default.
pub(crate) fn masked_email(email: &str) -> String {
    match email.split_once('@') {
        Some((name, domain)) => {
            let first: String = name.chars().take(1).collect();
            format!("{first}***@{domain}")
        }
        None => "***".into(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn emails_are_masked_in_lists() {
        assert_eq!(
            super::masked_email("sj@northwind.example"),
            "s***@northwind.example"
        );
        assert_eq!(super::masked_email("nobody"), "***");
    }
}
