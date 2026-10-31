//! Person-only `POST /v1/customer/account-recovery/login-id`: someone who forgot their sign-in id
//! gives an email or phone. Without proof of owning it, the answer is always the same and the id
//! goes to that contact (delivery is a stub); with a verification token for the contact and a store
//! that allows it (`login_id_recovery.delivery: show_masked_after_verification`), the masked id
//! is shown.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, ToolError,
};
use acom_store::{Cx, TxCx};
use chrono::Duration;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::log_person;
use crate::access::db;
use crate::verification::{not_delivered, usable, use_token};
use crate::{INVALID_CONTACT_FORMAT, VERIFICATION_EXPIRED, repo, secret, wording};

/// What kind of contact is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContactType {
    /// An email address.
    Email,
    /// A phone number.
    Telephone,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecoverLoginIdInput {
    /// email or telephone.
    #[schemars(example = &"email")]
    pub contact_type: ContactType,
    /// The email or phone number the account uses.
    #[schemars(length(min = 3, max = 200), example = &"jamie.rivera@example.com")]
    pub contact: String,
    /// A finished check of this contact (`login_id_recovery` purpose), to see the id here.
    #[schemars(regex(pattern = r"^ivf_[a-z0-9_]+$"), example = &"ivf_205")]
    pub verification_token: Option<String>,
}

/// Person-only: find a forgotten sign-in id.
pub struct RecoverLoginId;

impl Operation for RecoverLoginId {
    const META: OpMeta = OpMeta {
        address: "customer.person.recover_login_id",
        tool: "recover_login_id",
        group: Group::Engage,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/customer/account-recovery/login-id",
        summary: "Find a forgotten sign-in id from an email or phone. The same answer whether or not an account uses it, unless the contact is verified.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[
            INVALID_CONTACT_FORMAT,
            VERIFICATION_EXPIRED,
            ErrorCode::RATE_LIMITED,
        ],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "human:account",
    };
    type Input = RecoverLoginIdInput;
    type Output = Value;
}

fn number(tx: &TxCx, path: &str) -> i64 {
    tx.policy
        .lookup(&format!("customer.{path}"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

impl PersonOp<Cx> for RecoverLoginId {
    async fn run(&self, tx: &mut TxCx, input: RecoverLoginIdInput) -> Result<Value, ToolError> {
        let contact = input.contact.trim();
        let fits = match input.contact_type {
            ContactType::Email => wording::looks_like_email(contact),
            ContactType::Telephone => wording::looks_like_phone(contact),
        };
        if !fits {
            return Err(ToolError::new(
                INVALID_CONTACT_FORMAT,
                "Enter the email or phone number the account uses.",
            )
            .detail(json!({"field": "contact", "reason": "format"})));
        }
        let now = tx.clock.now();
        let normalized = secret::normalize(contact);
        let print = secret::hash(&normalized);
        let per_id = number(tx, "account_recovery.rate_limits.per_identifier.count");
        let minutes = number(
            tx,
            "account_recovery.rate_limits.per_identifier.per_minutes",
        );
        let recent: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM customer.recovery_request
             WHERE tenant_id = $1 AND identifier_fingerprint = $2 AND recovery_kind = 'login_id'
               AND requested_at >= $3",
        )
        .bind(tx.tenant.as_str())
        .bind(&print)
        .bind(now - Duration::minutes(minutes))
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        if recent >= per_id {
            return Err(ToolError::new(
                ErrorCode::RATE_LIMITED,
                "Too many requests for this contact. Try again later.",
            )
            .detail(json!({"retry_after_seconds": minutes * 60,
                           "policy_key": "account_recovery.rate_limits.per_identifier"})));
        }
        let account = repo::account_by_identifier(tx, &normalized).await?;
        let delivery = tx
            .policy
            .lookup("customer.account_recovery.login_id_recovery.delivery")
            .and_then(Value::as_str)
            .unwrap_or("send_to_verified_contact")
            .to_owned();

        // Proof of owning the contact, where the store shows the id on screen.
        if let Some(token) = &input.verification_token {
            let c = usable(
                tx,
                token,
                "login_id_recovery",
                Some(contact),
                "verify_identity",
            )
            .await
            .map_err(|_| {
                ToolError::new(
                    VERIFICATION_EXPIRED,
                    "The check of this contact expired or does not match; verify it again.",
                )
            })?;
            use_token(tx, &c.id).await?;
            if delivery == "show_masked_after_verification" {
                let items: Vec<Value> = account
                    .iter()
                    .map(|a| {
                        let id = a.login_id.clone().or_else(|| a.email.clone()).unwrap_or_default();
                        let mut methods = vec![];
                        if a.password_hash.is_some() {
                            methods.push("password");
                        }
                        json!({"login_id": wording::mask_login_id(&id),
                               "joined_at": a.registered_at.with_timezone(&tx.time.zone).format("%Y-%m").to_string(),
                               "login_methods": methods})
                    })
                    .collect();
                return Ok(json!({"items": items, "total": items.len()}));
            }
        }

        // Otherwise the id goes to the contact (a stub that records, not sends), and the answer
        // does not say whether an account uses it.
        let request_id = format!("rcv_{}", acom_core::id::new_suffix());
        let channel = match input.contact_type {
            ContactType::Email => "email",
            ContactType::Telephone => "sms",
        };
        sqlx::query(
            "INSERT INTO customer.recovery_request (tenant_id, id, recovery_kind, customer_id,
               identifier_fingerprint, session_id, message_channel, agent_id, requested_at, expires)
             VALUES ($1, $2, 'login_id', $3, $4, $5, $6, $7, $8, $8)",
        )
        .bind(tx.tenant.as_str())
        .bind(&request_id)
        .bind(account.as_ref().map(|a| a.id.as_str()))
        .bind(&print)
        .bind(tx.principal.session_id.as_deref())
        .bind(account.as_ref().map(|_| channel))
        .bind(&tx.principal.subject)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        if let Some(a) = &account {
            log_person(
                tx,
                Self::META.tool,
                ActionType::SEND,
                &a.id,
                json!({"login_id": "sent", "request_id": request_id,
                       "delivery": not_delivered(channel, contact)}),
            )
            .await?;
        }
        Ok(json!({"status": "accepted",
                  "message": "If an account uses this contact, we sent its sign-in id there."}))
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_is_person_only() {
        let s = closed_schema_for::<RecoverLoginIdInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(RecoverLoginId::META.person_scope, "human:account");
    }
}
