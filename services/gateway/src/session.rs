//! Server sessions (`platform.session`) and the principal each one stands for.
//!
//! The browser holds two cookies: `acom_session` (httpOnly; the session token) and `acom_csrf`
//! (readable by the page; sent back as `X-CSRF-Token` on every state-changing request). The
//! database keeps only SHA-256 hashes of both. Lookups are cached for a short time so a page
//! with many API calls does not hit the database for each one.

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use acom_auth::{Principal, Surface, scopes};
use acom_core::TenantId;
use acom_core::enums::ActorKind;
use acom_store::{Db, StoreError};
use chrono::{DateTime, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};

pub const SESSION_COOKIE: &str = "acom_session";
pub const CSRF_COOKIE: &str = "acom_csrf";
const CACHE_TTL: Duration = Duration::from_secs(30);

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A cookie value from the `Cookie` header.
pub fn cookie<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(axum::http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

/// A live session and who it is.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub csrf_hash: String,
    pub principal: Principal,
    pub expires_at: DateTime<Utc>,
}

/// A new session's tokens, to set as cookies.
#[derive(Debug, Clone)]
pub struct Issued {
    pub session: Session,
    pub token: String,
    pub csrf: String,
}

#[derive(Debug)]
pub struct Sessions {
    pub db: Db,
    pub tenant: TenantId,
    pub ttl: chrono::Duration,
    pub demo_mode: bool,
    cache: Mutex<HashMap<String, (Instant, Session)>>,
}

impl Sessions {
    pub fn new(db: Db, tenant: TenantId, ttl: chrono::Duration, demo_mode: bool) -> Self {
        Self {
            db,
            tenant,
            ttl,
            demo_mode,
            cache: Mutex::new(HashMap::new()),
        }
    }

    async fn tx(&self) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, StoreError> {
        let mut tx = self.db.pool().begin().await?;
        acom_store::scope_to_tenant(&mut tx, &self.tenant).await?;
        Ok(tx)
    }

    /// The scopes an operator's roles grant, plus demo controls for platform admins in demo mode.
    async fn operator(&self, admin_id: &str) -> Result<Option<Principal>, StoreError> {
        let mut tx = self.tx().await?;
        let row: Option<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT name, alternate_name, admin_status FROM platform.admin_user
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(self.tenant.as_str())
        .bind(admin_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((name, alternate, status)) = row else {
            return Ok(None);
        };
        if status != "active" {
            return Ok(None);
        }
        let granted: Vec<(String, String)> = sqlx::query_as(
            "SELECT ur.role_id, rs.scope FROM platform.admin_user_role ur
             JOIN platform.role_scope rs ON rs.tenant_id = ur.tenant_id AND rs.role_id = ur.role_id
             WHERE ur.tenant_id = $1 AND ur.admin_user_id = $2",
        )
        .bind(self.tenant.as_str())
        .bind(admin_id)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut set: BTreeSet<String> = granted.iter().map(|(_, s)| s.clone()).collect();
        // Operators may draft change sets through the agent; the agent token keeps this scope.
        set.insert("changeset:draft".into());
        if self.demo_mode && granted.iter().any(|(r, _)| r == "rol_platform_admin") {
            set.insert("admin:demo".into());
        }
        Ok(Some(Principal {
            subject: admin_id.to_owned(),
            kind: ActorKind::Operator,
            tenant: self.tenant.clone(),
            surface: Surface::Bo,
            scopes: set,
            name: Some(alternate.unwrap_or(name)),
            session_id: None,
        }))
    }

    async fn shopper(&self, customer_id: &str) -> Result<Option<Principal>, StoreError> {
        let mut tx = self.tx().await?;
        let row: Option<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT name, given_name, member_state FROM customer.person
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(self.tenant.as_str())
        .bind(customer_id)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row
            .filter(|(_, _, state)| state == "active" || state == "dormant")
            .map(|(name, given, _)| Principal {
                subject: customer_id.to_owned(),
                kind: ActorKind::Shopper,
                tenant: self.tenant.clone(),
                surface: Surface::Fo,
                scopes: scopes::shopper(),
                name: Some(given.unwrap_or(name)),
                session_id: None,
            }))
    }

    fn guest(&self, session_id: &str) -> Principal {
        Principal {
            subject: session_id.to_owned(),
            kind: ActorKind::Shopper,
            tenant: self.tenant.clone(),
            surface: Surface::Fo,
            scopes: scopes::guest(),
            name: None,
            session_id: Some(session_id.to_owned()),
        }
    }

    /// Who a stored session is (`None` when the account is gone or no longer active).
    async fn principal(
        &self,
        id: &str,
        kind: &str,
        subject: Option<&str>,
    ) -> Result<Option<Principal>, StoreError> {
        let p = match (kind, subject) {
            ("operator", Some(s)) => self.operator(s).await?,
            ("shopper", Some(s)) if s.starts_with("cus_") => self.shopper(s).await?,
            ("shopper", _) => Some(self.guest(id)),
            _ => None,
        };
        Ok(p.map(|mut p| {
            p.session_id = Some(id.to_owned());
            p
        }))
    }

    /// The session behind a cookie token, if it is live.
    pub async fn find(&self, token: &str) -> Result<Option<Session>, StoreError> {
        let key = hash(token);
        if let Ok(cache) = self.cache.lock()
            && let Some((at, s)) = cache.get(&key)
            && at.elapsed() < CACHE_TTL
        {
            return Ok((s.expires_at > Utc::now()).then(|| s.clone()));
        }
        let mut tx = self.tx().await?;
        #[allow(clippy::type_complexity)]
        let row: Option<(String, String, Option<String>, String, DateTime<Utc>)> = sqlx::query_as(
            "UPDATE platform.session SET last_seen_at = now()
             WHERE tenant_id = $1 AND token_hash = $2 AND revoked_at IS NULL AND expires_at > now()
             RETURNING id, actor_kind, subject_id, csrf_token_hash, expires_at",
        )
        .bind(self.tenant.as_str())
        .bind(&key)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let Some((id, kind, subject, csrf_hash, expires_at)) = row else {
            return Ok(None);
        };
        let Some(principal) = self.principal(&id, &kind, subject.as_deref()).await? else {
            return Ok(None);
        };
        let s = Session {
            id,
            csrf_hash,
            principal,
            expires_at,
        };
        if let Ok(mut cache) = self.cache.lock() {
            cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
            cache.insert(key, (Instant::now(), s.clone()));
        }
        Ok(Some(s))
    }

    /// Opens a session for an operator, a shopper, or (with no subject) a guest.
    pub async fn open(
        &self,
        kind: ActorKind,
        subject: Option<&str>,
    ) -> Result<Option<Issued>, StoreError> {
        let id = format!("ses_{}", acom_core::id::new_suffix());
        let Some(principal) = self.principal(&id, kind.as_str(), subject).await? else {
            return Ok(None);
        };
        let token = random_token();
        let csrf = random_token();
        let expires_at = Utc::now() + self.ttl;
        let mut tx = self.tx().await?;
        sqlx::query(
            "INSERT INTO platform.session (tenant_id, id, actor_kind, subject_id, token_hash,
               csrf_token_hash, expires_at, last_seen_at) VALUES ($1,$2,$3,$4,$5,$6,$7, now())",
        )
        .bind(self.tenant.as_str())
        .bind(&id)
        .bind(kind.as_str())
        .bind(subject)
        .bind(hash(&token))
        .bind(hash(&csrf))
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        if kind == ActorKind::Operator
            && let Some(s) = subject
        {
            sqlx::query(
                "UPDATE platform.admin_user SET last_login_at = now() WHERE tenant_id = $1 AND id = $2",
            )
            .bind(self.tenant.as_str())
            .bind(s)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(Some(Issued {
            session: Session {
                id,
                csrf_hash: hash(&csrf),
                principal,
                expires_at,
            },
            token,
            csrf,
        }))
    }

    /// Ends a session (logout).
    pub async fn revoke(&self, token: &str) -> Result<(), StoreError> {
        let key = hash(token);
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(&key);
        }
        let mut tx = self.tx().await?;
        sqlx::query(
            "UPDATE platform.session SET revoked_at = now(), updated_at = now()
             WHERE tenant_id = $1 AND token_hash = $2 AND revoked_at IS NULL",
        )
        .bind(self.tenant.as_str())
        .bind(key)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// The operator whose login id and password match, if any. Unknown ids and wrong
    /// passwords look the same to the caller.
    pub async fn check_password(
        &self,
        login_id: &str,
        password: &str,
        decoy: &str,
    ) -> Result<Login, StoreError> {
        let mut tx = self.tx().await?;
        let row: Option<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, password_hash, admin_status FROM platform.admin_user
             WHERE tenant_id = $1 AND login_id = $2",
        )
        .bind(self.tenant.as_str())
        .bind(login_id.trim().to_lowercase())
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        let Some((id, hash, status)) = row else {
            // Same work as a real check, so timing does not reveal which ids exist.
            acom_auth::password::verify(password, decoy);
            return Ok(Login::Rejected);
        };
        let ok = hash
            .as_deref()
            .is_some_and(|h| acom_auth::password::verify(password, h));
        Ok(match (ok, status.as_str()) {
            (false, _) => Login::Rejected,
            (true, "active") => Login::Operator(id),
            (true, other) => Login::Blocked(other.to_owned()),
        })
    }
}

impl Sessions {
    /// The demo personas `demo-login` offers, with a line about each, in the order configured.
    pub async fn describe(&self, ids: &[String]) -> Result<Vec<serde_json::Value>, StoreError> {
        let mut tx = self.tx().await?;
        let admins: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id, name, job_title, login_id FROM platform.admin_user
             WHERE tenant_id = $1 AND id = ANY($2) AND admin_status = 'active'",
        )
        .bind(self.tenant.as_str())
        .bind(ids)
        .fetch_all(&mut *tx)
        .await?;
        let shoppers: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT p.id, coalesce(p.given_name, p.name),
                    (SELECT m.tier_id FROM customer.program_membership m
                      WHERE m.tenant_id = p.tenant_id AND m.customer_id = p.id LIMIT 1)
             FROM customer.person p WHERE p.tenant_id = $1 AND p.id = ANY($2)",
        )
        .bind(self.tenant.as_str())
        .bind(ids)
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut out = Vec::new();
        for id in ids {
            if let Some((id, name, title, login)) = admins.iter().find(|a| &a.0 == id) {
                out.push(
                    serde_json::json!({"persona": id, "kind": "operator", "name": name,
                    "description": title, "login_id": login}),
                );
            } else if let Some((id, name, tier)) = shoppers.iter().find(|s| &s.0 == id) {
                out.push(
                    serde_json::json!({"persona": id, "kind": "shopper", "name": name,
                    "description": tier.as_ref().map(|t| format!("{t} member"))}),
                );
            } else if id == "guest" {
                out.push(
                    serde_json::json!({"persona": "guest", "kind": "guest", "name": "Guest",
                    "description": "Not signed in"}),
                );
            }
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Login {
    Operator(String),
    Rejected,
    /// The password matched but the account is `suspended` or `locked`.
    Blocked(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cookies_and_hashes_tokens() {
        let mut h = axum::http::HeaderMap::new();
        h.insert(
            axum::http::header::COOKIE,
            "a=1; acom_session=abc; acom_csrf=def".parse().unwrap(),
        );
        assert_eq!(cookie(&h, SESSION_COOKIE), Some("abc"));
        assert_eq!(cookie(&h, CSRF_COOKIE), Some("def"));
        assert_eq!(cookie(&h, "missing"), None);
        let t = random_token();
        assert_eq!(t.len(), 64);
        assert_ne!(t, random_token());
        assert_eq!(hash("x"), hash("x"));
        assert_ne!(hash("x"), hash("y"));
    }
}
