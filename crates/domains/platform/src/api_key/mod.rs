//! The `api_key` resource: credentials outside agents and integration clients sign in with
//! (`platform.api_key`). A key's secret is shown once, when it is issued, and kept only as a
//! SHA-256 hash; lists show the first characters.

pub mod issue_api_key;
pub mod list_api_keys;
pub mod revoke_api_key;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::repo::db;

/// No API key with that id.
pub const API_KEY_NOT_FOUND: ErrorCode = ErrorCode::new("api_key_not_found", ErrorClass::NotFound);
/// An active key already has this name.
pub const DUPLICATE_KEY_NAME: ErrorCode =
    ErrorCode::new("duplicate_key_name", ErrorClass::AlreadyExists);
/// A scope the key (or the session) may not get.
pub const SCOPE_NOT_GRANTED: ErrorCode = ErrorCode::new("scope_not_granted", ErrorClass::Forbidden);
/// The key or session was already revoked.
pub const ALREADY_REVOKED: ErrorCode = ErrorCode::new("already_revoked", ErrorClass::InvalidState);

/// What every secret starts with, so a leaked one is recognizable as ours.
pub(crate) const SECRET_PREFIX: &str = "ak_live_";
/// How many characters of the secret lists show (`ak_live_3f`).
pub(crate) const SHOWN_PREFIX_LEN: usize = 10;

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct KeyRow {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub secret_hash: String,
    pub scopes: Vec<String>,
    pub api_key_status: String,
    pub expires: Option<DateTime<Utc>>,
    pub creator_id: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub failed_attempt_count: i32,
    pub first_failed_at: Option<DateTime<Utc>>,
    pub locked_until: Option<DateTime<Utc>>,
    pub version: i32,
}

pub(crate) const COLUMNS: &str = "id, name, key_prefix, secret_hash, scopes, api_key_status,
    expires, creator_id, last_used_at, revoked_at, failed_attempt_count, first_failed_at,
    locked_until, version";

pub(crate) async fn key(tx: &mut TxCx, id: &str) -> Result<Option<KeyRow>, ToolError> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM platform.api_key WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) fn not_found(id: &str) -> ToolError {
    ToolError::new(API_KEY_NOT_FOUND, format!("There is no API key {id}."))
        .detail(json!({"entity": "api_key", "id": id}))
        .next("list_api_keys")
}
