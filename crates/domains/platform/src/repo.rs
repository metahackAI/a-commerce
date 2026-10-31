//! SQL and secrets shared across the platform's resources.

use std::collections::BTreeSet;

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub(crate) fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// The scopes an operator's roles grant (`platform.admin_user_role` → `platform.role_scope`).
pub(crate) async fn role_scopes_of(
    tx: &mut TxCx,
    admin_user_id: &str,
) -> Result<BTreeSet<String>, ToolError> {
    let scopes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT rs.scope FROM platform.admin_user_role ur
         JOIN platform.role_scope rs ON rs.tenant_id = ur.tenant_id AND rs.role_id = ur.role_id
         WHERE ur.tenant_id = $1 AND ur.admin_user_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(admin_user_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(scopes.into_iter().collect())
}

/// Ends every live session of an API client (`key_…`): its access and refresh tokens stop
/// working. Returns the sessions it ended.
pub(crate) async fn end_client_sessions(
    tx: &mut TxCx,
    client_id: &str,
    at: DateTime<Utc>,
) -> Result<Vec<String>, ToolError> {
    sqlx::query_scalar(
        "UPDATE platform.session SET revoked_at = $3, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND subject_id = $2 AND actor_kind = 'external_agent'
           AND revoked_at IS NULL
         RETURNING id",
    )
    .bind(tx.tenant.as_str())
    .bind(client_id)
    .bind(at)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// SHA-256 of a secret, in lowercase hex: the only form a secret is kept in.
pub(crate) fn sha256_hex(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A new secret: `prefix` and 64 hex characters from a cryptographically secure generator
/// seeded by the operating system.
pub(crate) fn new_secret(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{prefix}{hex}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_random_and_kept_as_hashes() {
        let a = new_secret("ak_live_");
        assert!(a.starts_with("ak_live_") && a.len() == 8 + 64);
        assert_ne!(a, new_secret("ak_live_"));
        assert_eq!(sha256_hex(&a), sha256_hex(&a));
        assert_eq!(sha256_hex(&a).len(), 64);
        assert_ne!(sha256_hex(&a), a);
    }
}
