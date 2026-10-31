//! What `partner` needs from `platform`: the organization an operator belongs to. A vendor
//! organization's operators act for the seller linked to it, so they see their own seller's
//! dashboard (role `seller`) and their own unmasked details.

use acom_contract::ToolError;
use acom_store::TxCx;

use crate::repo::db;

/// The organization (`org_…`) of an operator (`adm_…`); `None` when the operator belongs to
/// none or does not exist.
pub async fn organization_of(tx: &mut TxCx, admin_id: &str) -> Result<Option<String>, ToolError> {
    let org: Option<Option<String>> = sqlx::query_scalar(
        "SELECT organization_id FROM platform.admin_user WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(admin_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(org.flatten())
}
