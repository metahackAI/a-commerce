//! The `cache` resource (docs/api/merchandising.md#cache): clearing what store pages cache of
//! the display (category menus, slots, banners, popups, curated pages).
//!
//! The store runs no display cache yet: pages read slots, banners and popups when they are asked
//! (`display.schedule_compute: on_demand`). `invalidate_display_cache` is therefore a
//! deterministic stand-in: it works out the cache keys a clear would cover, records the request
//! (`merchandising.cache_invalidation`, `cin_`) so the hourly limit on clearing everything holds,
//! and says that nothing was cleared. A real cache plugs in behind the same tool.

pub mod invalidate_display_cache;

use acom_contract::ToolError;
use acom_store::TxCx;
use chrono::{DateTime, Utc};

use crate::repo::db;

/// A recorded clear.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct ClearRow {
    pub id: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub page: Vec<String>,
    pub cache_key: Vec<String>,
    pub start_time: DateTime<Utc>,
}

/// When the clears of everything since `since` ran, oldest first.
pub(crate) async fn full_clears_since(
    tx: &mut TxCx,
    since: DateTime<Utc>,
) -> Result<Vec<DateTime<Utc>>, ToolError> {
    sqlx::query_scalar(
        "SELECT start_time FROM merchandising.cache_invalidation
         WHERE tenant_id = $1 AND resource_type = 'all' AND start_time > $2
         ORDER BY start_time",
    )
    .bind(tx.tenant.as_str())
    .bind(since)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) async fn insert(
    tx: &mut TxCx,
    c: &ClearRow,
    reason: Option<&str>,
) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO merchandising.cache_invalidation (tenant_id, id, resource_type, resource_id,
           page, cache_key, reason, start_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&c.id)
    .bind(&c.resource_type)
    .bind(&c.resource_id)
    .bind(&c.page)
    .bind(&c.cache_key)
    .bind(reason)
    .bind(c.start_time)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub(crate) async fn clear(tx: &mut TxCx, id: &str) -> Result<Option<ClearRow>, ToolError> {
    sqlx::query_as::<_, ClearRow>(
        "SELECT id, resource_type, resource_id, page, cache_key, start_time
         FROM merchandising.cache_invalidation WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}
