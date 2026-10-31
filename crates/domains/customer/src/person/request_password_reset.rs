//! Person-only `POST /v1/customer/password-reset/requests`: someone who cannot sign in asks for
//! a reset. The answer is the same whether an account matches or not, so it never tells who has
//! an account (`account_recovery.uniform_response`); when one matches, a link or code goes to
//! its verified contact (delivery is a stub that records, not sends).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, ToolError,
};
use acom_store::TxCx;
use chrono::Duration;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{SendResetPlan, save_request};
use crate::access::db;
use crate::verification;
use crate::{INVALID_IDENTIFIER_FORMAT, repo, secret, wording};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestPasswordResetInput {
    /// The email, sign-in id or phone number of the account (`account_recovery.identifiers`).
    #[schemars(length(min = 3, max = 200), example = &"alex.kim@example.com")]
    pub identifier: String,
    /// A reset method the store allows (`identity_verification.required_for.password_reset`).
    /// The first one the account can receive when left out.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"email_link")]
    pub method: Option<String>,
    /// The page's bot-check token, when it has one. No bot-check provider is connected yet, so
    /// it is accepted and not checked.
    #[schemars(length(max = 2000), example = &"chk_8f2c")]
    #[allow(dead_code)]
    pub client_challenge: Option<String>,
}

/// Person-only: ask for a password reset.
pub struct RequestPasswordReset;

impl Operation for RequestPasswordReset {
    const META: OpMeta = OpMeta {
        address: "customer.person.request_password_reset",
        tool: "request_password_reset",
        group: Group::Engage,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/customer/password-reset/requests",
        summary: "Ask for a password reset link or code. The same answer whether or not an account matches.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[INVALID_IDENTIFIER_FORMAT, ErrorCode::RATE_LIMITED],
        audience: Audience::Both,
        action_type: ActionType::SEND,
        person_scope: "human:account",
    };
    type Input = RequestPasswordResetInput;
    type Output = Value;
}

fn number(tx: &TxCx, path: &str) -> i64 {
    tx.policy
        .lookup(&format!("customer.{path}"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

/// The uniform answer (account or not).
fn accepted(request_id: &str, retry_after: i64) -> Value {
    json!({"status": "accepted", "request_id": request_id,
           "message": "If an account matches, we sent instructions to its verified contact.",
           "retry_after_seconds": retry_after})
}

impl PersonOp<acom_store::Cx> for RequestPasswordReset {
    async fn run(
        &self,
        tx: &mut TxCx,
        input: RequestPasswordResetInput,
    ) -> Result<Value, ToolError> {
        let raw = input.identifier.trim();
        let kinds: Vec<String> = tx
            .policy
            .lookup("customer.account_recovery.identifiers")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let shape_ok = (kinds.iter().any(|k| k == "email") && wording::looks_like_email(raw))
            || (kinds.iter().any(|k| k == "telephone") && wording::looks_like_phone(raw))
            || (kinds.iter().any(|k| k == "login_id") && wording::looks_like_login_id(raw));
        if !shape_ok {
            return Err(ToolError::new(
                INVALID_IDENTIFIER_FORMAT,
                "Enter the email, sign-in id or phone number of the account.",
            )
            .detail(json!({"field": "identifier", "reason": "format", "allowed": kinds})));
        }
        let now = tx.clock.now();
        let normalized = secret::normalize(raw);
        let print = secret::hash(&normalized);
        let cooldown = number(tx, "identity_verification.resend_cooldown_seconds");
        let per_id = number(tx, "account_recovery.rate_limits.per_identifier.count");
        let per_id_minutes = number(
            tx,
            "account_recovery.rate_limits.per_identifier.per_minutes",
        );
        let per_client = number(tx, "account_recovery.rate_limits.per_client.count");
        let per_client_minutes = number(tx, "account_recovery.rate_limits.per_client.per_minutes");
        let session = tx.principal.session_id.clone();

        // A quick retry of the same identifier gets the same request back.
        let recent: Option<(String, chrono::DateTime<chrono::Utc>)> = sqlx::query_as(
            "SELECT id, requested_at FROM customer.recovery_request
             WHERE tenant_id = $1 AND identifier_fingerprint = $2 AND recovery_kind = 'password_reset'
               AND requested_at > $3
             ORDER BY requested_at DESC, id DESC LIMIT 1",
        )
        .bind(tx.tenant.as_str())
        .bind(&print)
        .bind(now - Duration::seconds(cooldown))
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        if let Some((id, at)) = recent {
            let left = (at + Duration::seconds(cooldown) - now)
                .num_seconds()
                .max(1);
            return Ok(accepted(&id, left));
        }
        // The same limits for everyone, whether an account matches or not.
        let by_id: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM customer.recovery_request
             WHERE tenant_id = $1 AND identifier_fingerprint = $2 AND requested_at >= $3",
        )
        .bind(tx.tenant.as_str())
        .bind(&print)
        .bind(now - Duration::minutes(per_id_minutes))
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let by_client: i64 = match &session {
            Some(s) => sqlx::query_scalar(
                "SELECT count(*) FROM customer.recovery_request
                 WHERE tenant_id = $1 AND session_id = $2 AND requested_at >= $3",
            )
            .bind(tx.tenant.as_str())
            .bind(s)
            .bind(now - Duration::minutes(per_client_minutes))
            .fetch_one(&mut *tx.tx)
            .await
            .map_err(db)?,
            None => 0,
        };
        if by_id >= per_id || by_client >= per_client {
            let retry = 60
                * if by_id >= per_id {
                    per_id_minutes
                } else {
                    per_client_minutes
                };
            return Err(ToolError::new(
                ErrorCode::RATE_LIMITED,
                "Too many reset requests. Try again later.",
            )
            .detail(json!({"retry_after_seconds": retry,
                           "policy_key": "account_recovery.rate_limits"})));
        }

        let request_id = format!("rcv_{}", acom_core::id::new_suffix());
        let account = repo::account_by_identifier(tx, &normalized).await?;
        let allowed = verification::allowed(&tx.policy, "password_reset");
        let chosen = account.as_ref().and_then(|a| {
            // The asked-for method, or the first one the account has a verified contact for.
            let methods: Vec<&String> = match &input.method {
                Some(m) => allowed.iter().filter(|x| *x == m).collect(),
                None => allowed.iter().collect(),
            };
            methods.into_iter().find_map(|m| {
                let def = verification::method(&tx.policy, "password_reset", m).ok()?;
                let (to, verified) = match def.channel.as_deref() {
                    Some("sms") => (a.telephone.clone(), a.telephone_verified_at),
                    _ => (a.email.clone(), a.email_verified_at),
                };
                verified.and(to).map(|to| (def, to))
            })
        });
        match (account, chosen) {
            (Some(a), Some((def, to))) if a.member_state != "blocked" => {
                let ttl = if def.kind == "magic_link" {
                    number(tx, "identity_verification.link_ttl_seconds")
                } else {
                    number(tx, "identity_verification.code.ttl_seconds")
                };
                let plan = SendResetPlan {
                    request_id: request_id.clone(),
                    customer_id: a.id.clone(),
                    method: def.id.clone(),
                    channel: def.channel.clone().unwrap_or_else(|| "email".into()),
                    to,
                    sends_code: def.kind == "one_time_code",
                    code_length: usize::try_from(number(tx, "identity_verification.code.length"))
                        .unwrap_or(6),
                    ttl_seconds: ttl,
                    reason: if a.password_hash.is_none() {
                        "Set-up link: the account signs in with a linked login only".into()
                    } else {
                        "Requested by the member".into()
                    },
                };
                // Keyed by the typed identifier, so the per-identifier limit sees it.
                save_request(tx, &plan, true, &print).await?;
            }
            _ => {
                // No account (or none that can receive it): the same row and the same answer.
                sqlx::query(
                    "INSERT INTO customer.recovery_request (tenant_id, id, recovery_kind,
                       identifier_fingerprint, session_id, agent_id, requested_at, expires)
                     VALUES ($1, $2, 'password_reset', $3, $4, $5, $6, $6)",
                )
                .bind(tx.tenant.as_str())
                .bind(&request_id)
                .bind(&print)
                .bind(session.as_deref())
                .bind(&tx.principal.subject)
                .bind(now)
                .execute(&mut *tx.tx)
                .await
                .map_err(db)?;
            }
        }
        Ok(accepted(&request_id, cooldown))
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_is_person_only() {
        let s = closed_schema_for::<RequestPasswordResetInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(RequestPasswordReset::META.kind, Kind::Person);
        assert_eq!(RequestPasswordReset::META.person_scope, "human:account");
    }
}
