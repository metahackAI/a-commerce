//! `send_password_reset`: when a member cannot sign in, an operator sends a reset link (or code)
//! to the member's already verified contact. It reaches the member, so a person always approves.
//! No temporary password is made, nothing goes to a new contact, and no one sees the password;
//! the member finishes with the person-only `POST /v1/customer/password-reset/confirm`.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::Duration;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{account_basis, account_or_not_found};
use crate::access::{always, day, db, policy_number, staff_only};
use crate::verification::{self, not_delivered};
use crate::{
    CUSTOMER_NOT_FOUND, MEMBER_NOT_ACTIVE, METHOD_NOT_ENABLED, NO_VERIFIED_CONTACT, secret,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendPasswordResetInput {
    /// The member who cannot sign in (`search_members`).
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_alex")]
    pub customer_id: String,
    /// A reset method the store allows (`identity_verification.required_for.password_reset`),
    /// for example `email_link` or `sms_code`. The first allowed one when left out.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"email_link")]
    pub method: Option<String>,
    /// Why, in a sentence (for the action log).
    #[schemars(length(min = 1, max = 500), example = &"Locked out after 5 attempts, asked by phone")]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendResetPlan {
    pub request_id: String,
    pub customer_id: String,
    pub method: String,
    /// `email` or `sms`.
    pub channel: String,
    /// The verified contact it goes to.
    pub to: String,
    pub sends_code: bool,
    pub code_length: usize,
    pub ttl_seconds: i64,
    pub reason: String,
}

pub struct SendPasswordReset;

impl Operation for SendPasswordReset {
    const META: OpMeta = OpMeta {
        address: "customer.person.send_password_reset",
        tool: "send_password_reset",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/customer/people/{customer_id}/password-reset",
        summary: "Send a password reset link or code to a locked-out member's verified email or phone. A person always approves; it never makes, shows or sets a password, and a sent link cannot be taken back.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::one("recovery_request_id")],
        checks: &[
            "account_recovery.verified_contact_only",
            "account_recovery.rate_limits",
        ],
        errors: &[
            NO_VERIFIED_CONTACT,
            ErrorCode::RATE_LIMITED,
            MEMBER_NOT_ACTIVE,
            CUSTOMER_NOT_FOUND,
            METHOD_NOT_ENABLED,
        ],
        audience: Audience::Operator,
        action_type: ActionType::SEND,
        person_scope: "",
    };
    type Input = SendPasswordResetInput;
    type Output = Value;
}

/// Reset requests of a member since a time.
pub(crate) async fn requests_since(
    tx: &mut TxCx,
    customer_id: &str,
    since: chrono::DateTime<chrono::Utc>,
) -> Result<(i64, Option<chrono::DateTime<chrono::Utc>>), ToolError> {
    sqlx::query_as(
        "SELECT count(*), min(requested_at) FROM customer.recovery_request
         WHERE tenant_id = $1 AND customer_id = $2 AND recovery_kind = 'password_reset'
           AND requested_at >= $3",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(since)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Saves a reset request; the link token or code is made here and only its hash is kept.
/// Earlier open requests stop working (`account_recovery.invalidate_previous_requests`).
pub(crate) async fn save_request(
    tx: &mut TxCx,
    p: &SendResetPlan,
    invalidate_previous: bool,
    fingerprint: &str,
) -> Result<bool, ToolError> {
    let now = tx.clock.now();
    if invalidate_previous {
        sqlx::query(
            "UPDATE customer.recovery_request SET invalidated_at = $3, updated_at = now(),
               version = version + 1
             WHERE tenant_id = $1 AND customer_id = $2 AND recovery_kind = 'password_reset'
               AND used_at IS NULL AND invalidated_at IS NULL AND id <> $4",
        )
        .bind(tx.tenant.as_str())
        .bind(&p.customer_id)
        .bind(now)
        .bind(&p.request_id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    let (token, code) = if p.sends_code {
        (None, Some(secret::code(p.code_length)))
    } else {
        (Some(secret::token()), None)
    };
    let done = sqlx::query(
        "INSERT INTO customer.recovery_request (tenant_id, id, recovery_kind, customer_id,
           identifier_fingerprint, session_id, method, message_channel, token_hash, code_hash,
           agent_id, reason, requested_at, expires)
         VALUES ($1, $2, 'password_reset', $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&p.request_id)
    .bind(&p.customer_id)
    .bind(fingerprint)
    .bind(tx.principal.session_id.as_deref())
    .bind(&p.method)
    .bind(&p.channel)
    .bind(token.as_deref().map(secret::hash))
    .bind(code.as_deref().map(secret::hash))
    .bind(&tx.principal.subject)
    .bind(&p.reason)
    .bind(now)
    .bind(now + Duration::seconds(p.ttl_seconds))
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(done.rows_affected() > 0)
}

impl WriteOp<Cx> for SendPasswordReset {
    type Plan = SendResetPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SendPasswordResetInput,
    ) -> Result<Planned<SendResetPlan>, ToolError> {
        staff_only(cx)?;
        let reason = input.reason.trim().to_owned();
        if reason.is_empty() {
            return Err(ToolError::invalid_input("Say why the reset is sent.")
                .detail(json!({"field": "reason", "reason": "missing"})));
        }
        let allowed = verification::allowed(&cx.policy, "password_reset");
        let method_id = input
            .method
            .or_else(|| allowed.first().cloned())
            .unwrap_or_default();
        let m = verification::method(&cx.policy, "password_reset", &method_id)?;
        let limit = policy_number(
            cx,
            "customer.account_recovery.rate_limits.per_identifier.count",
        )?;
        let minutes = policy_number(
            cx,
            "customer.account_recovery.rate_limits.per_identifier.per_minutes",
        )?;
        let ttl = if m.kind == "magic_link" {
            policy_number(cx, "customer.identity_verification.link_ttl_seconds")?
        } else {
            policy_number(cx, "customer.identity_verification.code.ttl_seconds")?
        };
        let length = policy_number(cx, "customer.identity_verification.code.length")?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = account_or_not_found(&mut tx, &input.customer_id).await?;
        let (count, first) =
            requests_since(&mut tx, &a.id, now - Duration::minutes(minutes)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if !matches!(a.member_state.as_str(), "active" | "dormant") {
            return Err(ToolError::new(
                MEMBER_NOT_ACTIVE,
                format!(
                    "{} is {}; a reset link is not sent.",
                    a.id,
                    a.member_state.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "person", "from": a.member_state}))
            .next("get_member"));
        }
        // Only a contact the member already proved they own (verified_contact_only).
        let (to, verified_at) = match m.channel.as_deref() {
            Some("sms") => (a.telephone.clone(), a.telephone_verified_at),
            _ => (a.email.clone(), a.email_verified_at),
        };
        let (Some(to), Some(verified_at)) = (to, verified_at) else {
            return Err(ToolError::new(
                NO_VERIFIED_CONTACT,
                format!(
                    "{} has no verified {} to send a reset to.",
                    a.id,
                    if m.channel.as_deref() == Some("sms") {
                        "phone"
                    } else {
                        "email"
                    }
                ),
            )
            .detail(json!({"rule": "account_recovery.verified_contact_only"}))
            .next("update_member"));
        };
        if count >= limit {
            let retry = first.map_or(60, |f| {
                (f + Duration::minutes(minutes) - now).num_seconds().max(1)
            });
            return Err(ToolError::new(
                ErrorCode::RATE_LIMITED,
                format!("{limit} reset requests were made in the last {minutes} minutes."),
            )
            .detail(json!({"retry_after_seconds": retry,
                           "policy_key": "account_recovery.rate_limits.per_identifier"})));
        }
        let channel = m.channel.clone().unwrap_or_else(|| "email".into());
        let shown = verification::masked(&to);
        let sent_word = if m.sends_code() && m.kind == "one_time_code" {
            "code_sent"
        } else {
            "link_sent"
        };
        let facts = json!({"customer_id": a.id, "reaches_customers": true});
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Send,
                entity: "recovery_request".into(),
                id: String::new(),
                from: Value::Null,
                to: json!(sent_word),
                note: Some(format!("To {shown}; earlier links stop working")),
            }],
            checks: vec![
                Check {
                    rule: "account_recovery.verified_contact_only".into(),
                    passed: true,
                    detail: Some(format!(
                        "Sends to {shown}, verified {}",
                        day(cx.time, verified_at)
                    )),
                    fix_tool: None,
                },
                Check {
                    rule: "account_recovery.rate_limits".into(),
                    passed: true,
                    detail: Some(format!(
                        "{} of {limit} requests in {minutes} minutes",
                        count + 1
                    )),
                    fix_tool: None,
                },
            ],
            basis: vec![account_basis(&a)],
            approval: always(
                cx,
                Self::META.tool,
                &facts,
                "Sends a password reset link to a customer".into(),
            ),
            plan: SendResetPlan {
                request_id: format!("rcv_{}", acom_core::id::new_suffix()),
                customer_id: a.id,
                method: m.id.clone(),
                channel,
                to,
                sends_code: m.kind == "one_time_code",
                code_length: usize::try_from(length).unwrap_or(6),
                ttl_seconds: ttl,
                reason,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SendResetPlan) -> Result<Committed, ToolError> {
        let invalidate = tx
            .policy
            .lookup("customer.account_recovery.invalidate_previous_requests")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        save_request(tx, plan, invalidate, &secret::fingerprint(&plan.to)).await?;
        let mut produced = Map::new();
        produced.insert("recovery_request_id".into(), json!(plan.request_id));
        produced.insert("delivery".into(), not_delivered(&plan.channel, &plan.to));
        Ok(Committed {
            id: plan.request_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SendResetPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let found: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM customer.recovery_request WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.request_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match found {
            Some(_) => vec![],
            None => vec![format!("{} was not saved", plan.request_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        _plan: &SendResetPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        // Not reached: a sent link cannot be taken back (UNDO is NotSupported).
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_always_waits_for_a_person_and_has_no_password() {
        let s = closed_schema_for::<SendPasswordResetInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert!(s["properties"].get("password").is_none());
        assert_eq!(SendPasswordReset::META.kind, Kind::WriteApproval);
        assert_eq!(SendPasswordReset::UNDO, UndoSupport::NotSupported);
    }
}
