//! What `customer` needs from `platform`: ending a member's sign-in sessions when their password
//! is reset or changed, or when they withdraw (`password.on_reset`, `password.on_change`).

use acom_contract::ToolError;
use acom_store::TxCx;

use crate::repo::db;

/// Ends every live session of a person (`cus_…`), all of them or all but `keep` (the session
/// that changed the password stays signed in). Returns how many ended.
pub async fn end_sessions(
    tx: &mut TxCx,
    customer_id: &str,
    keep: Option<&str>,
) -> Result<u64, ToolError> {
    let done = sqlx::query(
        "UPDATE platform.session SET revoked_at = now(), updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND subject_id = $2 AND revoked_at IS NULL
           AND ($3::text IS NULL OR id <> $3)",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(keep)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(done.rows_affected())
}
