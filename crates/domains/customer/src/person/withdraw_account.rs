//! Person-only `POST /v1/customer/me/withdrawal`: a signed-in member leaves the store. Only the
//! member can (`withdrawal.initiator: self_only`): operators and agents are refused. The screen
//! first shows `check_withdrawal_eligibility`; the member picks reasons, accepts the data
//! handling, signs in again, and presses the button. What happens is policy data
//! (`withdrawal`): blockers, settlement, masking and deletion now, what is kept and for how long,
//! and when the same person may join again.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, ToolError,
};
use acom_core::enums::{ActorKind, TierChangeCause, WithdrawalReason};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{attempt_recorded, check_password, eligibility, locked_until, log_person};
use crate::access::{day, db};
use crate::verification::{not_delivered, usable, use_token};
use crate::{
    ALREADY_WITHDRAWN, CONFIRMATION_REQUIRED, FORBIDDEN_INITIATOR, REAUTHENTICATION_FAILED,
    REAUTHENTICATION_LOCKED, WITHDRAWAL_BLOCKED, consent, repo, secret, wording,
};

/// The member confirms what they read.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WithdrawalConfirmation {
    /// The member accepted the data handling summary.
    #[schemars(example = true)]
    pub acknowledged_data_handling: bool,
    /// `checked_at` of the eligibility answer the member saw.
    #[schemars(example = &"2026-11-07T14:30:00-05:00")]
    pub eligibility_checked_at: String,
}

/// How the member proves it is them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReauthMethod {
    /// The current password, in `credential`.
    Password,
    /// A finished email code check, in `verification_token`.
    EmailCode,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reauthentication {
    /// password or email_code (`identity_verification.required_for.withdrawal_reauth`).
    #[schemars(example = &"password")]
    pub method: ReauthMethod,
    /// The current password, with `method: password`.
    #[schemars(length(min = 1, max = 200), example = &"harbor-lantern-42")]
    pub credential: Option<String>,
    /// A finished check (`withdrawal_reauth` purpose), with `method: email_code`.
    #[schemars(regex(pattern = r"^ivf_[a-z0-9_]+$"), example = &"ivf_206")]
    pub verification_token: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WithdrawAccountInput {
    /// Why the member leaves (`withdrawal.reasons`).
    #[schemars(length(min = 1), example = ["not_using", "too_many_messages"])]
    pub reasons: Vec<WithdrawalReason>,
    /// Anything the member wants to add, up to 500 characters.
    #[schemars(length(max = 500), example = &"I shop less these days.")]
    pub reason_text: Option<String>,
    /// What the member confirmed.
    pub confirmation: WithdrawalConfirmation,
    /// The member signing in again.
    pub reauthentication: Reauthentication,
}

/// Person-only: the member leaves the store.
pub struct WithdrawAccount;

impl Operation for WithdrawAccount {
    const META: OpMeta = OpMeta {
        address: "customer.person.withdraw",
        tool: "withdraw_account",
        group: Group::Engage,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/customer/me/withdrawal",
        summary: "The signed-in member leaves the store, after the eligibility check and signing in again.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[
            WITHDRAWAL_BLOCKED,
            REAUTHENTICATION_FAILED,
            REAUTHENTICATION_LOCKED,
            CONFIRMATION_REQUIRED,
            ErrorCode::ESTIMATE_CHANGED,
            ALREADY_WITHDRAWN,
            FORBIDDEN_INITIATOR,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::DELETE,
        person_scope: "human:account",
    };
    type Input = WithdrawAccountInput;
    type Output = Value;
}

fn words(tx: &TxCx, path: &str) -> Vec<String> {
    tx.policy
        .lookup(&format!("customer.withdrawal.{path}"))
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// What `withdrawal.on_withdraw.{key}` says to do.
fn on_withdraw(tx: &TxCx, key: &str) -> String {
    tx.policy
        .lookup(&format!("customer.withdrawal.on_withdraw.{key}"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn number(tx: &TxCx, path: &str) -> i64 {
    tx.policy
        .lookup(&format!("customer.withdrawal.{path}"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

/// When a new inquiry or remedy of the member appeared after `since`.
async fn changed_since(
    tx: &mut TxCx,
    customer_id: &str,
    since: DateTime<Utc>,
) -> Result<bool, ToolError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM customer.customer_inquiry
                        WHERE tenant_id = $1 AND customer_id = $2 AND date_received > $3)
             OR EXISTS (SELECT 1 FROM customer.remedy
                        WHERE tenant_id = $1 AND customer_id = $2 AND requested_at > $3)",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(since)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

impl PersonOp<Cx> for WithdrawAccount {
    async fn run(&self, tx: &mut TxCx, input: WithdrawAccountInput) -> Result<Value, ToolError> {
        // Only the member themselves (an operator's session is refused).
        let me = match (tx.principal.kind, tx.principal.customer_id()) {
            (ActorKind::Shopper, Some(id)) => id.to_owned(),
            _ => {
                return Err(ToolError::new(
                    FORBIDDEN_INITIATOR,
                    "Only the member can close their own account.",
                )
                .detail(json!({"reason": "initiator", "rule": "withdrawal.initiator"})));
            }
        };
        let now = tx.clock.now();
        let a = repo::account(tx, &me)
            .await?
            .ok_or_else(crate::access::not_signed_in)?;
        if a.member_state == "withdrawn" {
            return Err(ToolError::new(
                ALREADY_WITHDRAWN,
                "This account is already closed.",
            ));
        }
        // What the member confirmed, and how recently they looked.
        let max_age = number(tx, "confirmation_max_age_minutes");
        let checked = tx
            .time
            .parse(&input.confirmation.eligibility_checked_at)
            .ok()
            .filter(|t| *t <= now && now - *t <= Duration::minutes(max_age));
        let Some(checked_at) = checked.filter(|_| input.confirmation.acknowledged_data_handling)
        else {
            return Err(ToolError::new(
                CONFIRMATION_REQUIRED,
                format!(
                    "Read the eligibility again (it is valid for {max_age} minutes) and accept the data handling."
                ),
            )
            .detail(json!({"field": "confirmation",
                           "rule": "withdrawal.confirmation_max_age_minutes"})));
        };
        let allowed_reasons = words(tx, "reasons");
        if let Some(r) = input
            .reasons
            .iter()
            .find(|r| !allowed_reasons.iter().any(|a| a == r.as_str()))
        {
            return Err(
                ToolError::invalid_input(format!("{r} is not a reason this store lists.")).detail(
                    json!({"field": "reasons", "reason": "enum", "allowed": allowed_reasons}),
                ),
            );
        }
        // Signing in again.
        if let Some(until) = locked_until(tx, &a) {
            return Err(ToolError::new(
                REAUTHENTICATION_LOCKED,
                "Too many wrong passwords; try again later or reset the password.",
            )
            .detail(json!({"until": tx.time.format(until)})));
        }
        let methods = crate::verification::allowed(&tx.policy, "withdrawal_reauth");
        let r = &input.reauthentication;
        let method_word = match r.method {
            ReauthMethod::Password => "password",
            ReauthMethod::EmailCode => "email_code",
        };
        let failed = || ToolError::new(REAUTHENTICATION_FAILED, "Signing in again did not work.");
        if !methods.iter().any(|m| m == method_word) {
            return Err(failed().detail(json!({"field": "reauthentication.method",
                                              "allowed": methods})));
        }
        match r.method {
            ReauthMethod::Password => {
                let pw = r.credential.as_deref().unwrap_or_default();
                if !check_password(tx, &a, pw).await? {
                    return Err(attempt_recorded(failed()));
                }
            }
            ReauthMethod::EmailCode => {
                let token = r.verification_token.as_deref().unwrap_or_default();
                let c = usable(
                    tx,
                    token,
                    "withdrawal_reauth",
                    a.email.as_deref(),
                    "verify_identity",
                )
                .await
                .map_err(|_| failed())?;
                use_token(tx, &c.id).await?;
            }
        }
        // Blockers, checked again now.
        let e = eligibility(tx, &a.id).await?;
        if changed_since(tx, &a.id, checked_at).await? && !e.blockers.is_empty() {
            return Err(ToolError::new(
                ErrorCode::ESTIMATE_CHANGED,
                "Something changed since you checked; check the eligibility again.",
            )
            .detail(json!({"blockers": e.blockers})));
        }
        if !e.blockers.is_empty() {
            let log = tx
                .policy
                .lookup("customer.withdrawal.on_withdraw.log_blocked_attempts")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let err = ToolError::new(
                WITHDRAWAL_BLOCKED,
                "The account cannot be closed yet; see what blocks it.",
            )
            .detail(json!({"blockers": e.blockers}));
            if log {
                // The blocked attempt is kept in the action log (withdrawal.on_withdraw).
                log_person(
                    tx,
                    Self::META.tool,
                    ActionType::DELETE,
                    &a.id,
                    json!({"blocked": true, "blockers": e.blockers.len()}),
                )
                .await?;
                return Err(attempt_recorded(err));
            }
            return Err(err);
        }

        // Leaving.
        let cooling = number(tx, "cooling_off_days");
        let pending = cooling > 0;
        let state = if pending {
            "pending_withdrawal"
        } else {
            "withdrawn"
        };
        let rejoin_after = now + Duration::days(number(tx, "rejoin.block_days"));
        let reuse = |kind: &str| -> Option<DateTime<Utc>> {
            match tx
                .policy
                .lookup(&format!("customer.withdrawal.identifier_reuse.{kind}"))
                .and_then(Value::as_str)
            {
                Some("immediately") => Some(now),
                Some("never") => None,
                _ => Some(rejoin_after),
            }
        };
        let reusable_after = match (reuse("email"), reuse("login_id")) {
            (Some(x), Some(y)) => Some(x.max(y)),
            _ => None,
        };
        let prints: Vec<String> = [
            a.email.as_deref(),
            a.telephone.as_deref(),
            a.login_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(secret::fingerprint)
        .collect();
        let retain: Vec<Value> = tx
            .policy
            .lookup("customer.withdrawal.data_handling.retain")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let data_retention: Vec<Value> = retain
            .iter()
            .map(|r| {
                let days = r.get("days").and_then(Value::as_i64).unwrap_or(0);
                json!({"data": r.get("data"), "until": day(tx.time, now + Duration::days(days)),
                       "why": r.get("why")})
            })
            .collect();
        let mut settled = Vec::new();
        // Coupons, subscriptions, linked logins and balances belong to other domains; they are
        // named as the policy says and left for those domains (not connected here).
        for (kind, action) in [
            ("coupons", on_withdraw(tx, "coupons")),
            (
                "active_subscriptions",
                on_withdraw(tx, "active_subscriptions"),
            ),
            ("linked_logins", on_withdraw(tx, "linked_logins")),
        ] {
            settled.push(json!({"type": kind, "action": action, "connected": false}));
        }
        if !pending {
            // Tier: removed, and the history says why.
            for m in repo::memberships(tx, &a.id).await? {
                repo::add_history(
                    tx,
                    &repo::TierChange {
                        customer_id: a.id.clone(),
                        program_id: m.member_program_id.clone(),
                        from_tier_id: Some(m.tier_id.clone()),
                        to_tier_id: None,
                        cause: TierChangeCause::Withdrawal.as_str().into(),
                        evaluation_id: None,
                        snapshot: None,
                        agent_id: a.id.clone(),
                        reason: None,
                        expires: None,
                    },
                )
                .await?;
            }
            sqlx::query(
                "DELETE FROM customer.program_membership WHERE tenant_id = $1 AND customer_id = $2",
            )
            .bind(tx.tenant.as_str())
            .bind(&a.id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
            settled.push(json!({"type": "tier", "action": on_withdraw(tx, "tier")}));
            // Employee discount ends.
            sqlx::query(
                "UPDATE customer.employee_role SET end_date = $3, updated_at = now(),
                   version = version + 1
                 WHERE tenant_id = $1 AND customer_id = $2 AND end_date IS NULL",
            )
            .bind(tx.tenant.as_str())
            .bind(&a.id)
            .bind(now.with_timezone(&tx.time.zone).date_naive())
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
            // Marketing consent is withdrawn (append only).
            let optional = tx
                .policy
                .lookup("customer.consent.optional")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            for c in consent::current(tx, &a.id).await? {
                if c.granted && optional.contains(&c.consent_type) {
                    consent::record(
                        tx,
                        &consent::new_id(),
                        &a.id,
                        &c.consent_type,
                        false,
                        Self::META.tool,
                    )
                    .await?;
                }
            }
            // Deleted now: addresses, memories, wishlist (data_handling.delete_now).
            let delete_now = words(tx, "data_handling.delete_now");
            for (data, table) in [
                ("addresses", "customer.postal_address"),
                ("preferences", "customer.preference"),
                ("wishlist", "customer.wishlist_item"),
            ] {
                if delete_now.iter().any(|d| d == data) {
                    sqlx::query(sqlx::AssertSqlSafe(format!(
                        "DELETE FROM {table} WHERE tenant_id = $1 AND customer_id = $2"
                    )))
                    .bind(tx.tenant.as_str())
                    .bind(&a.id)
                    .execute(&mut *tx.tx)
                    .await
                    .map_err(db)?;
                }
            }
            // Masked now: the record stays, without the person's contact data.
            let mask_now = words(tx, "data_handling.mask_now");
            let masked = |f: &str| mask_now.iter().any(|m| m == f);
            sqlx::query(
                "UPDATE customer.person SET member_state = 'withdrawn', withdrawn_at = $3,
                   name = CASE WHEN $4 THEN $5 ELSE name END,
                   given_name = CASE WHEN $4 THEN NULL ELSE given_name END,
                   family_name = CASE WHEN $4 THEN NULL ELSE family_name END,
                   email = CASE WHEN $6 THEN NULL ELSE email END,
                   telephone = CASE WHEN $7 THEN NULL ELSE telephone END,
                   birth_date = CASE WHEN $8 THEN NULL ELSE birth_date END,
                   login_id = NULL, password_hash = NULL, password_history = '{}',
                   email_verified_at = NULL, telephone_verified_at = NULL, sizes = '{}',
                   card_last4 = NULL, updated_at = now(), version = version + 1
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tx.tenant.as_str())
            .bind(&a.id)
            .bind(now)
            .bind(masked("name"))
            .bind(wording::mask_name(&a.name))
            .bind(masked("email"))
            .bind(masked("telephone"))
            .bind(masked("birth_date"))
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        } else {
            sqlx::query(
                "UPDATE customer.person SET member_state = 'pending_withdrawal', updated_at = now(),
                   version = version + 1 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tx.tenant.as_str())
            .bind(&a.id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        }
        let reasons: Vec<&str> = input.reasons.iter().map(|r| r.as_str()).collect();
        sqlx::query(
            "INSERT INTO customer.withdrawal (tenant_id, customer_id, reasons, reason_text,
               effective_at, identifier_fingerprints, rejoin_after, identifier_reusable_after,
               settled, data_retention)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (tenant_id, customer_id) DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&a.id)
        .bind(&reasons)
        .bind(
            input
                .reason_text
                .as_deref()
                .map(|t| t.chars().take(500).collect::<String>()),
        )
        .bind(now + Duration::days(cooling))
        .bind(&prints)
        .bind(rejoin_after)
        .bind(reusable_after)
        .bind(json!(settled))
        .bind(json!(data_retention))
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        let revoked = repo::revoke_sessions(tx, &a.id, None).await?;
        let notice = a
            .email
            .as_deref()
            .filter(|_| {
                tx.policy
                    .lookup("customer.withdrawal.on_withdraw.notify_member")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .map(|e| not_delivered("email", e));
        log_person(
            tx,
            Self::META.tool,
            ActionType::DELETE,
            &a.id,
            json!({"status": state, "reasons": reasons, "sessions_revoked": revoked}),
        )
        .await?;
        let effective = now + Duration::days(cooling);
        Ok(json!({
            "ok": true, "id": a.id, "status": state,
            "effective_at": tx.time.format(effective),
            "cancel_until": pending.then(|| tx.time.format(effective)),
            "settled": settled,
            "data_retention": data_retention,
            "rejoin_after": day(tx.time, rejoin_after),
            "login_id_reusable_after": reusable_after.map(|t| day(tx.time, t)),
            "sessions_revoked": revoked,
            "notice": notice,
        }))
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_is_person_only_and_closed() {
        let s = closed_schema_for::<WithdrawAccountInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(WithdrawAccount::META.person_scope, "human:account");
        assert_eq!(
            s["properties"]["reauthentication"]["properties"]["method"]["enum"],
            json!(["password", "email_code"])
        );
    }
}
