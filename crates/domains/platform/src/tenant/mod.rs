//! The `tenant` resource: stores (`platform.online_store`, API `tenant`). The store of a request
//! is the token's `tenant` claim alone; row-level security keeps every other store out of reach.

pub mod get_tenant;
pub mod list_tenants;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_store::TxCx;
use serde_json::{Value, json};

use crate::repo::db;

/// No store with that id.
pub const TENANT_NOT_FOUND: ErrorCode = ErrorCode::new("tenant_not_found", ErrorClass::NotFound);

#[derive(Debug, sqlx::FromRow)]
struct StoreRow {
    id: String,
    name: String,
    url: String,
    currency: String,
    time_zone: String,
    in_language: String,
    store_status: String,
    feature_flags: Value,
}

const COLUMNS: &str =
    "id, name, url, currency, time_zone, in_language, store_status, feature_flags";

/// How many policy values the store overrides (leaves of its `platform.policy_override` rows).
async fn overridden_keys(tx: &mut TxCx) -> Result<usize, ToolError> {
    let partials: Vec<Value> =
        sqlx::query_scalar("SELECT partial FROM platform.policy_override WHERE tenant_id = $1")
            .bind(tx.tenant.as_str())
            .fetch_all(&mut *tx.tx)
            .await
            .map_err(db)?;
    Ok(partials
        .iter()
        .map(|p| {
            let mut leaves = Vec::new();
            crate::policy::leaves(p, "", &mut leaves);
            leaves.len()
        })
        .sum())
}

fn store(r: &StoreRow, overrides: usize) -> Value {
    json!({
        "id": r.id, "name": r.name, "url": r.url, "in_language": r.in_language,
        "currency": r.currency.trim(), "acom:time_zone": r.time_zone,
        "acom:feature_flags": r.feature_flags, "acom:policy_overrides": overrides,
        "acom:status": r.store_status,
    })
}
