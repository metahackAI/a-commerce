//! `refresh_token` (`platform.auth.refresh`): exchange a refresh token for a new access token and
//! a new refresh token. The old one stops working at once; if it ever comes back, that is reuse
//! and every session of the client ends.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, Group, Kind, Method, OpMeta, Operation,
    Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{REFRESH_EXPIRED, REFRESH_REUSED, REVOKED, open_session};
use crate::api_key::key;
use crate::repo::{db, end_client_sessions, sha256_hex};
use crate::rules::ClientSessionRules;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefreshTokenInput {
    /// The refresh token from login or the last refresh. Never stored.
    #[schemars(length(min = 1, max = 512), example = &"rt_9c1e…", extend("writeOnly" = true))]
    pub refresh_token: String,
}

/// What `refresh_token` runs: the session whose refresh token was shown, not the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshTokenPlan {
    pub session_id: String,
    pub client_id: String,
    pub scopes: Vec<String>,
}

/// Exchange a refresh token.
pub struct RefreshToken;

impl Operation for RefreshToken {
    const META: OpMeta = OpMeta {
        address: "platform.auth.refresh",
        tool: "refresh_token",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/platform/auth/refresh",
        summary: "Exchange an API client's refresh token for a new access token and refresh token, with the same store and scopes. The old refresh token stops working.",
        consumes: &[Slot::one("refresh_token")],
        produces: &[Slot::one("access_token")],
        checks: &[],
        errors: &[REFRESH_EXPIRED, REFRESH_REUSED, REVOKED],
        audience: Audience::Shopper,
        action_type: ActionType::AUTHORIZE,
        person_scope: "",
    };
    type Input = RefreshTokenInput;
    type Output = Value;
}

#[derive(Debug, sqlx::FromRow)]
struct Held {
    id: String,
    subject_id: Option<String>,
    scopes: Option<Vec<String>>,
    revoked_at: Option<DateTime<Utc>>,
    replaced_by_id: Option<String>,
    refresh_expires_at: Option<DateTime<Utc>>,
}

fn revoked() -> ToolError {
    ToolError::new(
        REVOKED,
        "This refresh token is no longer valid; sign in again.",
    )
    .detail(json!({"reason": "revoked"}))
    .next("login")
}

fn reused() -> ToolError {
    ToolError::new(
        REFRESH_REUSED,
        "This refresh token was already exchanged, so every session of this client has ended; sign in again.",
    )
    .detail(json!({"reason": "refresh_reused"}))
    .next("login")
}

/// The session a refresh token belongs to, whatever its state.
async fn holder(tx: &mut TxCx, token_hash: &str, lock: bool) -> Result<Option<Held>, ToolError> {
    let sql = format!(
        "SELECT id, subject_id, scopes, revoked_at, replaced_by_id, refresh_expires_at
         FROM platform.session
         WHERE tenant_id = $1 AND refresh_token_hash = $2 AND actor_kind = 'external_agent'{}",
        if lock { " FOR UPDATE" } else { "" }
    );
    sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(tx.tenant.as_str())
        .bind(token_hash)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)
}

impl WriteOp<Cx> for RefreshToken {
    type Plan = RefreshTokenPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RefreshTokenInput,
    ) -> Result<Planned<RefreshTokenPlan>, ToolError> {
        let rules = ClientSessionRules::from_policy(&cx.policy)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let held = holder(&mut tx, &sha256_hex(input.refresh_token.trim()), false).await?;
        let k = match held.as_ref().and_then(|h| h.subject_id.as_deref()) {
            Some(client) => key(&mut tx, client).await?,
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;
        let Some(h) = held else {
            return Err(revoked());
        };
        let client = h.subject_id.clone().unwrap_or_default();
        if h.replaced_by_id.is_some() {
            // Reuse: someone holds a token that was already exchanged. Every session of the
            // client ends now, before the error goes back.
            let mut w = cx.begin().await.map_err(ToolError::from)?;
            end_client_sessions(&mut w, &client, now).await?;
            w.commit().await.map_err(ToolError::from)?;
            return Err(reused());
        }
        if h.revoked_at.is_some() {
            return Err(revoked());
        }
        if h.refresh_expires_at.is_none_or(|e| e <= now) {
            return Err(ToolError::new(
                REFRESH_EXPIRED,
                "This refresh token has expired; sign in again.",
            )
            .detail(json!({"reason": "refresh_expired"}))
            .next("login"));
        }
        let key_ok =
            k.is_some_and(|k| k.api_key_status == "active" && k.expires.is_none_or(|e| e > now));
        if !key_ok {
            return Err(revoked());
        }
        let scopes = h.scopes.clone().unwrap_or_default();
        Ok(Planned {
            changes: vec![
                Change {
                    op: ChangeOp::Create,
                    entity: "session".into(),
                    id: String::new(),
                    from: Value::Null,
                    to: json!({"client_id": client, "scopes": scopes,
                               "expires_in": rules.access_token_minutes * 60}),
                    note: None,
                },
                Change {
                    op: ChangeOp::Revoke,
                    entity: "session".into(),
                    id: h.id.clone(),
                    from: Value::Null,
                    to: Value::Null,
                    note: Some("Its refresh token is exchanged and stops working".into()),
                },
            ],
            checks: Vec::new(),
            basis: Vec::new(),
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan: RefreshTokenPlan {
                session_id: h.id,
                client_id: client,
                scopes,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &RefreshTokenPlan) -> Result<Committed, ToolError> {
        let rules = ClientSessionRules::from_policy(&tx.policy)?;
        let now = tx.clock.now();
        let state: Option<(Option<DateTime<Utc>>, Option<String>)> = sqlx::query_as(
            "SELECT revoked_at, replaced_by_id FROM platform.session
             WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.session_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        match state {
            Some((_, Some(_))) => return Err(reused()),
            Some((Some(_), None)) | None => return Err(revoked()),
            Some((None, None)) => {}
        }
        let opened = open_session(tx, &plan.client_id, &plan.scopes, &rules).await?;
        sqlx::query(
            "UPDATE platform.session SET revoked_at = $3, replaced_by_id = $4, updated_at = now(),
               version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.session_id)
        .bind(now)
        .bind(&opened.session_id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        Ok(Committed {
            id: opened.session_id,
            produced: opened.produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RefreshTokenPlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let replaced: Option<Option<String>> = sqlx::query_scalar(
            "SELECT replaced_by_id FROM platform.session WHERE tenant_id = $1 AND id = $2",
        )
        .bind(cx.tenant.as_str())
        .bind(&plan.session_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = replaced.flatten().as_deref() == Some(done.id.as_str());
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} was not replaced by {}",
                    plan.session_id, done.id
                )]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        _plan: &RefreshTokenPlan,
        done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            acom_contract::ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "Tokens already handed out cannot be taken back; end {} with logout.",
                done.id
            ),
        )
        .next("logout"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::closed_schema_for;

    #[test]
    fn the_refresh_token_is_a_write_only_input() {
        let s = closed_schema_for::<RefreshTokenInput>();
        assert_eq!(s["properties"]["refresh_token"]["writeOnly"], true);
        assert_eq!(s["required"], json!(["refresh_token"]));
    }
}
