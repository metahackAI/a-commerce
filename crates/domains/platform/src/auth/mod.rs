//! The `auth` resource: sessions of outside agents and integration clients that sign in with an
//! API key (`login`, `refresh_token`, `logout`). People's browser sessions are the gateway's
//! (`/api/auth/*`, docs/backend/gateway.md §6) and these tools never touch them: a client session
//! is a `platform.session` row with `actor_kind = 'external_agent'` and the key as its subject,
//! which the gateway's cookie lookup never takes for a person.
//!
//! Secrets stay out of storage: the client secret and the refresh token are write-only inputs
//! (never stored with the preview), tokens are kept as SHA-256 hashes, and the tokens a run hands
//! out are shown once (`acom_changeset::SHOWN_ONCE`).

pub mod login;
pub mod logout;
pub mod refresh_token;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_core::enums::ActorKind;
use acom_store::TxCx;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Map, Value, json};

use crate::repo::{db, new_secret, sha256_hex};
use crate::rules::ClientSessionRules;

/// The client id or secret is wrong (an unknown client looks the same).
pub const INVALID_CREDENTIALS: ErrorCode =
    ErrorCode::new("invalid_credentials", ErrorClass::Unauthenticated);
/// Too many wrong secrets: the key is locked for a while.
pub const ACCOUNT_LOCKED: ErrorCode = ErrorCode::new("account_locked", ErrorClass::Locked);
/// The refresh token is past its lifetime.
pub const REFRESH_EXPIRED: ErrorCode =
    ErrorCode::new("refresh_expired", ErrorClass::Unauthenticated);
/// A refresh token came back after it was exchanged: every session of the client ends.
pub const REFRESH_REUSED: ErrorCode = ErrorCode::new("refresh_reused", ErrorClass::Unauthenticated);
/// The session was signed out, or its key was revoked.
pub const REVOKED: ErrorCode = ErrorCode::new("revoked", ErrorClass::Unauthenticated);

/// What a client session's tokens are for, as the claims the gateway will put in its tokens.
pub(crate) fn claims(cx_tenant: &str, client_id: &str, scopes: &[String]) -> Value {
    json!({"sub": client_id, "tenant": cx_tenant, "kind": ActorKind::ExternalAgent,
           "scopes": scopes})
}

/// A new session for a client: tokens made here, kept as hashes, handed back once.
pub(crate) struct Opened {
    pub session_id: String,
    pub produced: Map<String, Value>,
}

/// Opens a client session (`ses_…`) in the transaction and says what the run answers: the token
/// type, lifetime, claims, and the tokens themselves under `shown_once`.
pub(crate) async fn open_session(
    tx: &mut TxCx,
    client_id: &str,
    scopes: &[String],
    rules: &ClientSessionRules,
) -> Result<Opened, ToolError> {
    let now = tx.clock.now();
    let session_id = format!("ses_{}", acom_core::id::new_suffix());
    let access = new_secret("at_");
    let refresh = new_secret("rt_");
    // Client sessions have no browser, so no CSRF token; the column keeps a hash of a value
    // nobody holds.
    let unused_csrf = new_secret("");
    let expires = now + Duration::minutes(rules.access_token_minutes);
    let refresh_expires = now + Duration::days(rules.refresh_token_days);
    sqlx::query(
        "INSERT INTO platform.session (tenant_id, id, actor_kind, subject_id, token_hash,
           csrf_token_hash, expires_at, last_seen_at, scopes, refresh_token_hash,
           refresh_expires_at)
         VALUES ($1, $2, 'external_agent', $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(tx.tenant.as_str())
    .bind(&session_id)
    .bind(client_id)
    .bind(sha256_hex(&access))
    .bind(sha256_hex(&unused_csrf))
    .bind(expires)
    .bind(now)
    .bind(scopes)
    .bind(sha256_hex(&refresh))
    .bind(refresh_expires)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    let mut produced = Map::new();
    produced.insert("token_type".into(), json!("Bearer"));
    produced.insert("expires_in".into(), json!(rules.access_token_minutes * 60));
    produced.insert(
        "claims".into(),
        claims(tx.tenant.as_str(), client_id, scopes),
    );
    produced.insert(
        acom_changeset::SHOWN_ONCE.into(),
        json!({"access_token": access, "refresh_token": refresh}),
    );
    Ok(Opened {
        session_id,
        produced,
    })
}

/// The client's live sessions, oldest first.
pub(crate) async fn live_sessions(
    tx: &mut TxCx,
    client_id: &str,
) -> Result<Vec<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM platform.session
         WHERE tenant_id = $1 AND subject_id = $2 AND actor_kind = 'external_agent'
           AND revoked_at IS NULL AND refresh_expires_at > $3
         ORDER BY created_at, id",
    )
    .bind(tx.tenant.as_str())
    .bind(client_id)
    .bind(tx.clock.now())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The oldest live sessions that must end so one more fits under
/// `client_sessions.max_sessions_per_client`.
pub(crate) fn over_the_cap(live: &[String], rules: &ClientSessionRules) -> Vec<String> {
    let cap = usize::try_from(rules.max_sessions_per_client.max(1)).unwrap_or(1);
    let excess = (live.len() + 1).saturating_sub(cap);
    live.iter().take(excess).cloned().collect()
}

/// Ends the given sessions.
pub(crate) async fn end_sessions(
    tx: &mut TxCx,
    ids: &[String],
    at: DateTime<Utc>,
) -> Result<(), ToolError> {
    if ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE platform.session SET revoked_at = $3, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = ANY($2) AND revoked_at IS NULL",
    )
    .bind(tx.tenant.as_str())
    .bind(ids)
    .bind(at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_session_ends_the_oldest_beyond_the_cap() {
        let rules = ClientSessionRules::from_policy(&crate::rules::tests::defaults()).unwrap();
        let cap = usize::try_from(rules.max_sessions_per_client).unwrap();
        let live: Vec<String> = (0..cap).map(|i| format!("ses_{i}")).collect();
        assert_eq!(over_the_cap(&live, &rules), vec!["ses_0".to_owned()]);
        assert!(over_the_cap(&live[..cap - 1], &rules).is_empty());
    }

    const CLIENT: &str = "key_test_client";
    const SECRET: &str = "ak_live_test_secret_for_the_auth_tools";
    const REQUEST: &str = "req_test_platform_auth";

    async fn clean() {
        for sql in [
            "DELETE FROM platform.action_log WHERE request_id = $1",
            "DELETE FROM platform.preview WHERE request_id = $1",
        ] {
            crate::testdb::owner_exec(sql, &[REQUEST]).await;
        }
        crate::testdb::owner_exec(
            "UPDATE platform.session SET replaced_by_id = NULL WHERE subject_id = $1",
            &[CLIENT],
        )
        .await;
        for sql in [
            "DELETE FROM platform.session WHERE subject_id = $1",
            "DELETE FROM platform.api_key WHERE id = $1",
        ] {
            crate::testdb::owner_exec(sql, &[CLIENT]).await;
        }
    }

    /// The whole life of a client session through the preview store: login, refresh, a reused
    /// refresh token, logout, and the lockout after wrong secrets. No secret is ever stored.
    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database and MIGRATION_DATABASE_URL"]
    async fn client_sessions_keep_no_secret_and_rotate_their_refresh_tokens() {
        clean().await;
        crate::testdb::owner_exec(
            "INSERT INTO platform.api_key (tenant_id, id, name, key_prefix, secret_hash, scopes)
             VALUES ('store_northwind', $1, 'Test client', 'ak_live_te', $2,
                     '{external:read,external:cart}')",
            &[CLIENT, &crate::repo::sha256_hex(SECRET)],
        )
        .await;
        let registry = crate::testdb::registry();
        let mut cx = crate::testdb::cx_with(
            CLIENT,
            ActorKind::ExternalAgent,
            ["tools:write:platform".to_owned()].into(),
        )
        .await;
        cx.request_id = REQUEST.to_owned();
        let login = |secret: &str| {
            json!({"grant_type": "client_credentials", "client_id": CLIENT,
                   "client_secret_ref": secret, "requested_scopes": ["external:read", "tools:write:sales"]})
        };

        // login: the preview keeps no secret; the run answers the tokens once.
        let p = acom_changeset::preview(&registry, &cx, "login", login(SECRET), None)
            .await
            .unwrap();
        assert!(!p.needs_approval);
        let mut tx = cx.read().await.unwrap();
        let (input, input_hash): (Value, String) = sqlx::query_as(
            "SELECT input, input_hash FROM platform.preview WHERE tenant_id = $1 AND id = $2",
        )
        .bind("store_northwind")
        .bind(&p.preview_id)
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(input["client_secret_ref"], acom_changeset::NOT_STORED);
        assert!(!input.to_string().contains(SECRET) && !input_hash.contains(SECRET));
        let run = acom_changeset::commit(&registry, &cx, "login", &p.preview_id)
            .await
            .unwrap();
        let out = json!(run);
        assert_eq!(out["claims"]["scopes"], json!(["external:read"]));
        assert_eq!(out["token_type"], "Bearer");
        let refresh = out["refresh_token"].as_str().unwrap().to_owned();
        let access = out["access_token"].as_str().unwrap().to_owned();
        let first_session = run.id.clone();
        let again = acom_changeset::commit(&registry, &cx, "login", &p.preview_id)
            .await
            .unwrap();
        assert!(
            json!(again).get("access_token").is_none(),
            "shown once only"
        );
        let mut tx = cx.read().await.unwrap();
        let kept: Vec<String> = sqlx::query_scalar(
            "SELECT coalesce(p.result::text, '') || coalesce(a.result::text, '')
             FROM platform.preview p JOIN platform.action_log a
               ON a.tenant_id = p.tenant_id AND a.preview_id = p.id
             WHERE p.tenant_id = $1 AND p.id = $2",
        )
        .bind("store_northwind")
        .bind(&p.preview_id)
        .fetch_all(&mut *tx.tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert!(!kept.is_empty());
        assert!(
            kept.iter()
                .all(|t| !t.contains(&refresh) && !t.contains(&access))
        );

        // refresh_token: a new pair; the old refresh token is spent.
        let r = acom_changeset::preview(
            &registry,
            &cx,
            "refresh_token",
            json!({"refresh_token": refresh}),
            None,
        )
        .await
        .unwrap();
        let rotated = acom_changeset::commit(&registry, &cx, "refresh_token", &r.preview_id)
            .await
            .unwrap();
        assert_ne!(rotated.id, first_session);
        assert!(json!(rotated)["refresh_token"].is_string());

        // The spent token comes back: reuse ends every session of the client.
        let e = acom_changeset::preview(
            &registry,
            &cx,
            "refresh_token",
            json!({"refresh_token": refresh}),
            None,
        )
        .await
        .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("refresh_reused", Some("login"))
        );
        let mut tx = cx.read().await.unwrap();
        assert!(live_sessions(&mut tx, CLIENT).await.unwrap().is_empty());
        tx.commit().await.unwrap();

        // logout: a fresh session ends; a second time there is nothing left to end.
        let p = acom_changeset::preview(&registry, &cx, "login", login(SECRET), None)
            .await
            .unwrap();
        let fresh = acom_changeset::commit(&registry, &cx, "login", &p.preview_id)
            .await
            .unwrap();
        let mut me = (*cx.principal).clone();
        me.session_id = Some(fresh.id.clone());
        let mut signed_in = cx.clone();
        signed_in.principal = std::sync::Arc::new(me);
        let out = acom_changeset::preview(&registry, &signed_in, "logout", json!({}), None)
            .await
            .unwrap();
        acom_changeset::commit(&registry, &signed_in, "logout", &out.preview_id)
            .await
            .unwrap();
        let e = acom_changeset::preview(&registry, &signed_in, "logout", json!({}), None)
            .await
            .unwrap_err();
        assert_eq!(e.code, "already_revoked");

        // Wrong secrets lock the key, as wrong passwords lock an account.
        let rules = ClientSessionRules::from_policy(&cx.policy).unwrap();
        let mut last = None;
        for _ in 0..rules.lockout.failed_attempts {
            last = Some(
                acom_changeset::preview(&registry, &cx, "login", login("ak_live_wrong"), None)
                    .await
                    .unwrap_err(),
            );
        }
        assert_eq!(last.unwrap().code, "account_locked");
        let e = acom_changeset::preview(&registry, &cx, "login", login(SECRET), None)
            .await
            .unwrap_err();
        assert_eq!(e.code, "account_locked");
        clean().await;
    }
}
