//! `person`: members and their accounts. Reads (`get_my_profile`, `get_return_history`,
//! `search_members`, `get_member`, `check_availability`, `check_withdrawal_eligibility`), the
//! operator writes (`register_member`, `update_member`, `block_member`, `unblock_member`,
//! `verify_identity`, `verify_business`, `send_password_reset`) and the member's own
//! person-only endpoints (withdrawal, password reset and change, sign-in id recovery). Personal
//! data is masked for operators unless unmasked with a reason; passwords never reach a tool.

use acom_contract::{ActionType, Basis, ToolError};
use acom_core::enums::ActionStatus;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use crate::access::db;
use crate::service::providers;
use crate::wording::mask_email;
use crate::{ACCESS_REASON_REQUIRED, CUSTOMER_NOT_FOUND, UNMASK_FORBIDDEN, repo, secret};

mod block_member;
mod change_password;
mod check_availability;
mod check_withdrawal_eligibility;
mod confirm_password_reset;
mod get_member;
mod get_my_profile;
mod get_return_history;
mod recover_login_id;
mod register_member;
mod request_password_reset;
mod search_members;
mod send_password_reset;
mod unblock_member;
mod update_member;
mod verify_business;
mod verify_identity;
mod withdraw_account;

pub use block_member::*;
pub use change_password::*;
pub use check_availability::*;
pub use check_withdrawal_eligibility::*;
pub use confirm_password_reset::*;
pub use get_member::*;
pub use get_my_profile::*;
pub use get_return_history::*;
pub use recover_login_id::*;
pub use register_member::*;
pub use request_password_reset::*;
pub use search_members::*;
pub use send_password_reset::*;
pub use unblock_member::*;
pub use update_member::*;
pub use verify_business::*;
pub use verify_identity::*;
pub use withdraw_account::*;

pub(crate) fn customer_not_found(id: &str) -> ToolError {
    ToolError::new(CUSTOMER_NOT_FOUND, format!("There is no customer {id}."))
        .detail(json!({"entity": "customer", "id": id}))
        .next("search_members")
}

/// A saved address as profiles and lists show it.
pub(crate) fn address_json(a: &repo::AddressRow) -> Value {
    json!({"id": a.id, "label": a.label, "street_address": a.street_address,
           "address_locality": a.address_locality, "address_region": a.address_region,
           "postal_code": a.postal_code, "address_country": a.address_country,
           "is_primary": a.is_primary})
}

/// A memory as the profile shows it.
fn memory_json(p: &repo::PreferenceRow) -> Value {
    json!({"memory_id": p.id, "key": p.preference_key, "value": p.value,
           "text": p.description, "source": p.source})
}

/// Checks an unmask request and writes it to the action log (privacy.access_reason_logging).
pub(crate) async fn record_unmask(
    cx: &Cx,
    tool: &'static str,
    unmask: bool,
    reason: Option<&str>,
    object: Option<&str>,
) -> Result<bool, ToolError> {
    if !unmask {
        return Ok(false);
    }
    let reason = reason
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| {
            ToolError::new(
                ACCESS_REASON_REQUIRED,
                "Say why you need the unmasked data; the reason goes to the action log.",
            )
            .detail(json!({"field": "access_reason", "reason": "missing"}))
            .next(tool)
        })?;
    use acom_core::enums::ActorKind;
    if !matches!(
        cx.principal.kind,
        ActorKind::Operator | ActorKind::AgentOnBehalf
    ) {
        return Err(ToolError::new(
            UNMASK_FORBIDDEN,
            "Only an operator may unmask customer data.",
        )
        .detail(json!({"reason": "role"}))
        .next("list_roles"));
    }
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    acom_changeset::log_action(
        &mut tx,
        acom_changeset::LogEntry {
            object: object.map(|id| ("person", id)),
            result: Some(json!({"unmasked": true, "access_reason": reason})),
            ..acom_changeset::LogEntry::new(ActionType::SEARCH.0, ActionStatus::Completed, tool)
        },
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(true)
}

// ── accounts ────────────────────────────────────────────────────────────────────────────

/// The account, or `customer_not_found`.
pub(crate) async fn account_or_not_found(
    tx: &mut TxCx,
    id: &str,
) -> Result<repo::AccountRow, ToolError> {
    repo::account(tx, id)
        .await?
        .ok_or_else(|| customer_not_found(id))
}

pub(crate) fn account_basis(a: &repo::AccountRow) -> Basis {
    Basis {
        entity: "customer.person".into(),
        id: a.id.clone(),
        version: i64::from(a.version),
    }
}

/// Whether an email, sign-in id or phone (normalized) is in use by a member who did not leave.
pub(crate) async fn identifier_taken(
    tx: &mut TxCx,
    field: &str,
    normalized: &str,
) -> Result<bool, ToolError> {
    let column = match field {
        "email" => "lower(email)",
        "login_id" => "lower(login_id)",
        _ => "regexp_replace(telephone, '[^0-9+]', '', 'g')",
    };
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT EXISTS (SELECT 1 FROM customer.person
         WHERE tenant_id = $1 AND member_state <> 'withdrawn' AND {column} = $2)"
    )))
    .bind(tx.tenant.as_str())
    .bind(normalized)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A withdrawal whose fingerprints include one of these identifiers.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct HeldRow {
    pub rejoin_after: DateTime<Utc>,
    pub identifier_reusable_after: Option<DateTime<Utc>>,
}

pub(crate) async fn withdrawals_matching(
    tx: &mut TxCx,
    normalized: &[String],
) -> Result<Vec<HeldRow>, ToolError> {
    let prints: Vec<String> = normalized.iter().map(|n| secret::hash(n)).collect();
    sqlx::query_as::<_, HeldRow>(
        "SELECT rejoin_after, identifier_reusable_after FROM customer.withdrawal
         WHERE tenant_id = $1 AND identifier_fingerprints && $2
         ORDER BY rejoin_after DESC",
    )
    .bind(tx.tenant.as_str())
    .bind(&prints)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Whether an identifier of a member who left is still held (`withdrawal.identifier_reuse`).
pub(crate) async fn held_identifier(
    tx: &mut TxCx,
    normalized: &str,
    now: DateTime<Utc>,
) -> Result<bool, ToolError> {
    let rows = withdrawals_matching(tx, &[normalized.to_owned()]).await?;
    Ok(rows
        .iter()
        .any(|r| r.identifier_reusable_after.is_none_or(|t| t > now)))
}

/// The entry tier (the lowest one without requirements) of every program, for new members.
pub(crate) async fn entry_tiers(tx: &mut TxCx) -> Result<Vec<(String, String)>, ToolError> {
    let tiers = repo::tiers(tx, None).await?;
    let mut out: Vec<(String, String, i32)> = Vec::new();
    for t in tiers {
        if !crate::progress::requirement_list(&t.has_tier_requirement).is_empty() {
            continue;
        }
        match out.iter_mut().find(|(p, _, _)| *p == t.member_program_id) {
            Some(e) if t.rank < e.2 => *e = (t.member_program_id, t.id, t.rank),
            Some(_) => {}
            None => out.push((t.member_program_id, t.id, t.rank)),
        }
    }
    Ok(out.into_iter().map(|(p, t, _)| (p, t)).collect())
}

/// A new membership number, unique in the store.
pub(crate) fn membership_number() -> String {
    format!("M-{}", acom_core::id::new_suffix().to_uppercase())
}

/// The name an operator sees: masked unless it is the member asking.
pub(crate) fn shown_contact(cx: &Cx, value: Option<&str>) -> Value {
    match (value, crate::access::caller(&cx.principal)) {
        (None, _) => Value::Null,
        (Some(v), crate::access::Caller::Shopper(_)) => json!(v),
        (Some(v), _) if v.contains('@') => json!(mask_email(v)),
        (Some(v), _) => json!(crate::verification::masked(v)),
    }
}

/// The member state before and after a block or unblock.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StatePlan {
    pub customer_id: String,
    pub before: repo::StateFields,
    pub after: repo::StateFields,
}

/// Shared by block and unblock: write the planned state and read it back.
pub(crate) async fn write_planned(
    tx: &mut TxCx,
    p: &StatePlan,
) -> Result<acom_contract::Committed, ToolError> {
    repo::write_state(tx, &p.customer_id, &p.after).await?;
    let mut produced = serde_json::Map::new();
    produced.insert("customer_id".into(), json!(p.customer_id));
    Ok(acom_contract::Committed {
        id: p.customer_id.clone(),
        produced,
    })
}

pub(crate) async fn verify_planned(
    cx: &Cx,
    p: &StatePlan,
) -> Result<acom_contract::Verification, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let a = repo::account(&mut tx, &p.customer_id).await?;
    tx.commit().await.map_err(ToolError::from)?;
    let mismatches = match a {
        Some(a) if a.member_state == p.after.member_state => vec![],
        Some(a) => vec![format!("{} is {}", a.id, a.member_state)],
        None => vec![format!("{} is gone", p.customer_id)],
    };
    Ok(acom_contract::Verification {
        matched: mismatches.is_empty(),
        mismatches,
    })
}

// ── withdrawal eligibility (check_withdrawal_eligibility, POST /v1/customer/me/withdrawal) ──

/// What stands in the way of leaving, what leaving does, and what stays.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Eligibility {
    pub blockers: Vec<Value>,
    /// Items the policy only warns about (`warn`): shown, not blocking.
    pub warnings: Vec<Value>,
    pub on_withdrawal: Vec<Value>,
    /// What could not be checked because the domain holding it is not connected here.
    pub unchecked: Vec<&'static str>,
}

/// The policy word for one key of `withdrawal.blockers`, `balances` or `on_withdraw`.
fn rule(tx: &TxCx, path: &str) -> String {
    tx.policy
        .lookup(&format!("customer.withdrawal.{path}"))
        .and_then(Value::as_str)
        .unwrap_or("ignore")
        .to_owned()
}

/// Works out the eligibility of a member now. Shared by the read and the person-only withdrawal,
/// which checks again when the member presses the button.
pub(crate) async fn eligibility(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Eligibility, ToolError> {
    let now = tx.clock.now();
    let time = tx.time;
    let mut blockers = Vec::new();
    let mut unchecked = Vec::new();
    let window = tx
        .policy
        .lookup("customer.withdrawal.blockers.claim_window_days")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let end_of_day = |t: DateTime<Utc>| {
        let day = t.with_timezone(&time.zone).date_naive();
        time.parse(&format!("{day}T23:59:00")).unwrap_or(t)
    };

    if rule(tx, "blockers.open_order") != "ignore" {
        match providers::member_orders() {
            Some(read) => {
                let orders = read(tx, customer_id, DateTime::<Utc>::UNIX_EPOCH).await?;
                for o in orders {
                    let clears_at = o
                        .delivered_at
                        .map(|d| end_of_day(d + Duration::days(window)));
                    if clears_at.is_some_and(|c| c <= now) {
                        continue;
                    }
                    let detail = match o.delivered_at {
                        None => format!("Order {} is not delivered yet", o.order_number),
                        Some(d) => format!(
                            "Order {} was delivered {}; returns stay open {window} days",
                            o.order_number,
                            d.with_timezone(&time.zone).format("%b %-d")
                        ),
                    };
                    blockers.push(json!({"type": "open_order", "id": o.order_id,
                        "detail": detail, "clears_at": clears_at.map(|c| time.format(c)),
                        "effect": rule(tx, "blockers.open_order"), "next_tool": "get_order"}));
                }
            }
            None => unchecked.push("open_order"),
        }
    }
    if rule(tx, "blockers.open_claim") != "ignore" {
        match providers::return_claims() {
            Some(read) => {
                let claims = read(tx, customer_id, DateTime::<Utc>::UNIX_EPOCH).await?;
                for c in claims.iter().filter(|c| {
                    !matches!(
                        c.status.as_str(),
                        "completed" | "declined" | "withdrawn" | "converted"
                    )
                }) {
                    blockers.push(json!({"type": "open_claim", "id": c.id,
                        "detail": format!("Return of order {} is {}", c.order_number, c.status.replace('_', " ")),
                        "clears_at": null, "effect": rule(tx, "blockers.open_claim"),
                        "next_tool": "get_claim"}));
                }
            }
            None => unchecked.push("open_claim"),
        }
    }
    if rule(tx, "blockers.open_inquiry") != "ignore" {
        let open: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM customer.customer_inquiry
             WHERE tenant_id = $1 AND customer_id = $2 AND inquiry_status <> 'resolved'
             ORDER BY date_received, id",
        )
        .bind(tx.tenant.as_str())
        .bind(customer_id)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        for (id,) in open {
            blockers.push(json!({"type": "open_inquiry", "id": id,
                "detail": "Inquiry still in progress", "clears_at": null,
                "effect": rule(tx, "blockers.open_inquiry"), "next_tool": "get_inquiry"}));
        }
    }
    if rule(tx, "blockers.pending_remedy") != "ignore" {
        for r in crate::remedy::open_of_member(tx, customer_id).await? {
            let a = r.amount();
            blockers.push(json!({"type": "pending_remedy", "id": r.id,
                "detail": format!("{} {} remedy is {}", a.value, a.currency, r.remedy_status.replace('_', " ")),
                "amount": a, "clears_at": null,
                "effect": rule(tx, "blockers.pending_remedy"), "next_tool": "approve_remedy"}));
        }
    }
    // Refunds, points and store credit live in fulfillment and promotion, which do not tell
    // customer about them; they are named as not checked rather than guessed.
    if rule(tx, "blockers.pending_refund") != "ignore" {
        unchecked.push("pending_refund");
    }
    for balance in ["loyalty_points", "store_credit"] {
        if rule(tx, &format!("balances.{balance}")) != "ignore" {
            unchecked.push(if balance == "loyalty_points" {
                "loyalty_points"
            } else {
                "store_credit"
            });
        }
    }
    let (mut blockers, mut warnings): (Vec<Value>, Vec<Value>) =
        blockers.into_iter().partition(|b| b["effect"] == "block");
    for b in blockers.iter_mut().chain(warnings.iter_mut()) {
        if let Some(o) = b.as_object_mut() {
            o.remove("effect");
        }
    }

    let mut on_withdrawal = Vec::new();
    let words = |tx: &TxCx, k: &str| rule(tx, &format!("on_withdraw.{k}"));
    let coupons = words(tx, "coupons");
    on_withdrawal
        .push(json!({"type": "coupons", "detail": "Unused coupons are voided", "action": coupons}));
    let subs = words(tx, "active_subscriptions");
    on_withdrawal.push(json!({"type": "active_subscriptions",
        "detail": "Subscriptions end after the current period", "action": subs}));
    let linked = words(tx, "linked_logins");
    on_withdrawal.push(json!({"type": "linked_logins",
        "detail": "Sign-in with an external account is unlinked", "action": linked}));
    if let Some(e) = crate::employee::of_member(tx, customer_id).await?
        && e.end_date.is_none()
    {
        on_withdrawal.push(json!({"type": "employee_benefits", "id": e.id,
            "detail": "The employee discount ends with the account", "action": words(tx, "employee_benefits")}));
    }
    let memberships = repo::memberships(tx, customer_id).await?;
    if let Some(m) = memberships.first() {
        let name = repo::tier(tx, &m.tier_id)
            .await?
            .map_or_else(|| m.tier_id.clone(), |t| t.name);
        on_withdrawal.push(json!({"type": "tier",
            "detail": format!("{name} status ends and does not carry over"), "action": words(tx, "tier")}));
    }
    Ok(Eligibility {
        blockers,
        warnings,
        on_withdrawal,
        unchecked,
    })
}

/// `withdrawal.data_handling` as the eligibility answer shows it.
pub(crate) fn data_handling(tx_policy: &acom_policy::PolicySet) -> Value {
    let dh = tx_policy
        .lookup("customer.withdrawal.data_handling")
        .cloned()
        .unwrap_or(Value::Null);
    json!({"deleted_now": dh.get("delete_now"), "masked_now": dh.get("mask_now"),
           "authored_content": dh.get("authored_content"), "retained": dh.get("retain")})
}

// ── passwords (person-only endpoints only) ──────────────────────────────────────────────

/// Marks a person-only error as a failed attempt to keep: the endpoint's transaction is kept
/// (acom-http), so the wrong code or password still counts towards the lockout.
pub(crate) fn attempt_recorded(mut e: ToolError) -> ToolError {
    let mut d = e.detail.take().map(|b| *b).unwrap_or_else(|| json!({}));
    if let Some(o) = d.as_object_mut() {
        o.insert("attempt_recorded".into(), json!(true));
    }
    e.detail(d)
}

/// The store's password rules, or an internal error when the policy has none.
pub(crate) fn password_rules(tx: &TxCx) -> Result<crate::password::Rules, ToolError> {
    tx.policy
        .lookup("customer.password")
        .and_then(crate::password::Rules::from_policy)
        .ok_or_else(|| {
            ToolError::new(
                acom_contract::ErrorCode::INTERNAL,
                "The policy value `customer.password` is missing.",
            )
        })
}

/// Sets a new password after checking it against the rules and the recent ones; clears the
/// lockout (`password.lockout.unlock_by`).
pub(crate) async fn set_password(
    tx: &mut TxCx,
    a: &repo::AccountRow,
    new_password: &str,
) -> Result<(), ToolError> {
    let rules = password_rules(tx)?;
    let ids: Vec<&str> = [a.email.as_deref(), a.login_id.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    let broken = rules.broken(new_password, &ids);
    if !broken.is_empty() {
        return Err(ToolError::new(
            crate::PASSWORD_POLICY,
            "The new password does not meet the store's password rules.",
        )
        .detail(json!({"rules": broken, "policy_key": "password"})));
    }
    if crate::password::reused(
        new_password,
        a.password_hash.as_deref(),
        &a.password_history,
        rules.history_block,
    ) {
        return Err(ToolError::new(
            crate::PASSWORD_REUSED,
            format!(
                "Choose a password different from your last {}.",
                rules.history_block
            ),
        )
        .detail(json!({"policy_key": "password.history_block"})));
    }
    let hash = acom_auth::password::hash(new_password).map_err(|e| {
        ToolError::new(
            acom_contract::ErrorCode::INTERNAL,
            format!("hashing failed: {e}"),
        )
    })?;
    let history = crate::password::next_history(
        a.password_hash.as_deref(),
        &a.password_history,
        rules.history_block,
    );
    sqlx::query(
        "UPDATE customer.person SET password_hash = $3, password_history = $4,
           password_changed_at = $5, failed_password_attempts = 0,
           first_failed_password_at = NULL, locked_until = NULL, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(&a.id)
    .bind(hash)
    .bind(&history)
    .bind(tx.clock.now())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The lock on password checks, when there is one now.
pub(crate) fn locked_until(tx: &TxCx, a: &repo::AccountRow) -> Option<DateTime<Utc>> {
    a.locked_until.filter(|t| *t > tx.clock.now())
}

/// Checks the member's current password. A wrong one counts towards `password.lockout`; the
/// caller returns its error with [`attempt_recorded`] so the count is kept. `Ok(true)` when it
/// matches.
pub(crate) async fn check_password(
    tx: &mut TxCx,
    a: &repo::AccountRow,
    password: &str,
) -> Result<bool, ToolError> {
    let ok = a
        .password_hash
        .as_deref()
        .is_some_and(|h| acom_auth::password::verify(password, h));
    if ok {
        if a.failed_password_attempts > 0 {
            sqlx::query(
                "UPDATE customer.person SET failed_password_attempts = 0,
                   first_failed_password_at = NULL, updated_at = now(), version = version + 1
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tx.tenant.as_str())
            .bind(&a.id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        }
        return Ok(true);
    }
    let lockout = tx
        .policy
        .lookup("customer.password.lockout")
        .and_then(crate::password::Lockout::from_policy);
    if let Some(l) = lockout {
        let (count, first, locked) = l.after_failure(
            a.failed_password_attempts,
            a.first_failed_password_at,
            tx.clock.now(),
        );
        sqlx::query(
            "UPDATE customer.person SET failed_password_attempts = $3,
               first_failed_password_at = $4, locked_until = COALESCE($5, locked_until),
               updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&a.id)
        .bind(count)
        .bind(first)
        .bind(locked)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    Ok(false)
}

/// Writes a person-only action to the action log, with no secret in it.
pub(crate) async fn log_person(
    tx: &mut TxCx,
    tool: &str,
    action: ActionType,
    object: &str,
    result: Value,
) -> Result<(), ToolError> {
    acom_changeset::log_action(
        tx,
        acom_changeset::LogEntry {
            object: Some(("person", object)),
            result: Some(result),
            ..acom_changeset::LogEntry::new(action.0, ActionStatus::Completed, tool)
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reads_people_from_the_database() {
        let mut tx = crate::testdb::tx("cus_jamie").await;
        let jamie = repo::person(&mut tx, "cus_jamie").await.unwrap().unwrap();
        assert_eq!(jamie.card_last4.as_deref(), Some("4821"));
        let (total, rows) = repo::search_members(
            &mut tx,
            &repo::MemberFilter {
                query: Some("alex".into()),
                ..Default::default()
            },
            20,
            0,
        )
        .await
        .unwrap();
        assert!(total >= 1);
        assert!(rows.iter().any(|r| r.id == "cus_alex"));
    }

    /// Integration: a sign-up from a code check to leaving the store, in one transaction that is
    /// rolled back. Needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn signs_up_sets_a_password_and_leaves() {
        use acom_contract::{PersonOp, WriteOp};
        use acom_core::enums::{ConsentType, MemberType, WithdrawalReason};

        let guest = crate::testdb::cx("ses_dev_guest").await;
        let mut tx = guest.begin().await.unwrap();

        // 1. Send a code (the code itself never leaves the commit), then confirm it.
        let send = VerifyIdentity
            .preview(
                &guest,
                VerifyIdentityInput {
                    purpose: VerifyPurpose::Signup,
                    method: "email_code".into(),
                    destination: Some("Robin.Test@Example.com".into()),
                    subject: None,
                    code: None,
                },
            )
            .await
            .unwrap();
        let sent = VerifyIdentity.commit(&mut tx, &send.plan).await.unwrap();
        assert_eq!(sent.produced["delivery"]["delivered"], false);
        assert!(!sent.produced.contains_key("code"));
        // The test knows the code only by setting its hash.
        sqlx::query("UPDATE customer.identity_verification SET code_hash = $2 WHERE id = $1")
            .bind(&sent.id)
            .bind(secret::hash("246810"))
            .execute(&mut *tx.tx)
            .await
            .unwrap();
        let confirm = VerifyIdentityPlan {
            step: Step::ConfirmCode,
            check: crate::verification::NewCheck {
                id: sent.id.clone(),
                ..send.plan.check.clone()
            },
            token_ttl_seconds: 300,
        };
        let verified = VerifyIdentity.commit(&mut tx, &confirm).await.unwrap();
        let token = verified.produced["verification_token"]
            .as_str()
            .unwrap()
            .to_owned();
        let c = crate::verification::usable(
            &mut tx,
            &token,
            "signup",
            Some("robin.test@example.com"),
            "x",
        )
        .await
        .unwrap();
        assert_eq!(c.action_status, "completed");

        // 2. Register with the token and the mandatory consents.
        let entries = entry_tiers(&mut tx).await.unwrap();
        assert_eq!(entries, [("prg_rewards".to_owned(), "member".to_owned())]);
        let plan = RegisterPlan {
            customer_id: "cus_robin_test".into(),
            member_type: MemberType::Individual.as_str().into(),
            name: "Robin Test".into(),
            email: "robin.test@example.com".into(),
            telephone: None,
            legal_name: None,
            tax_id: None,
            verification_id: token.clone(),
            verified_email_at: c.verified_at,
            verified_telephone_at: None,
            memberships: vec![("prg_rewards".into(), "member".into(), membership_number())],
            consents: vec![
                (
                    crate::consent::new_id(),
                    ConsentType::Terms.as_str().into(),
                    true,
                ),
                (
                    crate::consent::new_id(),
                    ConsentType::Privacy.as_str().into(),
                    true,
                ),
            ],
            setup_request_id: "rcv_robin_setup".into(),
            setup_method: Some("email_link".into()),
            setup_channel: "email".into(),
            setup_ttl_seconds: 1800,
        };
        let made = RegisterMember.commit(&mut tx, &plan).await.unwrap();
        assert_eq!(made.produced["password_setup"]["delivered"], false);
        let e = crate::verification::usable(&mut tx, &token, "signup", None, "x")
            .await
            .unwrap_err();
        assert_eq!(e.code, "identity_not_verified");
        assert!(
            identifier_taken(&mut tx, "email", "robin.test@example.com")
                .await
                .unwrap()
        );

        // 3. Set the first password with the set-up link (the test sets the token's hash).
        sqlx::query("UPDATE customer.recovery_request SET token_hash = $2 WHERE id = $1")
            .bind("rcv_robin_setup")
            .bind(secret::hash("setup-token-for-robin-0001"))
            .execute(&mut *tx.tx)
            .await
            .unwrap();
        let weak = ConfirmPasswordReset
            .run(
                &mut tx,
                ConfirmPasswordResetInput {
                    token: Some("setup-token-for-robin-0001".into()),
                    request_id: None,
                    code: None,
                    new_password: "abc12".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(weak.code, "password_policy");
        let set = ConfirmPasswordReset
            .run(
                &mut tx,
                ConfirmPasswordResetInput {
                    token: Some("setup-token-for-robin-0001".into()),
                    request_id: None,
                    code: None,
                    new_password: "harbor-lantern-42".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(set["next"], "sign_in");
        let again = ConfirmPasswordReset
            .run(
                &mut tx,
                ConfirmPasswordResetInput {
                    token: Some("setup-token-for-robin-0001".into()),
                    request_id: None,
                    code: None,
                    new_password: "harbor-lantern-43".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(again.code, "invalid_or_expired");

        // 4. As the member: a wrong current password is counted; the right one changes it.
        tx.principal = crate::testdb::principal("cus_robin_test");
        let wrong = ChangePassword
            .run(
                &mut tx,
                ChangePasswordInput {
                    current_password: Some("not-it-99".into()),
                    verification_token: None,
                    new_password: "quiet-meadow-81".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(wrong.code, "current_password_wrong");
        assert_eq!(wrong.detail.unwrap()["attempt_recorded"], true);
        let reused = ChangePassword
            .run(
                &mut tx,
                ChangePasswordInput {
                    current_password: Some("harbor-lantern-42".into()),
                    verification_token: None,
                    new_password: "harbor-lantern-42".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(reused.code, "password_reused");
        ChangePassword
            .run(
                &mut tx,
                ChangePasswordInput {
                    current_password: Some("harbor-lantern-42".into()),
                    verification_token: None,
                    new_password: "quiet-meadow-81".into(),
                },
            )
            .await
            .unwrap();

        // 5. Leave the store: nothing blocks a new member.
        let e = eligibility(&mut tx, "cus_robin_test").await.unwrap();
        assert!(e.blockers.is_empty(), "{:?}", e.blockers);
        let left = WithdrawAccount
            .run(
                &mut tx,
                WithdrawAccountInput {
                    reasons: vec![WithdrawalReason::NotUsing],
                    reason_text: None,
                    confirmation: WithdrawalConfirmation {
                        acknowledged_data_handling: true,
                        eligibility_checked_at: "2026-11-07T09:55".into(),
                    },
                    reauthentication: Reauthentication {
                        method: ReauthMethod::Password,
                        credential: Some("quiet-meadow-81".into()),
                        verification_token: None,
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(left["status"], "withdrawn");
        let a = repo::account(&mut tx, "cus_robin_test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(a.member_state, "withdrawn");
        assert_eq!(a.email, None);
        assert_eq!(a.password_hash, None);
        assert!(
            repo::memberships(&mut tx, "cus_robin_test")
                .await
                .unwrap()
                .is_empty()
        );
        let now = tx.clock.now();
        assert!(
            held_identifier(&mut tx, "robin.test@example.com", now)
                .await
                .unwrap()
        );

        // 6. The anonymous reset request answers the same with or without an account.
        tx.principal = crate::testdb::principal("ses_dev_guest");
        let known = RequestPasswordReset
            .run(
                &mut tx,
                RequestPasswordResetInput {
                    identifier: "alex.kim@example.com".into(),
                    method: None,
                    client_challenge: None,
                },
            )
            .await
            .unwrap();
        let unknown = RequestPasswordReset
            .run(
                &mut tx,
                RequestPasswordResetInput {
                    identifier: "nobody.here@example.com".into(),
                    method: None,
                    client_challenge: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(known["message"], unknown["message"]);
        assert_eq!(known["status"], unknown["status"]);
        // Rolled back on drop.
    }

    /// Integration: operator writes on members, previewed and run in a rolled-back transaction.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn blocks_verifies_and_checks_eligibility() {
        use acom_contract::{ReadOp, WriteOp};

        let cx = crate::testdb::cx("adm_sj").await;
        let block = BlockMember
            .preview(
                &cx,
                BlockMemberInput {
                    customer_id: "cus_sam".into(),
                    reason: "Repeated chargebacks.".into(),
                    block_until: Some("2026-12-31T23:59".into()),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            block.approval,
            acom_contract::ApprovalNeed::Needed { .. }
        ));
        let mut tx = cx.begin().await.unwrap();
        let done = BlockMember.commit(&mut tx, &block.plan).await.unwrap();
        assert_eq!(
            repo::account(&mut tx, "cus_sam")
                .await
                .unwrap()
                .unwrap()
                .member_state,
            "blocked"
        );
        BlockMember.undo(&mut tx, &block.plan, &done).await.unwrap();
        assert_eq!(
            repo::account(&mut tx, "cus_sam")
                .await
                .unwrap()
                .unwrap()
                .member_state,
            "active"
        );
        drop(tx);

        let wrong = VerifyBusiness
            .preview(
                &cx,
                VerifyBusinessInput {
                    customer_id: "cus_biz_31".into(),
                    tax_id: "99-0000000".into(),
                    legal_name: "Harbor Supply Co.".into(),
                    registration_document: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(wrong.code, "registry_mismatch");
        let ok = VerifyBusiness
            .preview(
                &cx,
                VerifyBusinessInput {
                    customer_id: "cus_biz_31".into(),
                    tax_id: "123456789".into(),
                    legal_name: "harbor supply co".into(),
                    registration_document: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(ok.changes[0].to["acom:business_verification"], "verified");

        let reset = SendPasswordReset
            .preview(
                &cx,
                SendPasswordResetInput {
                    customer_id: "cus_alex".into(),
                    method: None,
                    reason: "Locked out, asked by phone.".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(reset.plan.to, "alex.kim@example.com");
        let none = SendPasswordReset
            .preview(
                &cx,
                SendPasswordResetInput {
                    customer_id: "cus_sam".into(),
                    method: None,
                    reason: "Locked out.".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(none.code, "no_verified_contact");

        let alex = CheckWithdrawalEligibility
            .read(
                &cx,
                CheckWithdrawalEligibilityInput {
                    customer_id: Some("cus_alex".into()),
                    fields: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(alex["eligible"], false);
        assert!(
            alex["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["id"] == "inq_771")
        );
        let taken = CheckAvailability
            .read(
                &cx,
                CheckAvailabilityInput {
                    field: IdentifierField::Email,
                    value: "Alex.Kim@example.com".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(taken["reason"], "taken");
    }
}
