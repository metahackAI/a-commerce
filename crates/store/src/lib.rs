//! Persistence for the commerce service.
//!
//! Owns the request context (`Cx`: store, principal, clock, policy snapshot, pool) and the
//! transaction context (`TxCx`) that the operation traits in `acom-contract` run against.
//! Every transaction sets `app.tenant_id`, so row-level security keeps stores apart even if a
//! query forgets a `tenant_id` filter. The schema is `migrations/*.sql`, embedded in the binary.
//! Design: `docs/backend/commerce.md` §5, `docs/backend/database.md`.

pub mod page;
pub mod runtime;

use std::sync::Arc;

use acom_auth::Principal;
use acom_contract::{Ctx, ErrorCode, ToolError};
use acom_core::{Clock, StoreTime, TenantId};
use acom_policy::PolicySet;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::{Postgres, Transaction};

/// The migrations in `migrations/`, embedded at compile time.
pub use page::Page;
pub use runtime::Runtime;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("policy error: {0}")]
    Policy(String),
}

impl From<StoreError> for ToolError {
    fn from(e: StoreError) -> Self {
        // Connection problems are retryable; anything else is our bug. Details stay in logs.
        let code = match &e {
            StoreError::Database(sqlx::Error::PoolTimedOut | sqlx::Error::Io(_)) => {
                ErrorCode::UPSTREAM_UNAVAILABLE
            }
            _ => ErrorCode::INTERNAL,
        };
        tracing::error!(error = %e, "store error");
        ToolError::new(code, "The store's database could not complete the request.")
    }
}

/// A connection pool to the commerce database.
#[derive(Debug, Clone)]
pub struct Db {
    pool: PgPool,
}

impl Db {
    pub async fn connect(url: &str, max_connections: u32) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    /// A pool that connects on first use, so a service can start (and report "not ready")
    /// while the database is still coming up.
    pub fn connect_lazy(url: &str, max_connections: u32) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(std::time::Duration::from_secs(3))
            .connect_lazy(url)?;
        Ok(Self { pool })
    }

    /// Applies pending migrations (`acom-commerce migrate`, run as the owner role). sqlx holds
    /// an advisory lock while it runs, so two runners at once are safe.
    pub async fn migrate(&self) -> Result<(), StoreError> {
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }

    /// `Ok` when the database answers and every embedded migration has been applied.
    /// Services use this as their readiness check.
    pub async fn migrations_applied(&self) -> Result<(), String> {
        let expected = MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0);
        let applied: Option<i64> =
            sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
                .fetch_one(&self.pool)
                .await
                .map_err(|e| match e {
                    sqlx::Error::Database(d) if d.code().as_deref() == Some("42P01") => {
                        "migrations have not run yet".to_owned()
                    }
                    other => format!("database unreachable: {other}"),
                })?;
        match applied {
            Some(v) if v >= expected => Ok(()),
            v => Err(format!(
                "migrations pending: applied {}, expected {expected}",
                v.unwrap_or(0)
            )),
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

/// What one request runs with.
#[derive(Debug, Clone)]
pub struct Cx {
    pub tenant: TenantId,
    pub principal: Arc<Principal>,
    pub request_id: String,
    pub clock: Arc<dyn Clock>,
    /// The store's time zone: inputs are read and outputs written in it.
    pub time: StoreTime,
    pub policy: Arc<PolicySet>,
    pub db: Db,
}

impl Cx {
    /// Opens a transaction scoped to this request's store.
    pub async fn begin(&self) -> Result<TxCx, StoreError> {
        let mut tx = self.db.pool.begin().await?;
        scope_to_tenant(&mut tx, &self.tenant).await?;
        Ok(TxCx {
            tx,
            tenant: self.tenant.clone(),
            principal: self.principal.clone(),
            request_id: self.request_id.clone(),
            clock: self.clock.clone(),
            time: self.time,
            policy: self.policy.clone(),
        })
    }

    /// The same request run by the system: the change-set runner after a person approved.
    pub fn as_system(&self, subject: &str) -> Cx {
        Cx {
            principal: Arc::new(Principal::system(self.tenant.clone(), subject)),
            ..self.clone()
        }
    }

    /// Runs read queries inside a read-only transaction scoped to the store.
    pub async fn read(&self) -> Result<TxCx, StoreError> {
        let mut t = self.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *t.tx)
            .await?;
        Ok(t)
    }
}

/// An open transaction scoped to one store.
#[derive(Debug)]
pub struct TxCx {
    pub tx: Transaction<'static, Postgres>,
    pub tenant: TenantId,
    pub principal: Arc<Principal>,
    pub request_id: String,
    pub clock: Arc<dyn Clock>,
    pub time: StoreTime,
    pub policy: Arc<PolicySet>,
}

impl TxCx {
    pub async fn commit(self) -> Result<(), StoreError> {
        self.tx.commit().await?;
        Ok(())
    }
}

impl Ctx for Cx {
    type Tx = TxCx;
}

/// `set_config('app.tenant_id', …, true)`: local to the transaction, read by every RLS policy.
/// Every transaction on store data starts with this.
pub async fn scope_to_tenant(
    tx: &mut Transaction<'static, Postgres>,
    tenant: &TenantId,
) -> Result<(), StoreError> {
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant.as_str())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_the_migrations_in_order() {
        let versions: Vec<i64> = MIGRATOR.iter().map(|m| m.version).collect();
        assert!(!versions.is_empty());
        assert!(versions.windows(2).all(|w| w[0] < w[1]), "{versions:?}");
        assert_eq!(versions[0], 1);
    }

    /// Integration test: needs a database prepared by `infra/postgres/init.sql`, with
    /// `MIGRATION_DATABASE_URL` (owner role) and `DATABASE_URL` (service role) as in `.env.example`.
    /// Run on request with `cargo test -p acom-store -- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn migrations_apply_and_rls_isolates_stores() {
        let owner = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
        Db::connect(&owner, 1)
            .await
            .expect("connect as owner")
            .migrate()
            .await
            .expect("migrate");
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let db = Db::connect(&url, 2).await.expect("connect");
        db.migrations_applied()
            .await
            .expect("service role sees the applied migrations");
        let mut tx = db.pool().begin().await.expect("begin");
        let a = TenantId::parse("store_a").expect("id");
        scope_to_tenant(&mut tx, &a).await.expect("scope");
        let visible: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM catalog.product_group WHERE tenant_id <> 'store_a'",
        )
        .fetch_one(&mut *tx)
        .await
        .expect("query");
        assert_eq!(visible, 0);
    }
}
