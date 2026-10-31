//! The `subscription` resource (`docs/api/sales.md` §subscription): standing orders.
//! `prepare_subscription` prepares one from the caller's cart (preview only), the shopper starts
//! it with the person-only `POST /v1/sales/subscriptions` (`start_subscription`), and
//! `pause_subscription`, `resume_subscription`, `cancel_subscription`, `update_schedule` and
//! `list_subscriptions` look after it. On each charge date the job `subscription.renew`
//! ([`renew`]) places the order and charges the saved card.
//!
//! Dates are store-time days from the store `Clock`; how far apart two orders are comes from
//! `order.subscription.cycle_length`, and which discounts a subscription gets from
//! `order.subscription.discount_layers`.

pub mod cancel_subscription;
pub mod list_subscriptions;
pub mod pause_subscription;
pub mod prepare_subscription;
pub mod renew;
pub mod resume_subscription;
pub mod start_subscription;
pub mod update_schedule;

use acom_catalog::service::for_sales as catalog;
use acom_contract::{Basis, Change, ChangeOp, Committed, ErrorCode, ToolError, Verification};
use acom_core::{Money, StoreTime};
use acom_customer::service::for_sales as customer;
use acom_promotion::service::for_sales::{self as promotion, Breakdown};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::checkout::payment_label;
use crate::owner::{self, Viewer};
use crate::pricing::{self, PricedLine};
use crate::repo::{self, SubscriptionRow, SubscriptionState};
use crate::{not_signed_in, rules};

/// The job that places a subscription's order on its charge date.
pub(crate) const RENEW_JOB: &str = "subscription.renew";

/// One line of a subscription: what every order of it repeats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SubscriptionLine {
    pub variant_id: String,
    pub product_id: String,
    pub name: String,
    pub size: Option<String>,
    pub color: Option<String>,
    pub quantity: u32,
    /// List price per unit when the subscription was priced.
    pub price: Money,
    /// The whole line's discount from the subscription's discount layers.
    pub discount: Money,
}

pub(crate) fn lines_of(r: &SubscriptionRow) -> Result<Vec<SubscriptionLine>, ToolError> {
    serde_json::from_value(r.ordered_item.clone()).map_err(|e| {
        ToolError::new(
            ErrorCode::INTERNAL,
            format!("subscription {} lines: {e}", r.id),
        )
    })
}

/// The lines and what each order comes to, from priced lines and their benefit calculation,
/// keeping only the discount layers subscriptions get (`subscription.discount_layers`): the
/// subtotal, less those discounts, plus shipping.
pub(crate) fn priced(
    lines: &[PricedLine],
    b: &Breakdown,
    layers: &[String],
) -> (Vec<SubscriptionLine>, Money) {
    let has = |layer: &str| layers.iter().any(|l| l == layer);
    let zero = promotion::money(Decimal::ZERO, b.currency);
    let mut out = Vec::new();
    let mut total = b.shipping.value;
    for l in lines {
        let r = b.lines.iter().find(|r| r.cart_line_id == l.line_id);
        let mut discount = Decimal::ZERO;
        if let Some(r) = r {
            if has("item_price") {
                discount += r.item_discount.value;
            }
            if has("order_discount") {
                discount += r.order_discount.value;
            }
        }
        let price = promotion::money(l.variant.list_price.value, b.currency);
        total += price.value * Decimal::from(l.quantity) - discount;
        out.push(SubscriptionLine {
            variant_id: l.variant.variant_id.clone(),
            product_id: l.variant.product_id.clone(),
            name: l.variant.product_name.clone(),
            size: l.variant.size.clone(),
            color: l.variant.color.clone(),
            quantity: l.quantity,
            price,
            discount: if discount.is_zero() {
                zero
            } else {
                promotion::money(discount, b.currency)
            },
        });
    }
    (out, promotion::money(total, b.currency))
}

/// Prices a subscription's lines again at today's prices and the shopper's tier (a changed
/// quantity, an order of it).
pub(crate) async fn reprice(
    tx: &mut TxCx,
    customer_id: &str,
    lines: &[SubscriptionLine],
) -> Result<(Vec<SubscriptionLine>, Money), ToolError> {
    let layers = rules::subscription_rules(&tx.policy)?.discount_layers;
    let ids: Vec<String> = lines.iter().map(|l| l.variant_id.clone()).collect();
    let facts = catalog::variants(tx, &ids).await?;
    let mut priced_lines = Vec::new();
    for (n, l) in lines.iter().enumerate() {
        let variant = facts
            .iter()
            .find(|v| v.variant_id == l.variant_id)
            .cloned()
            .ok_or_else(|| {
                ToolError::new(
                    ErrorCode::INVALID_STATE,
                    format!("{} is no longer sold.", l.name),
                )
                .detail(json!({"entity": "variant", "id": l.variant_id}))
                .next("cancel_subscription")
            })?;
        priced_lines.push(PricedLine {
            line_id: format!("line_{}", n + 1),
            variant,
            quantity: l.quantity,
            note: None,
        });
    }
    let tier_id = customer::member_tier(tx, customer_id).await?;
    let b = promotion::calculate(
        tx,
        &promotion::Request {
            customer_id: Some(customer_id.to_owned()),
            tier_id,
            lines: pricing::benefit_lines(&priced_lines),
            coupon_ids: Vec::new(),
            spend_loyalty: None,
        },
    )
    .await?;
    Ok(priced(&priced_lines, &b, &layers))
}

/// A subscription as people read it.
pub(crate) fn subscription_json(r: &SubscriptionRow, time: &StoreTime) -> Result<Value, ToolError> {
    let lines = lines_of(r)?;
    Ok(json!({
        "id": r.id,
        "customer_id": r.customer_id,
        "status": r.subscription_status,
        "billing_cycle": r.billing_cycle,
        "next_charge_date": r.next_charge_date.format("%Y-%m-%d").to_string(),
        "amount": repo::money(r.total_payment_due_value, &r.total_payment_due_currency)?,
        "lines": lines.iter().map(|l| json!({
            "variant_id": l.variant_id, "product_id": l.product_id, "name": l.name,
            "size": l.size, "color": l.color, "quantity": l.quantity,
        })).collect::<Vec<_>>(),
        "delivery": {
            "address_id": r.order_delivery.get("address_id").cloned().unwrap_or(Value::Null),
            "address_label": r.order_delivery.get("address_label").cloned().unwrap_or(Value::Null),
        },
        "payment": {
            "payment_method": r.payment_method,
            "label": payment_label(&r.payment_method, r.card_last4.as_deref()),
        },
        "paused_at": r.paused_at.map(|t| time.format(t)),
        "cancelled_at": r.cancelled_at.map(|t| time.format(t)),
    }))
}

pub(crate) fn not_found(id: &str) -> ToolError {
    ToolError::new(
        ErrorCode::NOT_FOUND,
        format!("There is no subscription {id}."),
    )
    .detail(json!({"entity": "subscription", "id": id}))
    .next("list_subscriptions")
}

/// The subscription, when the caller may see it: its shopper, or an operator.
pub(crate) async fn visible_subscription(
    cx: &Cx,
    tx: &mut TxCx,
    id: &str,
) -> Result<SubscriptionRow, ToolError> {
    let r = repo::subscription(tx, id)
        .await?
        .ok_or_else(|| not_found(id))?;
    match owner::viewer(&cx.principal) {
        Viewer::Staff => Ok(r),
        Viewer::Shopper(c) if c == r.customer_id => Ok(r),
        Viewer::Shopper(_) => Err(ToolError::new(
            ErrorCode::FORBIDDEN,
            "This is not your subscription.",
        )
        .detail(json!({"reason": "owner"}))
        .next("list_subscriptions")),
        Viewer::Guest => Err(not_signed_in()),
    }
}

/// `invalid_state` for a subscription that cannot do this from where it is.
pub(crate) fn invalid_state(r: &SubscriptionRow, doing: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INVALID_STATE,
        format!(
            "Subscription {} is {}; it cannot be {doing}.",
            r.id, r.subscription_status
        ),
    )
    .detail(json!({"entity": "subscription", "from": r.subscription_status, "allowed_to": []}))
    .next("list_subscriptions")
}

pub(crate) fn basis(r: &SubscriptionRow) -> Vec<Basis> {
    vec![Basis {
        entity: "sales.subscription".into(),
        id: r.id.clone(),
        version: i64::from(r.version),
    }]
}

/// When the job runs for a charge date: the start of that day in store time.
pub(crate) fn charge_time(time: &StoreTime, date: NaiveDate) -> Result<DateTime<Utc>, ToolError> {
    time.parse(&format!("{date}T00:00"))
        .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))
}

/// Schedules the order of a subscription's charge date (`subscription.renew`). The job places
/// nothing if the subscription is no longer active then, or its date moved.
pub(crate) async fn schedule_renewal(
    tx: &mut TxCx,
    subscription_id: &str,
    date: NaiveDate,
) -> Result<(), ToolError> {
    let at = charge_time(&tx.time, date)?;
    acom_changeset::jobs::schedule(
        tx,
        RENEW_JOB,
        at,
        &json!({"subscription_id": subscription_id,
                "charge_date": date.format("%Y-%m-%d").to_string()}),
    )
    .await?;
    Ok(())
}

/// What pausing, resuming or cancelling a subscription changes: its state before and after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StatePlan {
    pub subscription_id: String,
    pub from: SubscriptionState,
    pub to: SubscriptionState,
}

impl StatePlan {
    /// The preview's `changes`: the state, and the next charge date when it moves.
    pub fn changes(&self) -> Vec<Change> {
        let moved = self.from.next_charge_date != self.to.next_charge_date;
        vec![Change {
            op: ChangeOp::Update,
            entity: "subscription".into(),
            id: self.subscription_id.clone(),
            from: json!(self.from.status),
            to: json!(self.to.status),
            note: if self.from == self.to {
                Some(format!("Already {}", self.to.status))
            } else if moved {
                Some(format!(
                    "Next charge {}",
                    self.to.next_charge_date.format("%Y-%m-%d")
                ))
            } else {
                None
            },
        }]
    }
}

/// Writes the planned state (again, harmlessly, when it is already there). A charge date that
/// moved gets its order scheduled.
pub(crate) async fn apply_state(tx: &mut TxCx, plan: &StatePlan) -> Result<Committed, ToolError> {
    let r = repo::subscription_for_update(tx, &plan.subscription_id)
        .await?
        .ok_or_else(|| not_found(&plan.subscription_id))?;
    let now = SubscriptionState::from(&r);
    if now != plan.to {
        repo::set_subscription_state(tx, &r.id, &plan.to).await?;
        if plan.to.status == "active" && now.status != "active" {
            schedule_renewal(tx, &r.id, plan.to.next_charge_date).await?;
        }
    }
    let mut produced = Map::new();
    produced.insert("subscription_id".into(), json!(r.id));
    produced.insert("status".into(), json!(plan.to.status));
    produced.insert(
        "next_charge_date".into(),
        json!(plan.to.next_charge_date.format("%Y-%m-%d").to_string()),
    );
    Ok(Committed { id: r.id, produced })
}

/// Whether the subscription is in the planned state.
pub(crate) async fn state_is(cx: &Cx, plan: &StatePlan) -> Result<Verification, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let r = repo::subscription(&mut tx, &plan.subscription_id).await?;
    tx.commit().await.map_err(ToolError::from)?;
    let now = r.as_ref().map(SubscriptionState::from);
    let matched = now.as_ref().is_some_and(|s| {
        s.status == plan.to.status && s.next_charge_date == plan.to.next_charge_date
    });
    Ok(Verification {
        matched,
        mismatches: if matched {
            vec![]
        } else {
            vec![format!(
                "subscription {} is {:?}",
                plan.subscription_id,
                now.map(|s| s.status)
            )]
        },
    })
}

/// Puts the state back as it was before the plan ran.
pub(crate) async fn restore_state(tx: &mut TxCx, plan: &StatePlan) -> Result<(), ToolError> {
    repo::subscription_for_update(tx, &plan.subscription_id)
        .await?
        .ok_or_else(|| not_found(&plan.subscription_id))?;
    repo::set_subscription_state(tx, &plan.subscription_id, &plan.from).await?;
    if plan.from.status == "active" && plan.to.status != "active" {
        schedule_renewal(tx, &plan.subscription_id, plan.from.next_charge_date).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_core::Currency;
    use std::str::FromStr;

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    fn state(status: &str, next: &str) -> SubscriptionState {
        SubscriptionState {
            status: status.into(),
            next_charge_date: NaiveDate::parse_from_str(next, "%Y-%m-%d").unwrap(),
            paused_at: None,
            cancelled_at: None,
            cancel_reason_text: None,
        }
    }

    #[test]
    fn a_resume_that_moves_the_date_says_so() {
        let plan = StatePlan {
            subscription_id: "sub_3302".into(),
            from: state("paused", "2026-11-01"),
            to: state("active", "2026-11-15"),
        };
        let c = &plan.changes()[0];
        assert_eq!(
            (c.from.clone(), c.to.clone()),
            (json!("paused"), json!("active"))
        );
        assert_eq!(c.note.as_deref(), Some("Next charge 2026-11-15"));
        let same = StatePlan {
            subscription_id: "sub_3301".into(),
            from: state("paused", "2026-12-01"),
            to: state("paused", "2026-12-01"),
        };
        assert_eq!(same.changes()[0].note.as_deref(), Some("Already paused"));
    }

    #[test]
    fn lines_survive_the_database() {
        let l = SubscriptionLine {
            variant_id: "var_merino_tee_m_charcoal".into(),
            product_id: "prd_merino_tee".into(),
            name: "Merino trail tee".into(),
            size: Some("M".into()),
            color: Some("Charcoal".into()),
            quantity: 1,
            price: usd("58.00"),
            discount: usd("0.00"),
        };
        let v = serde_json::to_value(vec![l.clone()]).unwrap();
        assert_eq!(v[0]["price"]["value"].to_string(), "58.00");
        let back: Vec<SubscriptionLine> = serde_json::from_value(v).unwrap();
        assert_eq!(back, vec![l]);
    }
}
