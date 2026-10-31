//! The `org` resource: organizations operators belong to (`platform.organization`, API `org`):
//! departments, vendors and branches, nested.

pub mod list_orgs;
pub mod set_org;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_store::TxCx;
use serde_json::json;

use crate::repo::db;

/// No organization with that id.
pub const ORG_NOT_FOUND: ErrorCode = ErrorCode::new("org_not_found", ErrorClass::NotFound);
/// The parent would be the organization itself or one of its own sub-organizations.
pub const CYCLE_DETECTED: ErrorCode = ErrorCode::new("cycle_detected", ErrorClass::Domain);

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct OrgRow {
    pub id: String,
    pub name: String,
    pub organization_type: String,
    pub parent_organization_id: Option<String>,
    pub version: i32,
}

pub(crate) async fn org(tx: &mut TxCx, id: &str) -> Result<Option<OrgRow>, ToolError> {
    sqlx::query_as(
        "SELECT id, name, organization_type, parent_organization_id, version
         FROM platform.organization WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) fn not_found(id: &str) -> ToolError {
    ToolError::new(ORG_NOT_FOUND, format!("There is no organization {id}."))
        .detail(json!({"entity": "org", "id": id}))
        .next("list_orgs")
}

/// The organization and every organization under it, at any depth.
pub(crate) async fn subtree(tx: &mut TxCx, id: &str) -> Result<Vec<String>, ToolError> {
    sqlx::query_scalar(
        "WITH RECURSIVE under(id) AS (
           SELECT id FROM platform.organization WHERE tenant_id = $1 AND id = $2
           UNION
           SELECT o.id FROM platform.organization o JOIN under u ON o.parent_organization_id = u.id
           WHERE o.tenant_id = $1)
         SELECT id FROM under",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}
