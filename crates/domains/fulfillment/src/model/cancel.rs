//! Cancels before shipment: which lines and reasons a shopper can choose, and the estimate
//! (`fulfillment.yml cancel`, `refund.split`). `get_cancel_options` shows it; the person-only
//! `POST /v1/fulfillment/orders/{order_id}/cancel` computes it again and acts on it.

use acom_core::Money;
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Value, json};

use super::{Rules, holds, money_text};

/// One order line as the cancel sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct CancelLine {
    pub order_item_id: String,
    pub name: String,
    pub ordered: i32,
    /// Units already in cancel claims that were not declined.
    pub already_cancelled: i32,
    /// An open claim (any type) on the line.
    pub open_claim: Option<String>,
    pub order_item_status: String,
    pub fulfillment_stage: Option<String>,
    pub unit_price: Money,
    /// Discount on the whole line, including its share of order coupons.
    pub line_discount: Money,
    pub promotion_id: Option<String>,
}

impl CancelLine {
    /// `line_state` in the policy: the warehouse step when there is one, else the item status.
    pub fn state(&self) -> &str {
        self.fulfillment_stage
            .as_deref()
            .unwrap_or(&self.order_item_status)
    }

    fn left(&self) -> i32 {
        if self.order_item_status == "cancelled" {
            0
        } else {
            (self.ordered - self.already_cancelled).max(0)
        }
    }

    /// What `units` of this line cost the shopper: list price less their rounded share of the
    /// line's discount (the same arithmetic as a return's refund).
    pub fn paid_for(&self, units: i32) -> Money {
        super::returns::ClaimLine {
            order_item_id: self.order_item_id.clone(),
            name: self.name.clone(),
            unit_price: self.unit_price,
            ordered: self.ordered,
            line_discount: self.line_discount,
            quantity: units,
        }
        .paid(true)
    }
}

/// One payment of the order.
#[derive(Debug, Clone, PartialEq)]
pub struct PaymentLine {
    pub id: String,
    pub method: String,
    pub amount: Money,
    pub card_last4: Option<String>,
    pub paid_at: Option<DateTime<Utc>>,
    /// False while the payment is still due.
    pub completed: bool,
}

impl PaymentLine {
    /// `card ending 4821`, `loyalty points`, …: where refunded money goes.
    pub fn label(&self) -> String {
        match (self.method.as_str(), &self.card_last4) {
            ("credit_card", Some(last4)) => format!("card ending {last4}"),
            ("credit_card", None) => "card".to_owned(),
            (other, _) => other.replace('_', " "),
        }
    }
}

/// A coupon the order used.
#[derive(Debug, Clone, PartialEq)]
pub struct CouponLine {
    pub member_coupon_id: String,
    pub coupon_id: String,
    pub promotion_id: String,
    /// `item_price` (item coupon), `order_discount` (order coupon) or `add_on`.
    pub layer: String,
    pub benefit: Option<Money>,
    pub min_subtotal: Option<Money>,
}

/// The order around the lines.
#[derive(Debug, Clone, PartialEq)]
pub struct CancelOrder {
    pub lines: Vec<CancelLine>,
    pub shipping_charged: Money,
    pub payments: Vec<PaymentLine>,
    pub coupons: Vec<CouponLine>,
    /// Refunds the order already had.
    pub already_refunded: Money,
}

/// Why a cancel cannot be estimated as asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancelBlock {
    OrderItemNotFound(String),
    AlreadyShipped(String),
    AlreadyCancelled(String),
    QuantityOverCancellable {
        order_item_id: String,
        cancellable: i32,
    },
    ClaimInProgress(String),
    ReasonNotSelectable {
        allowed: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CancelItem {
    pub order_item_id: String,
    pub name: String,
    pub ordered: i32,
    pub cancellable_quantity: i32,
    pub order_item_status: String,
    pub fulfillment_stage: Option<String>,
    /// `self_service`, `store_review` or `not_allowed`.
    pub path: String,
    pub unit_paid: Money,
    pub whole_quantity_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReasonChoice {
    pub reason: String,
    pub liability: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub needs_store_review: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtraCharge {
    pub kind: String,
    pub amount: Money,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Recalculated {
    pub coupon_id: String,
    pub amount: Money,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RefundPart {
    pub to: String,
    pub method: String,
    pub amount: Money,
    #[serde(skip)]
    pub payment_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CouponBack {
    pub coupon_id: String,
    pub amount: Money,
    #[serde(skip)]
    pub member_coupon_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Estimate {
    /// `full` or `partial`.
    pub scope: String,
    pub cancelled_amount: Money,
    pub extra_charge: Vec<ExtraCharge>,
    pub discount_recalculated: Vec<Recalculated>,
    pub refund_total: Money,
    pub refund_breakdown: Vec<RefundPart>,
    pub money_back_business_days: Value,
    pub coupons_restored: Vec<CouponBack>,
    pub points_pending_cancelled: Money,
    pub charge_due: Money,
    pub needs_refund_destination: bool,
    /// `store_automatic` or `store_person`.
    pub confirmed_by: String,
    pub revocable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NotCancellable {
    pub order_item_id: String,
    pub why: String,
    pub next_tool: String,
}

/// Everything a cancel would do.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CancelOutcome {
    pub items: Vec<CancelItem>,
    pub reasons: Vec<ReasonChoice>,
    pub estimate: Estimate,
    pub not_cancellable: Vec<NotCancellable>,
    /// The lines and units cancelled.
    #[serde(skip)]
    pub chosen: Vec<(String, i32)>,
    #[serde(skip)]
    pub reason: String,
    #[serde(skip)]
    pub liability: String,
    /// What the claim becomes: `completed`, `approved`, `charge_due` or `requested`.
    #[serde(skip)]
    pub status: String,
}

fn path_for(rules: &Rules, state: &str) -> &'static str {
    if rules
        .list("cancel.self_service_states")
        .iter()
        .any(|s| s == state)
    {
        "self_service"
    } else if rules
        .list("cancel.operator_only_states")
        .iter()
        .any(|s| s == state)
    {
        "store_review"
    } else {
        "not_allowed"
    }
}

/// The liability a cancel reason carries (`cancel.reasons`).
pub fn cancel_liability(rules: &Rules, reason: &str) -> Option<String> {
    let reasons = rules.lookup("cancel.reasons")?.as_object()?;
    reasons.iter().find_map(|(liability, list)| {
        list.as_array()?
            .iter()
            .any(|r| r.as_str() == Some(reason))
            .then(|| liability.clone())
    })
}

/// The reasons a shopper may pick for these lines: every shopper-fault reason except the one
/// the system uses for unpaid orders, and the store-fault reasons whose condition holds
/// (`cancel.shopper_selectable_store_reasons`).
pub fn selectable_reasons(rules: &Rules, lines: &[&CancelLine]) -> Vec<ReasonChoice> {
    let mut out: Vec<ReasonChoice> = rules
        .list("cancel.reasons.customer_fault")
        .into_iter()
        .filter(|r| r != "payment_not_completed")
        .map(|reason| ReasonChoice {
            reason,
            liability: "customer_fault".into(),
            needs_store_review: false,
        })
        .collect();
    let Some(store) = rules
        .lookup("cancel.shopper_selectable_store_reasons")
        .and_then(Value::as_object)
    else {
        return out;
    };
    for (reason, rule) in store {
        let liability = cancel_liability(rules, reason).unwrap_or_else(|| "store_fault".into());
        let (allowed, review) = match rule {
            Value::String(s) if s == "needs_operator" => (true, true),
            cond => (
                lines.iter().any(|l| {
                    holds(
                        cond,
                        &json!({"line_state": l.state(), "past_promised_ship_date": false}),
                        rules,
                    )
                }),
                false,
            ),
        };
        if allowed {
            out.push(ReasonChoice {
                reason: reason.clone(),
                liability,
                needs_store_review: review,
            });
        }
    }
    out
}

fn sum(currency_of: Money, parts: impl IntoIterator<Item = Money>) -> Money {
    parts
        .into_iter()
        .fold(Money::zero(currency_of.currency), |a, m| {
            Money::new(a.value + m.value, a.currency)
        })
}

/// Splits a refund over the order's payments: cash-like methods first in payment order, the
/// rest to loyalty points (`refund.split: cash_like_first`). Earlier refunds use the same
/// order. Every payment gets a row, zero or not.
pub fn split_refund(payments: &[PaymentLine], already: Money, total: Money) -> Vec<RefundPart> {
    let mut ordered: Vec<&PaymentLine> = payments
        .iter()
        .filter(|p| p.method != "loyalty_points")
        .collect();
    ordered.extend(payments.iter().filter(|p| p.method == "loyalty_points"));
    let mut used = already.value;
    let mut left = total.value;
    ordered
        .into_iter()
        .map(|p| {
            let taken = used.min(p.amount.value).max(Decimal::ZERO);
            used -= taken;
            let room = p.amount.value - taken;
            let amount = left.min(room).max(Decimal::ZERO);
            left -= amount;
            RefundPart {
                to: p.label(),
                method: p.method.clone(),
                amount: Money::new(amount, total.currency).rounded(),
                payment_id: p.id.clone(),
            }
        })
        .collect()
}

/// What is asked of the cancel model.
#[derive(Debug, Clone, Copy, Default)]
pub struct CancelAsk<'a> {
    /// Lines and units; every line that can be cancelled when `None`.
    pub items: Option<&'a [(String, i32)]>,
    /// The reason; `changed_mind` (the most careful estimate) when `None`.
    pub reason: Option<&'a str>,
    /// An operator's decision on who is at fault, instead of the reason's.
    pub liability: Option<&'a str>,
    /// An operator approving a request: any recorded reason stands and lines the store must
    /// review can be approved.
    pub operator: bool,
}

/// The cancel of the asked lines for the asked reason.
pub fn cancel_outcome(
    rules: &Rules,
    order: &CancelOrder,
    ask: CancelAsk<'_>,
    now: DateTime<Utc>,
) -> Result<CancelOutcome, CancelBlock> {
    let asked = ask.items;
    let currency = order.shipping_charged;
    let mut items = Vec::new();
    let mut not_cancellable = Vec::new();
    for l in &order.lines {
        let path = path_for(rules, l.state());
        let left = l.left();
        if path == "not_allowed" || left == 0 {
            let why = if l.order_item_status == "cancelled" || left == 0 {
                "cancelled".to_owned()
            } else {
                l.state().to_owned()
            };
            let next = if why == "cancelled" {
                "get_claim"
            } else {
                "get_return_options"
            };
            not_cancellable.push(NotCancellable {
                order_item_id: l.order_item_id.clone(),
                why,
                next_tool: next.into(),
            });
            continue;
        }
        items.push(CancelItem {
            order_item_id: l.order_item_id.clone(),
            name: l.name.clone(),
            ordered: l.ordered,
            cancellable_quantity: left,
            order_item_status: l.order_item_status.clone(),
            fulfillment_stage: l.fulfillment_stage.clone(),
            path: path.into(),
            unit_paid: if l.ordered > 0 {
                Money::new(
                    (l.unit_price.value * Decimal::from(l.ordered) - l.line_discount.value)
                        / Decimal::from(l.ordered),
                    l.unit_price.currency,
                )
                .rounded()
            } else {
                l.unit_price
            },
            whole_quantity_only: false,
        });
    }

    // The lines and units this cancel covers.
    let chosen: Vec<(String, i32)> = match asked {
        Some(list) => {
            for (id, q) in list {
                let l = order
                    .lines
                    .iter()
                    .find(|l| &l.order_item_id == id)
                    .ok_or_else(|| CancelBlock::OrderItemNotFound(id.clone()))?;
                if let Some(c) = &l.open_claim {
                    return Err(CancelBlock::ClaimInProgress(c.clone()));
                }
                if l.order_item_status == "cancelled" || l.left() == 0 {
                    return Err(CancelBlock::AlreadyCancelled(id.clone()));
                }
                if path_for(rules, l.state()) == "not_allowed" {
                    return Err(CancelBlock::AlreadyShipped(id.clone()));
                }
                if *q < 1 || *q > l.left() {
                    return Err(CancelBlock::QuantityOverCancellable {
                        order_item_id: id.clone(),
                        cancellable: l.left(),
                    });
                }
            }
            list.to_vec()
        }
        None => {
            let open = order.lines.iter().find_map(|l| {
                items
                    .iter()
                    .any(|i| i.order_item_id == l.order_item_id)
                    .then(|| l.open_claim.clone())
                    .flatten()
            });
            if let Some(c) = open {
                return Err(CancelBlock::ClaimInProgress(c));
            }
            if items.is_empty() {
                let shipped = not_cancellable.iter().find(|n| n.why != "cancelled");
                return Err(match shipped {
                    Some(n) => CancelBlock::AlreadyShipped(n.order_item_id.clone()),
                    None => CancelBlock::AlreadyCancelled(
                        order
                            .lines
                            .first()
                            .map(|l| l.order_item_id.clone())
                            .unwrap_or_default(),
                    ),
                });
            }
            items
                .iter()
                .map(|i| (i.order_item_id.clone(), i.cancellable_quantity))
                .collect()
        }
    };
    let chosen_lines: Vec<&CancelLine> = order
        .lines
        .iter()
        .filter(|l| chosen.iter().any(|(id, _)| id == &l.order_item_id))
        .collect();
    let units = |l: &CancelLine| {
        chosen
            .iter()
            .find(|(id, _)| id == &l.order_item_id)
            .map_or(0, |(_, q)| *q)
    };

    let reasons = selectable_reasons(rules, &chosen_lines);
    let reason = ask.reason.unwrap_or("changed_mind").to_owned();
    let (liability, needs_review) = match reasons.iter().find(|r| r.reason == reason) {
        Some(choice) => (choice.liability.clone(), choice.needs_store_review),
        None if ask.operator => (
            cancel_liability(rules, &reason).unwrap_or_else(|| "customer_fault".into()),
            false,
        ),
        None => {
            return Err(CancelBlock::ReasonNotSelectable {
                allowed: reasons.iter().map(|r| r.reason.clone()).collect(),
            });
        }
    };
    let liability = ask.liability.map(str::to_owned).unwrap_or(liability);
    let needs_review = needs_review && !ask.operator;

    let cancelled_amount = sum(currency, chosen_lines.iter().map(|l| l.paid_for(units(l))));
    // What stays after this cancel, at list prices (free shipping is measured before member
    // benefits, order.yml).
    let stays = sum(
        currency,
        order.lines.iter().map(|l| {
            l.unit_price
                .times(Decimal::from((l.left() - units(l)).max(0)))
        }),
    );
    let before = sum(
        currency,
        order
            .lines
            .iter()
            .map(|l| l.unit_price.times(Decimal::from(l.left()))),
    );
    let full = order.lines.iter().all(|l| l.left() - units(l) <= 0);
    let any_delivered = order
        .lines
        .iter()
        .any(|l| l.order_item_status == "delivered");
    let paid = order.payments.iter().any(|p| p.completed);
    let customer_fault = rules
        .lookup("cancel.shipping_recalculation.applies_when")
        .is_some_and(|w| holds(w, &json!({"liability": liability}), rules));

    let mut extra_charge = Vec::new();
    let mut shipping_back = Money::zero(currency.currency);
    if paid && full && !order.shipping_charged.value.is_zero() {
        // The whole shipping group is cancelled: the shipping paid comes back.
        shipping_back = order.shipping_charged;
    } else if paid
        && customer_fault
        && !full
        && order.shipping_charged.value.is_zero()
        && before.value >= rules.free_shipping_min.value
        && stays.value < rules.free_shipping_min.value
    {
        extra_charge.push(ExtraCharge {
            kind: "lost_free_shipping".into(),
            amount: rules.shipping_rate,
            why: format!(
                "What stays ({}) is under the {} free-shipping minimum",
                money_text(stays),
                money_text(rules.free_shipping_min)
            ),
        });
    }

    let mut discount_recalculated = Vec::new();
    let mut coupons_restored = Vec::new();
    if full {
        for c in &order.coupons {
            coupons_restored.push(CouponBack {
                coupon_id: c.coupon_id.clone(),
                amount: c.benefit.unwrap_or(Money::zero(currency.currency)),
                member_coupon_id: c.member_coupon_id.clone(),
            });
        }
    } else {
        for c in &order.coupons {
            let on_cancelled_lines = chosen_lines.iter().any(|l| {
                l.promotion_id.as_deref() == Some(c.promotion_id.as_str()) && l.left() == units(l)
            });
            if c.layer == "item_price" && on_cancelled_lines {
                coupons_restored.push(CouponBack {
                    coupon_id: c.coupon_id.clone(),
                    amount: c.benefit.unwrap_or(Money::zero(currency.currency)),
                    member_coupon_id: c.member_coupon_id.clone(),
                });
            }
            if c.layer == "order_discount"
                && !any_delivered
                && let Some(min) = c.min_subtotal
                && stays.value < min.value
            {
                let taken = sum(
                    currency,
                    order
                        .lines
                        .iter()
                        .filter(|l| l.promotion_id.as_deref() == Some(c.promotion_id.as_str()))
                        .map(|l| {
                            let stay = (l.left() - units(l)).max(0);
                            l.line_discount
                                .times(Decimal::from(stay) / Decimal::from(l.ordered.max(1)))
                                .rounded()
                        }),
                );
                if !taken.value.is_zero() {
                    discount_recalculated.push(Recalculated {
                        coupon_id: c.coupon_id.clone(),
                        amount: taken,
                        why: format!(
                            "What stays ({}) is under the coupon's {} minimum",
                            money_text(stays),
                            money_text(min)
                        ),
                    });
                }
            }
        }
    }

    let charges = sum(currency, extra_charge.iter().map(|c| c.amount));
    let taken_back = sum(currency, discount_recalculated.iter().map(|c| c.amount));
    let base = if paid {
        cancelled_amount.value + shipping_back.value - taken_back.value
    } else {
        Decimal::ZERO
    };
    let refund_total = Money::new((base - charges.value).max(Decimal::ZERO), currency.currency);
    let charge_due = Money::new((charges.value - base).max(Decimal::ZERO), currency.currency);
    let refund_breakdown = split_refund(&order.payments, order.already_refunded, refund_total);
    let needs_refund_destination = refund_breakdown.iter().any(|part| {
        part.amount.value > Decimal::ZERO
            && order.payments.iter().any(|p| {
                p.id == part.payment_id
                    && match (rules.reversal_window_days.get(&p.method), p.paid_at) {
                        (Some(Some(days)), Some(at)) => at + Duration::days(*days) < now,
                        _ => false,
                    }
            })
    });

    let review = needs_review
        || (!ask.operator
            && chosen_lines
                .iter()
                .any(|l| path_for(rules, l.state()) == "store_review"));
    let all_unpaid = chosen_lines.iter().all(|l| l.state() == "payment_due");
    let auto = ask.operator
        || rules.lookup("cancel.auto_approve_when").is_some_and(|w| {
            chosen_lines.iter().all(|l| {
                holds(
                    w,
                    &json!({"line_state": l.state(), "extra_charge": charge_due}),
                    rules,
                )
            })
        });
    let status = if review {
        "requested"
    } else if all_unpaid {
        "completed"
    } else if auto {
        "approved"
    } else if !charge_due.value.is_zero() {
        "charge_due"
    } else {
        "requested"
    };
    let (min, max) = rules.money_back_days;
    let estimate = Estimate {
        scope: if full { "full" } else { "partial" }.into(),
        cancelled_amount,
        extra_charge,
        discount_recalculated,
        refund_total: refund_total.rounded(),
        refund_breakdown,
        money_back_business_days: json!({"min": min, "max": max}),
        coupons_restored,
        points_pending_cancelled: cancelled_amount.times(rules.points_rate).rounded(),
        charge_due: charge_due.rounded(),
        needs_refund_destination,
        confirmed_by: if status == "requested" {
            "store_person"
        } else {
            "store_automatic"
        }
        .into(),
        revocable: rules
            .lookup("cancel.revocable")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    Ok(CancelOutcome {
        items,
        reasons,
        estimate,
        not_cancellable,
        chosen,
        reason,
        liability,
        status: status.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::{rules, usd};

    fn line(id: &str, name: &str, price: &str, ordered: i32, discount: &str) -> CancelLine {
        CancelLine {
            order_item_id: id.into(),
            name: name.into(),
            ordered,
            already_cancelled: 0,
            open_claim: None,
            order_item_status: "processing".into(),
            fulfillment_stage: None,
            unit_price: usd(price),
            line_discount: usd(discount),
            promotion_id: None,
        }
    }

    fn card(amount: &str) -> PaymentLine {
        PaymentLine {
            id: "pay_20455".into(),
            method: "credit_card".into(),
            amount: usd(amount),
            card_last4: Some("4821".into()),
            paid_at: Some(chrono::Utc::now()),
            completed: true,
        }
    }

    /// The spec's example order: a VIP windbreaker ($89.00 − $17.80) and two socks packs,
    /// paid $93.20 by card and $10.00 in points, shipped free, with a $4.00 socks coupon.
    fn order() -> CancelOrder {
        let mut socks = line("oln_2", "Merino socks, 2-pack", "16.00", 2, "0.00");
        socks.promotion_id = Some("promo_socks".into());
        CancelOrder {
            lines: vec![
                line("oln_1", "Packable windbreaker, M", "89.00", 1, "17.80"),
                socks,
            ],
            shipping_charged: usd("0.00"),
            payments: vec![
                card("93.20"),
                PaymentLine {
                    id: "pay_points".into(),
                    method: "loyalty_points".into(),
                    amount: usd("10.00"),
                    card_last4: None,
                    paid_at: Some(chrono::Utc::now()),
                    completed: true,
                },
            ],
            coupons: vec![CouponLine {
                member_coupon_id: "mcp_socks4".into(),
                coupon_id: "cpn_socks4".into(),
                promotion_id: "promo_socks".into(),
                layer: "item_price".into(),
                benefit: Some(usd("4.00")),
                min_subtotal: None,
            }],
            already_refunded: usd("0.00"),
        }
    }

    #[test]
    fn partial_cancel_takes_back_free_shipping_from_the_refund() {
        let asked = [("oln_1".to_owned(), 1)];
        let o = cancel_outcome(
            &rules(),
            &order(),
            CancelAsk {
                items: Some(&asked),
                reason: Some("changed_mind"),
                ..CancelAsk::default()
            },
            chrono::Utc::now(),
        )
        .unwrap();
        assert_eq!(o.items.len(), 2);
        assert_eq!(o.items[0].unit_paid, usd("71.20"));
        assert_eq!(o.items[0].path, "self_service");
        let e = &o.estimate;
        assert_eq!(e.scope, "partial");
        assert_eq!(e.cancelled_amount, usd("71.20"));
        assert_eq!(e.extra_charge.len(), 1);
        assert_eq!(e.extra_charge[0].kind, "lost_free_shipping");
        assert_eq!(e.extra_charge[0].amount, usd("6.00"));
        assert_eq!(
            e.extra_charge[0].why,
            "What stays ($32.00) is under the $80.00 free-shipping minimum"
        );
        assert_eq!(e.refund_total, usd("65.20"));
        assert_eq!(e.refund_breakdown[0].to, "card ending 4821");
        assert_eq!(e.refund_breakdown[0].amount, usd("65.20"));
        assert_eq!(e.refund_breakdown[1].to, "loyalty points");
        assert_eq!(e.refund_breakdown[1].amount, usd("0.00"));
        assert_eq!(e.points_pending_cancelled, usd("0.71"));
        assert_eq!(e.charge_due, usd("0.00"));
        assert!(e.coupons_restored.is_empty());
        assert_eq!(e.confirmed_by, "store_automatic");
        assert!(!e.revocable);
        assert_eq!(o.status, "approved");
        assert_eq!(o.liability, "customer_fault");
    }

    #[test]
    fn full_cancel_refunds_cash_first_then_points_and_restores_coupons() {
        let o =
            cancel_outcome(&rules(), &order(), CancelAsk::default(), chrono::Utc::now()).unwrap();
        let e = &o.estimate;
        assert_eq!(e.scope, "full");
        assert!(e.extra_charge.is_empty());
        assert_eq!(e.refund_total, usd("103.20"));
        assert_eq!(e.refund_breakdown[0].amount, usd("93.20"));
        assert_eq!(e.refund_breakdown[1].amount, usd("10.00"));
        assert_eq!(e.coupons_restored.len(), 1);
        assert_eq!(e.coupons_restored[0].coupon_id, "cpn_socks4");
        assert_eq!(e.coupons_restored[0].amount, usd("4.00"));
    }

    #[test]
    fn reasons_follow_the_line_state() {
        let r = rules();
        let o = cancel_outcome(&r, &order(), CancelAsk::default(), chrono::Utc::now()).unwrap();
        let words: Vec<&str> = o.reasons.iter().map(|c| c.reason.as_str()).collect();
        assert!(words.contains(&"changed_mind"));
        assert!(words.contains(&"duplicate_order"));
        assert!(!words.contains(&"payment_not_completed"));
        assert!(!words.contains(&"out_of_stock"));
        let review = o
            .reasons
            .iter()
            .find(|c| c.reason == "product_info_inaccurate")
            .unwrap();
        assert!(review.needs_store_review);
        assert_eq!(review.liability, "store_fault");

        let mut waiting = order();
        waiting.lines[0].fulfillment_stage = Some("awaiting_stock".into());
        let asked = [("oln_1".to_owned(), 1)];
        let o = cancel_outcome(
            &r,
            &waiting,
            CancelAsk {
                items: Some(&asked),
                reason: Some("out_of_stock"),
                ..CancelAsk::default()
            },
            chrono::Utc::now(),
        )
        .unwrap();
        // Store fault: no shipping charge.
        assert!(o.estimate.extra_charge.is_empty());
        assert_eq!(o.estimate.refund_total, usd("71.20"));
        let e = cancel_outcome(
            &r,
            &order(),
            CancelAsk {
                items: Some(&asked),
                reason: Some("out_of_stock"),
                ..CancelAsk::default()
            },
            chrono::Utc::now(),
        );
        assert!(matches!(e, Err(CancelBlock::ReasonNotSelectable { .. })));
    }

    #[test]
    fn packing_lines_go_to_the_store_and_shipped_lines_are_refused() {
        let r = rules();
        let mut packing = order();
        packing.lines[0].fulfillment_stage = Some("packed".into());
        let asked = [("oln_1".to_owned(), 1)];
        let o = cancel_outcome(
            &r,
            &packing,
            CancelAsk {
                items: Some(&asked),
                ..CancelAsk::default()
            },
            chrono::Utc::now(),
        )
        .unwrap();
        assert_eq!(o.status, "requested");
        assert_eq!(o.estimate.confirmed_by, "store_person");

        let mut shipped = order();
        shipped.lines[0].fulfillment_stage = Some("shipped".into());
        let e = cancel_outcome(
            &r,
            &shipped,
            CancelAsk {
                items: Some(&asked),
                ..CancelAsk::default()
            },
            chrono::Utc::now(),
        );
        assert_eq!(e, Err(CancelBlock::AlreadyShipped("oln_1".into())));
        let o = cancel_outcome(&r, &shipped, CancelAsk::default(), chrono::Utc::now()).unwrap();
        assert_eq!(o.not_cancellable.len(), 1);
        assert_eq!(o.not_cancellable[0].why, "shipped");
        assert_eq!(o.not_cancellable[0].next_tool, "get_return_options");

        let too_many = [("oln_2".to_owned(), 3)];
        let e = cancel_outcome(
            &r,
            &order(),
            CancelAsk {
                items: Some(&too_many),
                ..CancelAsk::default()
            },
            chrono::Utc::now(),
        );
        assert!(matches!(
            e,
            Err(CancelBlock::QuantityOverCancellable { cancellable: 2, .. })
        ));
    }

    #[test]
    fn an_unpaid_order_cancels_at_once_without_a_refund() {
        let mut unpaid = order();
        for l in &mut unpaid.lines {
            l.order_item_status = "payment_due".into();
        }
        for p in &mut unpaid.payments {
            p.completed = false;
        }
        let o =
            cancel_outcome(&rules(), &unpaid, CancelAsk::default(), chrono::Utc::now()).unwrap();
        assert_eq!(o.status, "completed");
        assert_eq!(o.estimate.refund_total, usd("0.00"));
    }

    #[test]
    fn an_old_card_payment_needs_another_refund_destination() {
        let mut old = order();
        old.payments[0].paid_at = Some(chrono::Utc::now() - Duration::days(400));
        let o = cancel_outcome(&rules(), &old, CancelAsk::default(), chrono::Utc::now()).unwrap();
        assert!(o.estimate.needs_refund_destination);
    }

    #[test]
    fn an_operator_approves_a_packing_line_with_the_recorded_reason() {
        let r = rules();
        let mut packing = order();
        packing.lines[0].fulfillment_stage = Some("packed".into());
        let asked = [("oln_1".to_owned(), 1)];
        let ask = CancelAsk {
            items: Some(&asked),
            reason: Some("changed_mind"),
            operator: true,
            ..CancelAsk::default()
        };
        let o = cancel_outcome(&r, &packing, ask, chrono::Utc::now()).unwrap();
        assert_eq!(o.status, "approved");
        assert_eq!(o.estimate.refund_total, usd("65.20"));
        // Deciding the store was at fault drops the shipping charge.
        let store = CancelAsk {
            liability: Some("store_fault"),
            ..ask
        };
        let o = cancel_outcome(&r, &packing, store, chrono::Utc::now()).unwrap();
        assert_eq!(o.liability, "store_fault");
        assert_eq!(o.estimate.refund_total, usd("71.20"));
    }
}
