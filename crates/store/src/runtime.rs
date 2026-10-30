//! Builds the request context: the store's policies (defaults + the store's overrides) and time
//! zone, cached for a short time, and its clock (the demo clock in demo mode).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use acom_auth::Principal;
use acom_contract::ToolError;
use acom_core::{Clock, Currency, FixedClock, StoreTime, SystemClock, TenantId};
use acom_policy::PolicySet;
use chrono::{DateTime, Utc};

use crate::{Cx, Db, StoreError};

const POLICY_TTL: Duration = Duration::from_secs(30);

/// Shared by every request of a service.
pub struct Runtime {
    pub db: Db,
    defaults: Arc<PolicySet>,
    demo_mode: bool,
    stores: Mutex<HashMap<String, Cached>>,
}

#[derive(Clone)]
struct Cached {
    at: Instant,
    policy: Arc<PolicySet>,
    time: StoreTime,
    currency: Currency,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("demo_mode", &self.demo_mode)
            .finish()
    }
}

impl Runtime {
    pub fn new(db: Db, defaults: PolicySet, demo_mode: bool) -> Self {
        Self {
            db,
            defaults: Arc::new(defaults),
            demo_mode,
            stores: Mutex::new(HashMap::new()),
        }
    }

    pub fn defaults(&self) -> &Arc<PolicySet> {
        &self.defaults
    }

    /// Whether stores run on their demo clock (`ACOM_DEMO_MODE`).
    pub fn demo_mode(&self) -> bool {
        self.demo_mode
    }

    /// The context one request runs with.
    pub async fn context(&self, principal: Principal, request_id: String) -> Result<Cx, ToolError> {
        let tenant = principal.tenant.clone();
        let store = self.store(&tenant).await.map_err(ToolError::from)?;
        let clock = self.clock(&tenant).await.map_err(ToolError::from)?;
        Ok(Cx {
            tenant,
            principal: Arc::new(principal),
            request_id,
            clock,
            time: store.time,
            currency: store.currency,
            policy: store.policy,
            db: self.db.clone(),
        })
    }

    /// Forget cached policies (after a policy override changes).
    pub fn invalidate(&self, tenant: &TenantId) {
        if let Ok(mut cache) = self.stores.lock() {
            cache.remove(tenant.as_str());
        }
    }

    async fn store(&self, tenant: &TenantId) -> Result<Cached, StoreError> {
        if let Ok(cache) = self.stores.lock()
            && let Some(c) = cache.get(tenant.as_str())
            && c.at.elapsed() < POLICY_TTL
        {
            return Ok(c.clone());
        }
        let mut tx = self.db.pool().begin().await?;
        crate::scope_to_tenant(&mut tx, tenant).await?;
        let store: Option<(String, String)> =
            sqlx::query_as("SELECT time_zone, currency FROM platform.online_store WHERE id = $1")
                .bind(tenant.as_str())
                .fetch_optional(&mut *tx)
                .await?;
        let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
            "SELECT file, partial FROM platform.policy_override WHERE tenant_id = $1 ORDER BY file",
        )
        .bind(tenant.as_str())
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let (zone, currency) =
            store.ok_or_else(|| StoreError::Policy(format!("unknown store {tenant}")))?;
        let currency = Currency::parse(currency.trim())
            .map_err(|_| StoreError::Policy(format!("store {tenant}: bad currency {currency}")))?;
        let time =
            StoreTime::new(zone.parse().map_err(|_| {
                StoreError::Policy(format!("store {tenant}: bad time zone {zone}"))
            })?);
        // JSON is YAML, so overrides merge through the same path as the files.
        let texts: Vec<(String, String)> =
            rows.into_iter().map(|(f, p)| (f, p.to_string())).collect();
        let policy = self
            .defaults
            .with_overrides(texts.iter().map(|(f, t)| (f.as_str(), t.as_str())))
            .map_err(|e| StoreError::Policy(e.to_string()))?;
        let c = Cached {
            at: Instant::now(),
            policy: Arc::new(policy),
            time,
            currency,
        };
        if let Ok(mut cache) = self.stores.lock() {
            cache.insert(tenant.as_str().to_owned(), c.clone());
        }
        Ok(c)
    }

    async fn clock(&self, tenant: &TenantId) -> Result<Arc<dyn Clock>, StoreError> {
        if !self.demo_mode {
            return Ok(Arc::new(SystemClock));
        }
        let mut tx = self.db.pool().begin().await?;
        crate::scope_to_tenant(&mut tx, tenant).await?;
        let row: Option<(DateTime<Utc>, bool, DateTime<Utc>)> = sqlx::query_as(
            "SELECT now, is_running, updated_at FROM platform.demo_clock WHERE tenant_id = $1",
        )
        .bind(tenant.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(match row {
            // A running demo clock moves with the wall clock from the moment it was set.
            Some((at, true, set_at)) => Arc::new(FixedClock::at(at + (Utc::now() - set_at))),
            Some((at, false, _)) => Arc::new(FixedClock::at(at)),
            None => Arc::new(SystemClock),
        })
    }
}
