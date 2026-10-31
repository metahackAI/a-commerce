//! Person-only `POST /v1/customer/password-reset/confirm`: whoever got the reset link or code sets
//! a new password. Every session of the account ends (`password.on_reset`), the lockout is lifted,
//! and the action log says only that the password was reset.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, PersonOp, ToolError,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{attempt_recorded, log_person, set_password};
use crate::access::db;
use crate::verification::not_delivered;
use crate::{
    INVALID_OR_EXPIRED, PASSWORD_POLICY, PASSWORD_REUSED, TOO_MANY_ATTEMPTS, repo, secret,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmPasswordResetInput {
    /// The token from the reset link. Or give `request_id` and `code`.
    #[schemars(length(min = 16, max = 200), example = &"9f3c…")]
    pub token: Option<String>,
    /// The reset request the code belongs to (`request_id` of the request).
    #[schemars(regex(pattern = r"^rcv_[a-z0-9_]+$"), example = &"rcv_9f31")]
    pub request_id: Option<String>,
    /// The code that was sent, with `request_id`.
    #[schemars(regex(pattern = r"^[0-9]{4,12}$"), example = &"482913")]
    pub code: Option<String>,
    /// The new password (rules: `get_policy` `password`).
    #[schemars(length(min = 1, max = 200), example = &"harbor-lantern-42")]
    pub new_password: String,
}

/// Person-only: set a new password with a reset link or code.
pub struct ConfirmPasswordReset;

impl Operation for ConfirmPasswordReset {
    const META: OpMeta = OpMeta {
        address: "customer.person.confirm_password_reset",
        tool: "confirm_password_reset",
        group: Group::Engage,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/customer/password-reset/confirm",
        summary: "Set a new password with the reset link or code; every session of the account ends.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[
            INVALID_OR_EXPIRED,
            TOO_MANY_ATTEMPTS,
            PASSWORD_POLICY,
            PASSWORD_REUSED,
        ],
        audience: Audience::Both,
        action_type: ActionType::UPDATE,
        person_scope: "human:account",
    };
    type Input = ConfirmPasswordResetInput;
    type Output = Value;
}

#[derive(Debug, sqlx::FromRow)]
struct RequestRow {
    id: String,
    customer_id: Option<String>,
    code_hash: Option<String>,
    attempts: i32,
    expires: DateTime<Utc>,
    used_at: Option<DateTime<Utc>>,
    invalidated_at: Option<DateTime<Utc>>,
    message_channel: Option<String>,
}

const COLUMNS: &str = "id, customer_id, code_hash, attempts, expires, used_at, invalidated_at,
    message_channel";

fn invalid() -> ToolError {
    ToolError::new(
        INVALID_OR_EXPIRED,
        "This reset link or code does not work any more. Ask for a new one.",
    )
}

impl PersonOp<Cx> for ConfirmPasswordReset {
    async fn run(
        &self,
        tx: &mut TxCx,
        input: ConfirmPasswordResetInput,
    ) -> Result<Value, ToolError> {
        let now = tx.clock.now();
        let row: Option<RequestRow> = match (&input.token, &input.request_id, &input.code) {
            (Some(token), None, None) => {
                sqlx::query_as::<_, RequestRow>(sqlx::AssertSqlSafe(format!(
                    "SELECT {COLUMNS} FROM customer.recovery_request
                 WHERE tenant_id = $1 AND token_hash = $2 AND recovery_kind = 'password_reset'"
                )))
                .bind(tx.tenant.as_str())
                .bind(secret::hash(token.trim()))
                .fetch_optional(&mut *tx.tx)
                .await
                .map_err(db)?
            }
            (None, Some(id), Some(_)) => {
                sqlx::query_as::<_, RequestRow>(sqlx::AssertSqlSafe(format!(
                    "SELECT {COLUMNS} FROM customer.recovery_request
                 WHERE tenant_id = $1 AND id = $2 AND recovery_kind = 'password_reset'
                 FOR UPDATE"
                )))
                .bind(tx.tenant.as_str())
                .bind(id)
                .fetch_optional(&mut *tx.tx)
                .await
                .map_err(db)?
            }
            _ => {
                return Err(ToolError::invalid_input(
                    "Send the link's `token`, or `request_id` with `code`.",
                )
                .detail(json!({"field": "token", "reason": "missing"})));
            }
        };
        let Some(r) = row else {
            return Err(invalid());
        };
        if r.used_at.is_some() || r.invalidated_at.is_some() || r.expires <= now {
            return Err(invalid());
        }
        if let Some(code) = &input.code {
            let max = tx
                .policy
                .lookup("customer.account_recovery.confirm_max_attempts")
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
                .unwrap_or(i32::MAX);
            let right = r
                .code_hash
                .as_deref()
                .is_some_and(|h| secret::matches(h, code.trim()));
            if !right {
                let attempts = r.attempts + 1;
                let dead = attempts >= max;
                sqlx::query(
                    "UPDATE customer.recovery_request SET attempts = $3,
                       invalidated_at = CASE WHEN $4 THEN $5 ELSE invalidated_at END,
                       updated_at = now(), version = version + 1
                     WHERE tenant_id = $1 AND id = $2",
                )
                .bind(tx.tenant.as_str())
                .bind(&r.id)
                .bind(attempts)
                .bind(dead)
                .bind(now)
                .execute(&mut *tx.tx)
                .await
                .map_err(db)?;
                let e = if dead {
                    ToolError::new(
                        TOO_MANY_ATTEMPTS,
                        "Too many wrong codes; this request no longer works. Ask for a new one.",
                    )
                    .detail(json!({"limit": max,
                                   "policy_key": "account_recovery.confirm_max_attempts"}))
                } else {
                    invalid()
                };
                return Err(attempt_recorded(e));
            }
        }
        let Some(customer_id) = r.customer_id.clone() else {
            return Err(invalid());
        };
        let Some(a) = repo::account(tx, &customer_id).await? else {
            return Err(invalid());
        };
        if a.member_state == "withdrawn" {
            return Err(invalid());
        }
        set_password(tx, &a, &input.new_password).await?;
        sqlx::query(
            "UPDATE customer.recovery_request SET used_at = $3, updated_at = now(),
               version = version + 1 WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&r.id)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        let all = tx
            .policy
            .lookup("customer.password.on_reset")
            .and_then(Value::as_str)
            != Some("keep");
        let revoked = if all {
            acom_platform::service::for_customer::end_sessions(tx, &a.id, None).await?
        } else {
            0
        };
        let notify = tx
            .policy
            .lookup("customer.account_recovery.notify_on_reset")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let notice = match (notify, r.message_channel.as_deref()) {
            (true, Some("sms")) => a.telephone.as_deref().map(|t| not_delivered("sms", t)),
            (true, _) => a.email.as_deref().map(|e| not_delivered("email", e)),
            _ => None,
        };
        log_person(
            tx,
            Self::META.tool,
            ActionType::UPDATE,
            &a.id,
            json!({"password": "reset", "request_id": r.id, "sessions_revoked": revoked}),
        )
        .await?;
        Ok(
            json!({"ok": true, "id": a.id, "sessions_revoked": revoked, "signed_in": false,
                  "next": "sign_in", "notice": notice}),
        )
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_is_person_only() {
        let s = closed_schema_for::<ConfirmPasswordResetInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(ConfirmPasswordReset::META.person_scope, "human:account");
    }
}
