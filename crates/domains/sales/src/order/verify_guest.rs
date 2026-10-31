//! `verify_guest` (`sales.order.verify_guest`): someone who is not signed in opens one order by
//! its number and the email or phone it was placed with, and gets a short-lived token that
//! opens only that order (`order.guest_lookup`).
//!
//! A wrong try gets the same answer whether or not an order has that number, and is counted on
//! the number either way, so the answer never tells whether an order exists. The preview checks
//! the contact and shows the token it would hand out; wrong tries are counted as they happen
//! (like a wrong password at `login`): that count is the guard against guessing. The run hands
//! the token out once (`shown_once`) and keeps only its hash.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_customer::service::for_sales_guest as customer;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::repo::{self, GuestLookupRow};
use crate::rules::{self, GuestLookupRules};
use crate::{GUEST_MATCH_FAILED, TOO_MANY_ATTEMPTS, approval};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyGuestInput {
    /// The order number on the confirmation, with or without `#`.
    #[schemars(regex(pattern = r"^#?[0-9]{1,12}$"), example = &"20402")]
    pub order_number: String,
    /// The email the order was placed with. Give this or `telephone`.
    #[schemars(length(min = 3, max = 200), example = &"alex.kim@example.com")]
    pub email: Option<String>,
    /// The phone number the order was placed with; only its digits are compared. Give this or
    /// `email`.
    #[schemars(length(min = 7, max = 30), example = &"+1 207 555 0137")]
    pub telephone: Option<String>,
}

/// What `verify_guest` hands out: a token for one order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GuestPlan {
    order_number: String,
    order_id: String,
    ttl_seconds: u32,
}

pub(crate) struct VerifyGuest;

impl Operation for VerifyGuest {
    const META: OpMeta = OpMeta {
        address: "sales.order.verify_guest",
        tool: "verify_guest",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/sales/orders/verify-guest",
        summary: "Let someone who is not signed in open one order by its number and the email or phone it was placed with: a short-lived token for that order only. A wrong pair gets the same answer whether or not the order exists; too many wrong tries lock the number for a while.",
        consumes: &[
            Slot::one("order_number"),
            Slot::one("email"),
            Slot::one("telephone"),
        ],
        produces: &[Slot::one("guest_token")],
        checks: &[
            "guest_lookup.max_attempts",
            "guest_lookup.lock_minutes",
            "guest_lookup.token_ttl_seconds",
        ],
        errors: &[GUEST_MATCH_FAILED, TOO_MANY_ATTEMPTS],
        audience: Audience::Shopper,
        action_type: ActionType::AUTHORIZE,
        person_scope: "",
    };
    type Input = VerifyGuestInput;
    type Output = Value;
}

fn locked(cx: &Cx, until: DateTime<Utc>) -> ToolError {
    ToolError::new(
        TOO_MANY_ATTEMPTS,
        format!(
            "Too many wrong tries on this order number; try again after {}.",
            cx.time.format(until)
        ),
    )
    .detail(json!({"until": cx.time.format(until), "policy_key": "guest_lookup.lock_minutes"}))
}

fn no_match(attempts_left: u32) -> ToolError {
    ToolError::new(
        GUEST_MATCH_FAILED,
        "The order number and the email or phone do not match an order; check both and try again.",
    )
    .detail(json!({"attempts_left": attempts_left, "policy_key": "guest_lookup.max_attempts"}))
    .next("verify_guest")
}

/// The wrong tries counted after one more at `now`: tries older than the lock period start the
/// count over. Returns the count, when counting started, and the lock it trips, if any.
pub(crate) fn count_wrong_try(
    previous: Option<&GuestLookupRow>,
    rules: GuestLookupRules,
    now: DateTime<Utc>,
) -> (u32, DateTime<Utc>, Option<DateTime<Utc>>) {
    let window = Duration::minutes(i64::from(rules.lock_minutes));
    let (count, first) = match previous.and_then(|p| p.first_failed_at.map(|f| (p, f))) {
        Some((p, first)) if now - first <= window && p.locked_until.is_none_or(|u| u > now) => (
            u32::try_from(p.failed_attempt_count).unwrap_or(0) + 1,
            first,
        ),
        _ => (1, now),
    };
    let lock = (count >= rules.max_attempts).then(|| now + window);
    (count, first, lock)
}

/// Counts one wrong try in its own transaction: the try is recorded even though the preview
/// fails. Returns the lock it trips, if any.
async fn record_wrong_try(
    cx: &Cx,
    number: &str,
    previous: Option<&GuestLookupRow>,
    rules: GuestLookupRules,
) -> Result<(u32, Option<DateTime<Utc>>), ToolError> {
    let (count, first, lock) = count_wrong_try(previous, rules, cx.clock.now());
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    repo::record_guest_failure(&mut tx, number, count, first, lock).await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok((count, lock))
}

fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A new token: `gt_` and 64 hex characters from the operating system's secure generator.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("gt_{hex}")
}

impl WriteOp<Cx> for VerifyGuest {
    type Plan = GuestPlan;
    // A token once shown cannot be taken back; it stops working after its time.
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: VerifyGuestInput,
    ) -> Result<Planned<GuestPlan>, ToolError> {
        let number = input.order_number.trim().trim_start_matches('#').to_owned();
        if input.email.is_none() && input.telephone.is_none() {
            return Err(ToolError::invalid_input(
                "Give the email or the phone number the order was placed with.",
            )
            .detail(json!({"field": "email", "reason": "missing"}))
            .next("verify_guest"));
        }
        let rules = rules::guest_lookup_rules(&cx.policy)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let previous = repo::guest_lookup(&mut tx, &number).await?;
        if let Some(until) = previous
            .as_ref()
            .and_then(|p| p.locked_until)
            .filter(|u| *u > now)
        {
            return Err(locked(cx, until));
        }
        let order = repo::order_by_number(&mut tx, &number).await?;
        let matched = match order.as_ref().and_then(|o| o.customer_id.as_deref()) {
            Some(c) => {
                customer::contact_matches(
                    &mut tx,
                    c,
                    input.email.as_deref(),
                    input.telephone.as_deref(),
                )
                .await?
            }
            // An order without a shopper recorded no contact to match.
            None => false,
        };
        tx.commit().await.map_err(ToolError::from)?;
        let Some(order) = order.filter(|_| matched) else {
            let (count, lock) = record_wrong_try(cx, &number, previous.as_ref(), rules).await?;
            return Err(match lock {
                Some(until) => locked(cx, until),
                None => no_match(rules.max_attempts.saturating_sub(count)),
            });
        };
        let wrong_before = previous
            .as_ref()
            .filter(|p| p.locked_until.is_none_or(|u| u > now))
            .map_or(0, |p| p.failed_attempt_count);
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "guest_token".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"order_id": order.id, "order_number": order.order_number,
                           "expires_in": rules.token_ttl_seconds}),
                note: Some("Opens this one order only; shown once".into()),
            }],
            checks: vec![Check {
                rule: "guest_lookup.max_attempts".into(),
                passed: true,
                detail: Some(format!(
                    "{wrong_before} wrong tries before this one; {} lock the number",
                    rules.max_attempts
                )),
                fix_tool: None,
            }],
            basis: Vec::new(),
            approval: approval(cx, Self::META.tool, &json!({})),
            plan: GuestPlan {
                order_number: order.order_number,
                order_id: order.id,
                ttl_seconds: rules.token_ttl_seconds,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &GuestPlan) -> Result<Committed, ToolError> {
        let token = new_token();
        let expires = tx.clock.now() + Duration::seconds(i64::from(plan.ttl_seconds));
        repo::grant_guest_token(
            tx,
            &plan.order_number,
            &plan.order_id,
            &token_hash(&token),
            expires,
        )
        .await?;
        let mut produced = Map::new();
        produced.insert("order_id".into(), json!(plan.order_id));
        produced.insert("order_number".into(), json!(plan.order_number));
        produced.insert("expires_in".into(), json!(plan.ttl_seconds));
        produced.insert(
            acom_changeset::SHOWN_ONCE.into(),
            json!({"guest_token": token}),
        );
        Ok(Committed {
            id: plan.order_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &GuestPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::guest_lookup(&mut tx, &plan.order_number).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = row.is_some_and(|r| {
            r.order_id.as_deref() == Some(plan.order_id.as_str()) && r.token_expires_at.is_some()
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                vec![]
            } else {
                vec![format!("no token for order {}", plan.order_id)]
            },
        })
    }

    async fn undo(&self, _: &mut TxCx, _: &GuestPlan, _: &Committed) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            "A token once handed out cannot be taken back; it stops working when it expires.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<VerifyGuestInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["order_number"]));
    }

    fn rules() -> GuestLookupRules {
        crate::rules::guest_lookup_rules(&crate::rules::tests::defaults()).unwrap()
    }

    fn at(minute: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_794_000_000, 0).unwrap() + Duration::minutes(minute)
    }

    #[test]
    fn the_fifth_wrong_try_locks_the_number_for_fifteen_minutes() {
        let r = rules();
        let mut row: Option<GuestLookupRow> = None;
        for n in 1..=5 {
            let (count, first, lock) = count_wrong_try(row.as_ref(), r, at(n));
            assert_eq!(count, u32::try_from(n).unwrap());
            assert_eq!(first, at(1));
            assert_eq!(lock.is_some(), n == 5, "try {n}");
            row = Some(GuestLookupRow {
                failed_attempt_count: i32::try_from(count).unwrap(),
                first_failed_at: Some(first),
                locked_until: lock,
                order_id: None,
                token_expires_at: None,
            });
        }
        assert_eq!(row.as_ref().unwrap().locked_until, Some(at(20)));
        // Once the lock has passed, counting starts over.
        let (count, first, lock) = count_wrong_try(row.as_ref(), r, at(21));
        assert_eq!((count, first, lock), (1, at(21), None));
    }

    #[test]
    fn tokens_are_random_and_kept_as_hashes() {
        let (a, b) = (new_token(), new_token());
        assert_ne!(a, b);
        assert!(a.starts_with("gt_") && a.len() == 67);
        assert_eq!(token_hash(&a).len(), 64);
        assert_ne!(token_hash(&a), a);
    }

    #[tokio::test]
    async fn a_contact_is_needed() {
        let cx = crate::testdb::offline("ses_guest");
        let e = VerifyGuest
            .preview(
                &cx,
                serde_json::from_value(json!({"order_number": "#20402"})).unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["field"], "email");
    }

    /// Integration (`DATABASE_URL`, demo seed): Alex opens #20402 with his email without
    /// signing in; a wrong email and an order number nobody has get the same answer. The tries
    /// are rolled back afterwards.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn a_guest_opens_an_order_with_its_email_and_learns_nothing_from_a_wrong_one() {
        let guest = crate::testdb::cx("ses_guest_lookup").await;
        let right = VerifyGuest
            .preview(
                &guest,
                serde_json::from_value(
                    json!({"order_number": "#20402", "email": " Alex.Kim@example.com "}),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(right.plan.order_id, "ord_20402");
        assert_eq!(right.changes[0].to["expires_in"], 1800);
        let mut tx = guest.begin().await.unwrap();
        let done = VerifyGuest.commit(&mut tx, &right.plan).await.unwrap();
        assert!(
            done.produced["shown_once"]["guest_token"]
                .as_str()
                .unwrap()
                .starts_with("gt_")
        );
        drop(tx); // rolled back

        let wrong = |number: &str| {
            serde_json::from_value::<VerifyGuestInput>(
                json!({"order_number": number, "email": "someone@example.com"}),
            )
            .unwrap()
        };
        let numbers = ["20402", "99999999"];
        let mut tx = guest.read().await.unwrap();
        let mut before = Vec::new();
        for n in numbers {
            before.push(repo::guest_lookup(&mut tx, n).await.unwrap().is_some());
        }
        tx.commit().await.unwrap();
        let e1 = VerifyGuest.preview(&guest, wrong(numbers[0])).await;
        let e2 = VerifyGuest.preview(&guest, wrong(numbers[1])).await;
        // Leave the store as it was: the wrong tries are not kept.
        let mut tx = guest.begin().await.unwrap();
        for (n, existed) in numbers.iter().zip(before) {
            let sql = if existed {
                "UPDATE sales.guest_lookup SET failed_attempt_count = 0, first_failed_at = NULL,
                   locked_until = NULL WHERE tenant_id = $1 AND order_number = $2"
            } else {
                "DELETE FROM sales.guest_lookup WHERE tenant_id = $1 AND order_number = $2"
            };
            sqlx::query(sql)
                .bind("store_northwind")
                .bind(n)
                .execute(&mut *tx.tx)
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
        let (e1, e2) = (e1.unwrap_err(), e2.unwrap_err());
        assert_eq!(e1.code, "guest_match_failed");
        assert_eq!(
            (e1.code.clone(), e1.message.clone(), e1.detail.clone()),
            (e2.code, e2.message, e2.detail),
            "the same answer whether or not the order exists"
        );
    }
}
