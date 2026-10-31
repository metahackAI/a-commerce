//! `login` (`platform.auth.login`): an outside agent or integration client signs in with its API
//! key and gets an access token and a refresh token. People sign in at the gateway instead.
//!
//! The preview checks the credentials and shows the session it would open; the run opens it and
//! answers the tokens once. Wrong secrets count toward the lockout of `customer.yml`
//! `password.lockout` even though a preview changes nothing else: that count is the guard
//! against guessing, recorded as the attempt happens.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, Group, Kind, Method, OpMeta, Operation,
    Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::ActorKind;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    ACCOUNT_LOCKED, INVALID_CREDENTIALS, end_sessions, live_sessions, open_session, over_the_cap,
};
use crate::api_key::{KeyRow, SCOPE_NOT_GRANTED, key};
use crate::repo::{db, sha256_hex};
use crate::rules::ClientSessionRules;

/// How the client proves who it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GrantType {
    /// An outside agent's OAuth authorization code (`code`).
    OauthCode,
    /// An integration client's API key secret (`client_secret_ref`).
    ClientCredentials,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginInput {
    /// How the client proves who it is. oauth_code: needs `code`; client_credentials: needs
    /// `client_secret_ref`.
    #[schemars(example = &"client_credentials")]
    pub grant_type: GrantType,
    /// The client: the API key's id.
    #[schemars(regex(pattern = r"^key_[a-z0-9_]+$"), example = &"key_31")]
    pub client_id: String,
    /// The OAuth authorization code (oauth_code). Never stored.
    #[schemars(length(min = 1, max = 512), example = &"c_7Q2xk", extend("writeOnly" = true))]
    pub code: Option<String>,
    /// The API key's secret (client_credentials). Never stored.
    #[schemars(length(min = 1, max = 512), example = &"ak_live_3f…", extend("writeOnly" = true))]
    pub client_secret_ref: Option<String>,
    /// The scopes the session should have; it gets those the key also has. All of the key's
    /// scopes when left out.
    #[schemars(example = ["external:read", "external:cart"])]
    pub requested_scopes: Option<Vec<String>>,
}

/// What `login` runs: no secret, only who signs in with which scopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginPlan {
    pub client_id: String,
    pub scopes: Vec<String>,
}

/// Sign an API client in.
pub struct Login;

impl Operation for Login {
    const META: OpMeta = OpMeta {
        address: "platform.auth.login",
        tool: "login",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/platform/auth/login",
        summary: "Sign an outside agent or integration client in with its API key: an access and a refresh token, scopes limited to what the key has. People sign in at the gateway, never here.",
        consumes: &[],
        produces: &[Slot::one("access_token")],
        checks: &[],
        errors: &[INVALID_CREDENTIALS, ACCOUNT_LOCKED, SCOPE_NOT_GRANTED],
        audience: Audience::Shopper,
        action_type: ActionType::AUTHORIZE,
        person_scope: "",
    };
    type Input = LoginInput;
    type Output = Value;
}

fn wrong() -> ToolError {
    ToolError::new(INVALID_CREDENTIALS, "The client id or secret is not right.")
        .detail(json!({"reason": "invalid_credentials"}))
        .next("login")
}

fn locked(until: DateTime<Utc>, cx: &Cx) -> ToolError {
    ToolError::new(
        ACCOUNT_LOCKED,
        format!(
            "Too many wrong secrets: this client is locked until {}.",
            cx.time.format(until)
        ),
    )
    .detail(json!({"until": cx.time.format(until)}))
}

/// Whether a key can sign in at `now` at all (active, not expired).
fn usable(k: &KeyRow, now: DateTime<Utc>) -> bool {
    k.api_key_status == "active" && k.expires.is_none_or(|e| e > now)
}

/// Counts one wrong secret and locks the key when the count reaches the lockout rule. Runs in
/// its own transaction: the attempt is recorded even though the preview fails.
async fn count_wrong_secret(
    cx: &Cx,
    k: &KeyRow,
    rules: &ClientSessionRules,
) -> Result<Option<DateTime<Utc>>, ToolError> {
    let now = cx.clock.now();
    let window = Duration::minutes(rules.lockout.window_minutes);
    let fresh = k.first_failed_at.is_none_or(|f| now - f > window);
    let count = if fresh {
        1
    } else {
        i64::from(k.failed_attempt_count) + 1
    };
    let lock = (count >= rules.lockout.failed_attempts)
        .then(|| now + Duration::minutes(rules.lockout.lock_minutes));
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    sqlx::query(
        "UPDATE platform.api_key SET failed_attempt_count = $3,
           first_failed_at = CASE WHEN $4 THEN $5 ELSE first_failed_at END,
           locked_until = coalesce($6, locked_until), updated_at = now()
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(cx.tenant.as_str())
    .bind(&k.id)
    .bind(i32::try_from(count).unwrap_or(i32::MAX))
    .bind(fresh)
    .bind(now)
    .bind(lock)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(lock)
}

impl WriteOp<Cx> for Login {
    type Plan = LoginPlan;
    // Tokens once handed out cannot be taken back; `logout` ends the session.
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(&self, cx: &Cx, input: LoginInput) -> Result<Planned<LoginPlan>, ToolError> {
        let secret = match input.grant_type {
            GrantType::ClientCredentials => {
                input.client_secret_ref.as_deref().ok_or_else(|| {
                    ToolError::invalid_input("client_credentials needs `client_secret_ref`.")
                        .detail(json!({"field": "client_secret_ref", "reason": "missing"}))
                })?
            }
            GrantType::OauthCode => {
                if input.code.is_none() {
                    return Err(ToolError::invalid_input("oauth_code needs `code`.")
                        .detail(json!({"field": "code", "reason": "missing"})));
                }
                // This store has no authorization screen that hands out codes yet, so no code
                // can be right: answered like any wrong credential.
                return Err(wrong().detail(json!({"reason": "invalid_credentials",
                    "hint": "Authorization codes are not issued by this store yet; sign in with client_credentials and an API key."})));
            }
        };
        let rules = ClientSessionRules::from_policy(&cx.policy)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let k = key(&mut tx, &input.client_id).await?;
        let live = match &k {
            Some(k) => live_sessions(&mut tx, &k.id).await?,
            None => Vec::new(),
        };
        tx.commit().await.map_err(ToolError::from)?;
        let Some(k) = k else {
            return Err(wrong());
        };
        if let Some(until) = k.locked_until.filter(|u| *u > now) {
            return Err(locked(until, cx));
        }
        if sha256_hex(secret) != k.secret_hash {
            if usable(&k, now)
                && let Some(until) = count_wrong_secret(cx, &k, &rules).await?
            {
                return Err(locked(until, cx));
            }
            return Err(wrong());
        }
        if !usable(&k, now) {
            return Err(wrong());
        }
        let scopes: Vec<String> = match &input.requested_scopes {
            None => k.scopes.clone(),
            Some(asked) => {
                let given: Vec<String> = crate::role::tidy(asked)
                    .into_iter()
                    .filter(|s| k.scopes.contains(s))
                    .collect();
                if given.is_empty() {
                    return Err(ToolError::new(
                        SCOPE_NOT_GRANTED,
                        format!(
                            "This key has none of the scopes asked for; it has {}.",
                            k.scopes.join(", ")
                        ),
                    )
                    .detail(json!({"reason": "scope", "items": asked, "allowed": k.scopes}))
                    .next("list_permissions"));
                }
                given
            }
        };
        let ending = over_the_cap(&live, &rules);
        let mut changes = vec![Change {
            op: ChangeOp::Create,
            entity: "session".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"client_id": k.id, "kind": ActorKind::ExternalAgent, "scopes": scopes,
                       "expires_in": rules.access_token_minutes * 60}),
            note: None,
        }];
        changes.extend(ending.iter().map(|id| Change {
            op: ChangeOp::Revoke,
            entity: "session".into(),
            id: id.clone(),
            from: Value::Null,
            to: Value::Null,
            note: Some(format!(
                "The oldest session ends: a client keeps at most {} (client_sessions.max_sessions_per_client)",
                rules.max_sessions_per_client
            )),
        }));
        Ok(Planned {
            changes,
            checks: Vec::new(),
            basis: Vec::new(),
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan: LoginPlan {
                client_id: k.id,
                scopes,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &LoginPlan) -> Result<Committed, ToolError> {
        let rules = ClientSessionRules::from_policy(&tx.policy)?;
        let now = tx.clock.now();
        let k = key(tx, &plan.client_id).await?.ok_or_else(wrong)?;
        if !usable(&k, now) || k.locked_until.is_some_and(|u| u > now) {
            return Err(wrong());
        }
        let live = live_sessions(tx, &k.id).await?;
        end_sessions(tx, &over_the_cap(&live, &rules), now).await?;
        let opened = open_session(tx, &k.id, &plan.scopes, &rules).await?;
        sqlx::query(
            "UPDATE platform.api_key SET last_used_at = $3, failed_attempt_count = 0,
               first_failed_at = NULL, locked_until = NULL, updated_at = now()
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&k.id)
        .bind(now)
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
        plan: &LoginPlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row: Option<(Option<String>, Option<Vec<String>>)> = sqlx::query_as(
            "SELECT subject_id, scopes FROM platform.session WHERE tenant_id = $1 AND id = $2",
        )
        .bind(cx.tenant.as_str())
        .bind(&done.id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = row.is_some_and(|(s, sc)| {
            s.as_deref() == Some(&plan.client_id) && sc.as_ref() == Some(&plan.scopes)
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not the session previewed", done.id)]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        _plan: &LoginPlan,
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
pub(crate) mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    pub(crate) fn input(v: Value) -> LoginInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn secrets_are_write_only_inputs() {
        let s = closed_schema_for::<LoginInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["grant_type", "client_id"]));
        assert_eq!(s["properties"]["client_secret_ref"]["writeOnly"], true);
        assert_eq!(s["properties"]["code"]["writeOnly"], true);
        assert_eq!(
            acom_changeset::storable_input(
                &s,
                &json!({"grant_type": "client_credentials", "client_id": "key_31",
                        "client_secret_ref": "ak_live_secret"})
            )["client_secret_ref"],
            acom_changeset::NOT_STORED
        );
    }

    #[tokio::test]
    async fn codes_are_not_issued_here_and_modes_need_their_field() {
        let cx = crate::testdb::offline("key_31", ["external:read".to_owned()].into());
        let e = Login
            .preview(
                &cx,
                input(json!({"grant_type": "oauth_code", "client_id": "key_31", "code": "c_1"})),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("invalid_credentials", Some("login"))
        );
        let e = Login
            .preview(
                &cx,
                input(json!({"grant_type": "client_credentials", "client_id": "key_31"})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["field"], "client_secret_ref");
    }
}
