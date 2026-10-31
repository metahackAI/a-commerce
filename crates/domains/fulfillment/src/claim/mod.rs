//! The `claim` resource (docs/api/fulfillment.md `## claim`): cancel, return and exchange
//! claims. One file per operation; this module holds what they share: finding a claim the
//! caller may see, loading a line as a return, an exchange or a cancel sees it, the errors for
//! lines that cannot be returned or cancelled, the effects a cancel has besides the claim
//! itself, and refunding an extra charge a claim no longer needs.

pub(crate) mod approve_claim;
pub(crate) mod cancel_my_order;
pub(crate) mod cancel_order_lines;
pub(crate) mod convert_exchange_to_return;
pub(crate) mod evaluate_return_request;
pub(crate) mod get_cancel_options;
pub(crate) mod get_claim;
pub(crate) mod get_exchange_options;
pub(crate) mod get_return_options;
pub(crate) mod pay_claim_charge;
pub(crate) mod reject_claim;
pub(crate) mod search_claims;
pub(crate) mod search_return_requests;
pub(crate) mod send_exchange_request;
pub(crate) mod send_return_request;
pub(crate) mod upload_evidence;
pub(crate) mod withdraw_claim;

pub use approve_claim::ApproveClaim;
pub use cancel_my_order::CancelMyOrder;
pub use cancel_order_lines::CancelOrderLines;
pub use convert_exchange_to_return::ConvertExchangeToReturn;
pub use evaluate_return_request::EvaluateReturnRequest;
pub use get_cancel_options::GetCancelOptions;
pub use get_claim::GetClaim;
pub use get_exchange_options::GetExchangeOptions;
pub use get_return_options::GetReturnOptions;
pub use pay_claim_charge::PayClaimCharge;
pub use reject_claim::RejectClaim;
pub use search_claims::SearchClaims;
pub use search_return_requests::SearchReturnRequests;
pub use send_exchange_request::SendExchangeRequest;
pub use send_return_request::SendReturnRequest;
pub use upload_evidence::UploadEvidence;
pub use withdraw_claim::WithdrawClaim;

use std::collections::BTreeMap;

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{Basis, Change, ChangeOp, ErrorCode, ToolError};
use acom_core::Money;
use acom_customer::service::for_fulfillment as customer;
use acom_promotion::service::for_fulfillment as promotion;
use acom_sales::service::for_fulfillment as sales;
use acom_store::TxCx;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::errors::{
    ALREADY_CANCELLED, ALREADY_SHIPPED, CLAIM_IN_PROGRESS, CLAIM_NOT_FOUND, NOT_EXCHANGEABLE,
    NOT_RETURNABLE, NOT_SHIPPED_YET, NOT_YOUR_ORDER, ORDER_ITEM_NOT_FOUND, OUTSIDE_EXCHANGE_WINDOW,
    OUTSIDE_RETURN_WINDOW, QUANTITY_OVER_CANCELLABLE, REASON_NOT_SELECTABLE,
    REPLACEMENT_UNAVAILABLE,
};
use crate::model::cancel::{CancelBlock, CancelLine, CancelOrder, CouponLine, PaymentLine};
use crate::model::delivery::{days_since, estimate, exchange_dates, local_date, pickup_window};
use crate::model::exchange::{
    Candidate, Choice, ExchangeCost, choices, exchange_cost, exchange_liability, exchange_reasons,
    needs_person, not_exchangeable,
};
use crate::model::returns::{
    ClaimLine, OptionInputs, ReturnBlock, ReturnOption, ReturnableLine, Window, return_options,
    returnable,
};
use crate::model::zone::{Zone, default_zone, zone_for};
use crate::model::{Rules, date_text};
use crate::repo::{self, ClaimRow, NewRefund};
use crate::{delivered_at, order_for, original_payment, payment_label, shopper};

pub(crate) fn claim_not_found(id: &str, next: &'static str) -> ToolError {
    ToolError::new(CLAIM_NOT_FOUND, format!("There is no claim {id}."))
        .detail(json!({"entity": "claim", "id": id}))
        .next(next)
}

/// A claim the caller may see: shoppers only see their own (others look missing).
pub(crate) async fn visible_claim(
    tx: &mut TxCx,
    claim_id: &str,
    next: &'static str,
) -> Result<ClaimRow, ToolError> {
    let me = shopper(&tx.principal);
    let claim = repo::claim(tx, claim_id)
        .await?
        .ok_or_else(|| claim_not_found(claim_id, next))?;
    if me.as_deref().is_some_and(|m| m != claim.customer_id) {
        return Err(claim_not_found(claim_id, next));
    }
    Ok(claim)
}

/// The order lines of each claim with what was paid for them, keyed by claim id.
pub(crate) async fn claim_lines(
    tx: &mut TxCx,
    claim_ids: &[String],
) -> Result<BTreeMap<String, Vec<ClaimLine>>, ToolError> {
    let rows = repo::claim_items(tx, claim_ids).await?;
    let item_ids: Vec<String> = rows.iter().map(|r| r.order_item_id.clone()).collect();
    let items = sales::items(tx, &item_ids).await?;
    let mut out: BTreeMap<String, Vec<ClaimLine>> = BTreeMap::new();
    for r in rows {
        let Some(i) = items.iter().find(|i| i.id == r.order_item_id) else {
            continue;
        };
        out.entry(r.claim_id).or_default().push(ClaimLine {
            order_item_id: i.id.clone(),
            name: i.name.clone(),
            unit_price: i.price,
            ordered: i.order_quantity,
            line_discount: i.discount,
            quantity: r.order_quantity,
        });
    }
    Ok(out)
}

/// Everything known about returning one line now.
pub(crate) struct ReturnView {
    pub line: ReturnableLine,
    pub order: sales::Order,
    pub item: sales::OrderItem,
    pub variant: Option<catalog::Variant>,
    pub payment: Option<sales::Payment>,
}

/// Loads one line of the caller's order as a return sees it.
pub(crate) async fn return_view(
    tx: &mut TxCx,
    order_id: &str,
    order_item_id: &str,
    quantity_asked: Option<i32>,
    now: chrono::DateTime<chrono::Utc>,
    zone: chrono_tz::Tz,
) -> Result<ReturnView, ToolError> {
    let me = shopper(&tx.principal);
    let order = order_for(tx, order_id, me.as_deref(), "get_my_orders").await?;
    let items = sales::order_items(tx, order_id).await?;
    let item = items
        .into_iter()
        .find(|i| i.id == order_item_id)
        .ok_or_else(|| {
            ToolError::new(
                ORDER_ITEM_NOT_FOUND,
                format!("Order {} has no line {order_item_id}.", order.order_number),
            )
            .detail(json!({"entity": "order_item", "id": order_item_id}))
            .next("get_order")
        })?;
    let claims = repo::claims_on_order(tx, order_id).await?;
    let claim_ids: Vec<String> = claims.iter().map(|c| c.id.clone()).collect();
    let claim_items = repo::claim_items(tx, &claim_ids).await?;
    let on_line = |c: &ClaimRow| {
        claim_items
            .iter()
            .filter(|i| i.claim_id == c.id && i.order_item_id == item.id)
            .map(|i| i.order_quantity)
            .sum::<i32>()
    };
    let open_claim = claims
        .iter()
        .find(|c| c.is_open() && on_line(c) > 0)
        .map(|c| c.id.clone());
    let already_claimed = claims
        .iter()
        .filter(|c| {
            c.claim_type != "cancel" && !matches!(c.claim_status.as_str(), "declined" | "withdrawn")
        })
        .map(on_line)
        .sum();
    let deliveries = repo::deliveries_of_orders(tx, std::slice::from_ref(&order.id)).await?;
    let variant = catalog::variant(tx, &item.variant_id).await?;
    let payments = sales::payments(tx, order_id).await?;
    let not_returnable_because = variant.as_ref().and_then(|v| {
        if v.sale_type == "gift_card" {
            Some("gift_card".to_owned())
        } else {
            v.keywords.iter().find(|k| *k == "final_sale").cloned()
        }
    });
    let line = ReturnableLine {
        line: ClaimLine {
            order_item_id: item.id.clone(),
            name: item.name.clone(),
            unit_price: item.price,
            ordered: item.order_quantity,
            line_discount: item.discount,
            quantity: quantity_asked.unwrap_or(item.order_quantity),
        },
        order_item_status: item.order_item_status.clone(),
        fulfillment_stage: item.fulfillment_stage.clone(),
        days_since_delivery: delivered_at(&deliveries, &order.id).map(|d| days_since(d, now, zone)),
        already_claimed,
        open_claim,
        not_returnable_because,
    };
    Ok(ReturnView {
        line,
        payment: original_payment(&payments).cloned(),
        order,
        item,
        variant,
    })
}

/// The error a person or an agent sees when a line cannot be returned. `delivered` gives the
/// sentence its date.
pub(crate) fn return_block_error(
    b: ReturnBlock,
    delivered: Option<chrono::NaiveDate>,
) -> ToolError {
    match b {
        ReturnBlock::NotShippedYet => ToolError::new(
            NOT_SHIPPED_YET,
            "This item has not shipped yet; it can be cancelled instead.",
        )
        .next("get_cancel_options"),
        ReturnBlock::OutsideWindow { days, limit } => ToolError::new(
            OUTSIDE_RETURN_WINDOW,
            match delivered {
                Some(d) => format!(
                    "Delivered {}, {days} days ago; returns are accepted within {limit} days.",
                    date_text(d)
                ),
                None => format!("Returns are accepted within {limit} days of delivery."),
            },
        )
        .detail(json!({"rule": "return_window_days", "days_since_delivery": days, "limit": limit})),
        ReturnBlock::NotReturnable { why } => ToolError::new(
            NOT_RETURNABLE,
            match why.as_str() {
                "final_sale" => "Final-sale items cannot be returned.".to_owned(),
                "gift_card" => "Gift cards cannot be returned.".to_owned(),
                other => format!(
                    "This item cannot be returned ({}).",
                    other.replace('_', " ")
                ),
            },
        )
        .detail(json!({"rule": "return.non_returnable_when", "why": why})),
        ReturnBlock::ClaimInProgress { claim_id } => ToolError::new(
            CLAIM_IN_PROGRESS,
            format!("This item already has an open request ({claim_id})."),
        )
        .detail(json!({"existing_id": claim_id}))
        .next("get_claim"),
        ReturnBlock::TooMany { returnable } => {
            ToolError::invalid_input(format!("Up to {returnable} of this item can be returned."))
                .detail(json!({"field": "quantity", "reason": "range", "max": returnable}))
        }
    }
}

/// The return choices for a loaded line, or why there are none.
pub(crate) fn options_for(
    rules: &Rules,
    cx_policy: &acom_policy::PolicySet,
    v: &ReturnView,
    reason: &str,
    quantity: Option<i32>,
    now: chrono::DateTime<chrono::Utc>,
    time: &acom_core::StoreTime,
) -> Result<(i32, Vec<ReturnOption>), ToolError> {
    let delivered = v
        .line
        .days_since_delivery
        .map(|d| local_date(now, time.zone) - chrono::Duration::days(d));
    let q = returnable(rules, &v.line, quantity).map_err(|b| return_block_error(b, delivered))?;
    let line = ClaimLine {
        quantity: q,
        ..v.line.line.clone()
    };
    let (ships_on, arrives_by) = estimate(rules, now, time.zone);
    let pickup = rules
        .send_back_methods
        .iter()
        .any(|m| m == "pickup")
        .then(|| {
            let (start, end) = pickup_window(rules, now, time.zone);
            Window {
                start: time.format(start),
                end: time.format(end),
            }
        });
    let stock = v.variant.as_ref().and_then(|var| {
        var.is_managed
            .then(|| var.stock.as_ref().map_or(0, |s| s.inventory_level))
    });
    let inputs = OptionInputs {
        reason: reason.to_owned(),
        quantity: q,
        variant_id: v.item.variant_id.clone(),
        has_size: v.variant.as_ref().is_some_and(|var| var.size.is_some()),
        in_stock: stock,
        refund_to: v
            .payment
            .as_ref()
            .map(payment_label)
            .unwrap_or_else(|| "the original payment".to_owned()),
        ships_on,
        arrives_by,
        today: local_date(now, time.zone),
        pickup_window: pickup,
    };
    Ok((q, return_options(rules, cx_policy, &line, &inputs)))
}

/// Loads an order as the cancel model sees it.
/// `exclude` leaves one claim out (the request an operator is approving).
pub(crate) async fn cancel_view(
    tx: &mut TxCx,
    order: &sales::Order,
    exclude: Option<&str>,
) -> Result<CancelOrder, ToolError> {
    let items = sales::order_items(tx, &order.id).await?;
    let mut claims = repo::claims_on_order(tx, &order.id).await?;
    claims.retain(|c| Some(c.id.as_str()) != exclude);
    let claim_ids: Vec<String> = claims.iter().map(|c| c.id.clone()).collect();
    let claim_items = repo::claim_items(tx, &claim_ids).await?;
    let payments = sales::payments(tx, &order.id).await?;
    let coupons = promotion::coupons_used(tx, &order.id).await?;
    let refunds = repo::refunds_on_order(tx, &order.id).await?;
    let units = |c: &ClaimRow, line: &str| {
        claim_items
            .iter()
            .filter(|i| i.claim_id == c.id && i.order_item_id == line)
            .map(|i| i.order_quantity)
            .sum::<i32>()
    };
    let lines = items
        .iter()
        .map(|i| CancelLine {
            order_item_id: i.id.clone(),
            name: i.name.clone(),
            ordered: i.order_quantity,
            already_cancelled: claims
                .iter()
                .filter(|c| {
                    c.claim_type == "cancel" && c.claim_status != "declined" && !c.is_open()
                })
                .map(|c| units(c, &i.id))
                .sum(),
            open_claim: claims
                .iter()
                .find(|c| c.is_open() && units(c, &i.id) > 0)
                .map(|c| c.id.clone()),
            order_item_status: i.order_item_status.clone(),
            fulfillment_stage: i.fulfillment_stage.clone(),
            unit_price: i.price,
            line_discount: i.discount,
            promotion_id: i.promotion_id.clone(),
        })
        .collect();
    let mut already = Money::zero(order.total_payment_due.currency);
    for r in &refunds {
        already.value += r.amount()?.value;
    }
    Ok(CancelOrder {
        lines,
        shipping_charged: order.shipping,
        payments: payments
            .iter()
            .map(|p| PaymentLine {
                id: p.id.clone(),
                method: p.payment_method.clone(),
                amount: p.amount,
                card_last4: p.card_last4.clone(),
                paid_at: p.paid_at,
                completed: matches!(
                    p.payment_status.as_str(),
                    "payment_complete" | "payment_automatically_applied"
                ),
            })
            .collect(),
        coupons: coupons
            .into_iter()
            .map(|c| CouponLine {
                member_coupon_id: c.member_coupon_id,
                coupon_id: c.coupon_id,
                promotion_id: c.promotion_id,
                layer: c.discount_layer,
                benefit: c.benefit,
                min_subtotal: c.min_subtotal,
            })
            .collect(),
        already_refunded: already,
    })
}

/// The error for a cancel that cannot go ahead as asked. `next` tools follow the spec.
pub(crate) fn cancel_block_error(b: CancelBlock) -> ToolError {
    match b {
        CancelBlock::OrderItemNotFound(id) => {
            ToolError::new(ORDER_ITEM_NOT_FOUND, format!("The order has no line {id}."))
                .detail(json!({"entity": "order_item", "id": id}))
                .next("get_order")
        }
        CancelBlock::AlreadyShipped(id) => ToolError::new(
            ALREADY_SHIPPED,
            "This item has already shipped, so it can be returned instead of cancelled.",
        )
        .detail(json!({"order_item_id": id}))
        .next("get_return_options"),
        CancelBlock::AlreadyCancelled(id) => {
            ToolError::new(ALREADY_CANCELLED, "This item is already cancelled.")
                .detail(json!({"entity": "order_item", "id": id, "from": "cancelled"}))
                .next("get_claim")
        }
        CancelBlock::QuantityOverCancellable {
            order_item_id,
            cancellable,
        } => ToolError::new(
            QUANTITY_OVER_CANCELLABLE,
            format!("Only {cancellable} of this item can be cancelled."),
        )
        .detail(json!({"order_item_id": order_item_id, "limit": cancellable}))
        .next("get_cancel_options"),
        CancelBlock::ClaimInProgress(claim_id) => ToolError::new(
            CLAIM_IN_PROGRESS,
            format!("This item already has an open request ({claim_id})."),
        )
        .detail(json!({"existing_id": claim_id}))
        .next("get_claim"),
        CancelBlock::ReasonNotSelectable { allowed } => ToolError::new(
            REASON_NOT_SELECTABLE,
            "That reason cannot be chosen for these items.",
        )
        .detail(json!({"field": "reason", "allowed": allowed}))
        .next("get_cancel_options"),
    }
}

/// One line and units to cancel.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelItemInput {
    /// The order line.
    #[schemars(regex(pattern = r"^oln_[a-z0-9_]+$"), example = &"oln_1")]
    pub order_item_id: String,
    /// Units to cancel, 1 up to what is left on the line.
    #[schemars(range(min = 1), example = 1)]
    pub quantity: u32,
}

pub(crate) fn asked_items(items: &Option<Vec<CancelItemInput>>) -> Option<Vec<(String, i32)>> {
    items.as_ref().map(|l| {
        l.iter()
            .map(|i| {
                (
                    i.order_item_id.clone(),
                    i32::try_from(i.quantity).unwrap_or(i32::MAX),
                )
            })
            .collect()
    })
}

pub(crate) fn invalid_state(c: &ClaimRow, allowed: &[&str]) -> ToolError {
    ToolError::new(
        ErrorCode::INVALID_STATE,
        format!(
            "Claim {} is {}; this is not possible now.",
            c.id,
            c.claim_status.replace('_', " ")
        ),
    )
    .detail(json!({"entity": "claim", "from": c.claim_status, "allowed_to": allowed}))
    .next("get_claim")
}

/// One claim's approval, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApproveStep {
    pub claim_id: String,
    pub claim_type: String,
    pub order_id: String,
    pub from_status: String,
    pub to_status: String,
    /// The liability before and after, when the operator changed it.
    pub liability_change: Option<(String, String)>,
    /// `(order_item_id, status before)` for whole lines that become cancelled.
    pub lines_cancelled: Vec<(String, String)>,
    /// `(variant_id, units)` to put back in stock.
    pub restock: Vec<(String, i32)>,
    /// `(delivery_id, state before)` for parcels that are cancelled.
    pub deliveries_cancelled: Vec<(String, String)>,
    pub coupons_restored: Vec<String>,
    /// The order's status before, when the whole order becomes cancelled.
    pub order_cancelled_from: Option<String>,
    pub refund: Option<PlannedRefund>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedRefund {
    pub refund_id: String,
    pub payment_id: Option<String>,
    pub amount: Money,
    pub refund_method: String,
    pub refund_type: String,
    pub to: String,
}

/// `Store fault: store pays both shipping legs`, from `liability.pays`.
pub(crate) fn pays_sentence(rules: &crate::model::Rules, liability: &str) -> String {
    let who = liability.replace('_', " ");
    let mut who = who.chars();
    let who: String = who
        .next()
        .map(|f| f.to_uppercase().chain(who).collect())
        .unwrap_or_default();
    let pays = rules
        .liability_pays
        .get(liability)
        .map(|p| {
            p.iter()
                .map(|x| x.replace('_', " "))
                .collect::<Vec<_>>()
                .join(" and ")
        })
        .unwrap_or_default();
    let payer = if liability == "customer_fault" {
        "customer"
    } else {
        "store"
    };
    if pays.is_empty() {
        format!("{who}: nobody pays shipping")
    } else {
        format!("{who}: {payer} pays {pays}")
    }
}

/// What a cancel of `asked` lines does besides the claim: whole lines cancelled, stock put
/// back, parcels stopped, with the preview changes and the rows read.
pub(crate) struct CancelEffects {
    pub lines_cancelled: Vec<(String, String)>,
    pub restock: Vec<(String, i32)>,
    pub deliveries_cancelled: Vec<(String, String)>,
    pub stock_changes: Vec<Change>,
    pub parcel_changes: Vec<Change>,
    pub basis: Vec<Basis>,
}

pub(crate) async fn cancel_effects(
    tx: &mut TxCx,
    order_id: &str,
    view: &crate::model::cancel::CancelOrder,
    asked: &[(String, i32)],
) -> Result<CancelEffects, ToolError> {
    let lines = sales::order_items(tx, order_id).await?;
    let mut fx = CancelEffects {
        lines_cancelled: Vec::new(),
        restock: Vec::new(),
        deliveries_cancelled: Vec::new(),
        stock_changes: Vec::new(),
        parcel_changes: Vec::new(),
        basis: Vec::new(),
    };
    for (id, units) in asked {
        let Some(l) = lines.iter().find(|l| &l.id == id) else {
            continue;
        };
        let left = view
            .lines
            .iter()
            .find(|v| &v.order_item_id == id)
            .map_or(0, |v| v.ordered - v.already_cancelled);
        if *units >= left {
            fx.lines_cancelled
                .push((l.id.clone(), l.order_item_status.clone()));
        }
        if let Some(v) = catalog::variant(tx, &l.variant_id).await?
            && v.is_managed
            && let Some(s) = &v.stock
        {
            fx.stock_changes.push(Change {
                op: ChangeOp::Update,
                entity: "inventory_level".into(),
                id: v.id.clone(),
                from: json!(s.inventory_level),
                to: json!(s.inventory_level + units),
                note: None,
            });
            fx.restock.push((v.id.clone(), *units));
        }
    }
    let deliveries = repo::deliveries_of_orders(tx, &[order_id.to_owned()]).await?;
    for d in &deliveries {
        let lines_in: Vec<String> = match &d.item_shipped {
            Some(Value::Array(l)) => l
                .iter()
                .filter_map(|x| {
                    x.get("order_item_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .collect(),
            _ => lines.iter().map(|l| l.id.clone()).collect(),
        };
        let all_gone = lines_in.iter().all(|id| {
            fx.lines_cancelled.iter().any(|(x, _)| x == id)
                || lines
                    .iter()
                    .any(|l| &l.id == id && l.order_item_status == "cancelled")
        });
        if all_gone && !matches!(d.delivery_state.as_str(), "cancelled" | "delivered") {
            fx.parcel_changes.push(Change {
                op: ChangeOp::Update,
                entity: "parcel_delivery".into(),
                id: d.id.clone(),
                from: json!(d.delivery_state),
                to: json!("cancelled"),
                note: None,
            });
            fx.basis.push(Basis {
                entity: "fulfillment.parcel_delivery".into(),
                id: d.id.clone(),
                version: i64::from(d.version),
            });
            fx.deliveries_cancelled
                .push((d.id.clone(), d.delivery_state.clone()));
        }
    }
    Ok(fx)
}

/// Applies a step's effects besides the claim's own status: liability, cancelled lines,
/// stock, parcels, coupons, the order's status and the refund.
pub(crate) async fn apply_effects(
    tx: &mut TxCx,
    s: &ApproveStep,
    instrument: &str,
) -> Result<(), ToolError> {
    if let Some((_, to)) = &s.liability_change {
        repo::set_liability(tx, &s.claim_id, to).await?;
    }
    for (line, _) in &s.lines_cancelled {
        sales::set_item_status(tx, line, "cancelled").await?;
    }
    for (variant, units) in &s.restock {
        catalog::move_stock(tx, variant, *units).await?;
    }
    for (delivery, _) in &s.deliveries_cancelled {
        repo::set_delivery_state(tx, delivery, "cancelled").await?;
    }
    promotion::restore_coupons(tx, &s.coupons_restored).await?;
    if s.order_cancelled_from.is_some() {
        sales::set_order_status(tx, &s.order_id, "cancelled", "CancelAction", instrument).await?;
    }
    if let Some(r) = &s.refund {
        repo::insert_refund(
            tx,
            &NewRefund {
                id: &r.refund_id,
                claim_id: &s.claim_id,
                order_id: &s.order_id,
                payment_id: r.payment_id.as_deref(),
                amount: r.amount,
                refund_method: &r.refund_method,
                refund_type: &r.refund_type,
            },
        )
        .await?;
    }
    Ok(())
}

pub(crate) fn not_yours() -> ToolError {
    ToolError::new(
        NOT_YOUR_ORDER,
        "Only the shopper who placed the order can do this.",
    )
    .detail(json!({"reason": "owner"}))
}

pub(crate) fn changed(what: &str, new: Value) -> ToolError {
    ToolError::new(
        ErrorCode::ESTIMATE_CHANGED,
        format!("The {what} changed since you looked; please check the new amount."),
    )
    .detail(json!({"new_preview": new}))
}

/// The pickup window the shopper picked.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WindowInput {
    /// Start, RFC 3339.
    #[schemars(example = &"2026-11-09T09:00:00-05:00")]
    pub start: String,
    /// End, RFC 3339.
    #[schemars(example = &"2026-11-09T12:00:00-05:00")]
    pub end: String,
}

/// Where money goes when the original payment can no longer be reversed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RefundDestination {
    /// A bank account the shopper saved.
    BankAccount,
    /// Store credit on the shopper's account.
    StoreCredit,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefundToInput {
    /// bank_account or store_credit.
    #[serde(rename = "type")]
    #[schemars(example = &"store_credit")]
    pub destination: RefundDestination,
    /// The saved account reference (never the account number itself); none for store credit.
    #[serde(rename = "ref")]
    #[schemars(example = &"bank_1")]
    pub reference: Option<String>,
}

impl RefundToInput {
    /// As recorded on the claim's event: `{type, ref}`.
    fn wire(&self) -> Value {
        let kind = match self.destination {
            RefundDestination::BankAccount => "bank_account",
            RefundDestination::StoreCredit => "store_credit",
        };
        json!({"type": kind, "ref": self.reference})
    }
}

/// Everything known about exchanging one line now.
pub(crate) struct ExchangeView {
    pub order: sales::Order,
    pub item: sales::OrderItem,
    /// The variant bought.
    pub variant: catalog::Variant,
    /// Every variant of the same product.
    pub siblings: Vec<catalog::Variant>,
    pub days_since_delivery: Option<i64>,
    /// An open claim on the line, if any.
    pub open_claim: Option<String>,
    /// Units already in return or exchange claims that were not declined or withdrawn.
    pub already_claimed: i32,
    /// Where the carrier collects the item: `(saved address id, address)`.
    pub pickup_address: (Option<String>, Value),
    /// Where the replacement goes.
    pub delivery_address: (Option<String>, Value),
    pub zones: Vec<Zone>,
}

fn address_json(a: &customer::Address) -> Value {
    json!({"name": a.name, "street_address": a.street_address,
           "address_locality": a.address_locality, "address_region": a.address_region,
           "postal_code": a.postal_code, "address_country": a.address_country})
}

/// A saved address of the order's shopper, or the address the order was delivered to.
async fn exchange_address(
    tx: &mut TxCx,
    order: &sales::Order,
    delivered_to: &Value,
    address_id: Option<&str>,
    field: &str,
) -> Result<(Option<String>, Value), ToolError> {
    let Some(id) = address_id else {
        return Ok((None, delivered_to.clone()));
    };
    let owner = order.customer_id.clone().unwrap_or_default();
    let a = customer::address(tx, &owner, id).await?.ok_or_else(|| {
        ToolError::invalid_input(format!("{id} is not one of the shopper's saved addresses."))
            .detail(json!({"field": field, "reason": "format"}))
    })?;
    Ok((Some(a.id.clone()), address_json(&a)))
}

/// Loads one line of the caller's order as an exchange sees it.
pub(crate) async fn exchange_view(
    tx: &mut TxCx,
    order_id: &str,
    order_item_id: &str,
    addresses: (Option<&str>, Option<&str>),
    now: chrono::DateTime<chrono::Utc>,
    rules: &Rules,
) -> Result<ExchangeView, ToolError> {
    let zone = tx.time.zone;
    let me = shopper(&tx.principal);
    let order = order_for(tx, order_id, me.as_deref(), "get_my_orders").await?;
    let item = sales::order_items(tx, order_id)
        .await?
        .into_iter()
        .find(|i| i.id == order_item_id)
        .ok_or_else(|| {
            ToolError::new(
                ORDER_ITEM_NOT_FOUND,
                format!("Order {} has no line {order_item_id}.", order.order_number),
            )
            .detail(json!({"entity": "order_item", "id": order_item_id}))
            .next("get_order")
        })?;
    let claims = repo::claims_on_order(tx, order_id).await?;
    let claim_ids: Vec<String> = claims.iter().map(|c| c.id.clone()).collect();
    let claim_items = repo::claim_items(tx, &claim_ids).await?;
    let on_line = |c: &ClaimRow| {
        claim_items
            .iter()
            .filter(|i| i.claim_id == c.id && i.order_item_id == item.id)
            .map(|i| i.order_quantity)
            .sum::<i32>()
    };
    let open_claim = claims
        .iter()
        .find(|c| c.is_open() && on_line(c) > 0)
        .map(|c| c.id.clone());
    let already_claimed = claims
        .iter()
        .filter(|c| {
            c.claim_type != "cancel" && !matches!(c.claim_status.as_str(), "declined" | "withdrawn")
        })
        .map(on_line)
        .sum();
    let deliveries = repo::deliveries_of_orders(tx, std::slice::from_ref(&order.id)).await?;
    let delivered_to = deliveries
        .iter()
        .filter(|d| d.claim_id.is_none() && d.delivery_state != "cancelled")
        .max_by_key(|d| d.delivered_at)
        .map(|d| d.delivery_address.clone())
        .unwrap_or(Value::Null);
    let variant = catalog::variant(tx, &item.variant_id)
        .await?
        .ok_or_else(|| crate::broken(format!("variant {} is missing", item.variant_id)))?;
    let siblings = catalog::product_variants(tx, &variant.product_id)
        .await?
        .unwrap_or_default();
    let pickup_address =
        exchange_address(tx, &order, &delivered_to, addresses.0, "pickup_address_id").await?;
    let delivery_address = exchange_address(
        tx,
        &order,
        &delivered_to,
        addresses.1,
        "delivery_address_id",
    )
    .await?;
    let zones = crate::shipping_rate::zones(tx, rules).await?;
    Ok(ExchangeView {
        days_since_delivery: delivered_at(&deliveries, &order.id).map(|d| days_since(d, now, zone)),
        order,
        item,
        variant,
        siblings,
        open_claim,
        already_claimed,
        pickup_address,
        delivery_address,
        zones,
    })
}

/// What an exchange of a line costs and offers, for the reason and units asked.
pub(crate) struct ExchangeQuote {
    pub liability: String,
    pub exchangeable: i32,
    pub choices: Vec<Choice>,
    pub cost: ExchangeCost,
    /// A person at the store confirms it (`exchange.pickup.operator_approval_when`).
    pub person: bool,
    pub pickup_window: Window,
    pub ships_on: chrono::NaiveDate,
    pub arrives_by: chrono::NaiveDate,
    pub reship_after: String,
    /// Days after delivery the shopper may ask (`exchange.window.self_service_days`).
    pub window_days: i64,
}

/// The zone of an address, or the store's first zone when none covers it.
fn zone_of_address<'a>(zones: &'a [Zone], address: &Value) -> Option<&'a Zone> {
    zone_for(
        zones,
        address
            .get("address_country")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        address.get("address_region").and_then(Value::as_str),
    )
    .or(zones.first())
}

/// The exchange of a loaded line, or why it cannot be exchanged.
pub(crate) fn exchange_quote(
    rules: &Rules,
    v: &ExchangeView,
    reason: &str,
    quantity: Option<i32>,
    now: chrono::DateTime<chrono::Utc>,
    time: &acom_core::StoreTime,
) -> Result<ExchangeQuote, ToolError> {
    if let Some(c) = &v.open_claim {
        return Err(ToolError::new(
            CLAIM_IN_PROGRESS,
            format!("This item already has an open request ({c})."),
        )
        .detail(json!({"existing_id": c}))
        .next("get_claim"));
    }
    let liability = exchange_liability(rules, reason).ok_or_else(|| {
        ToolError::invalid_input(format!("An exchange cannot have the reason {reason}.")).detail(
            json!({"field": "reason", "reason": "enum", "allowed": exchange_reasons(rules)}),
        )
    })?;
    let state = match (
        v.item.order_item_status.as_str(),
        v.item.fulfillment_stage.as_deref(),
    ) {
        ("delivered", _) => "delivered",
        ("in_transit", _) | (_, Some("shipped")) => "shipped",
        (other, _) => other,
    };
    if matches!(state, "cancelled" | "returned") {
        return Err(ToolError::new(
            NOT_EXCHANGEABLE,
            format!("This item is {state} and cannot be exchanged."),
        )
        .detail(json!({"rule": "exchange.eligible_states", "why": state}))
        .next("get_return_options"));
    }
    if !rules
        .list("exchange.eligible_states")
        .iter()
        .any(|s| s == state)
    {
        return Err(ToolError::new(
            NOT_SHIPPED_YET,
            "This item has not shipped yet; cancel it and order the other size instead.",
        )
        .detail(json!({"rule": "exchange.eligible_states", "from": state}))
        .next("get_cancel_options"));
    }
    if let Some(why) = not_exchangeable(rules, &v.variant.sale_type, &v.variant.keywords) {
        return Err(ToolError::new(
            NOT_EXCHANGEABLE,
            format!("This item cannot be exchanged ({}).", why.replace('_', " ")),
        )
        .detail(json!({"rule": "exchange.not_exchangeable_when", "why": why}))
        .next("get_return_options"));
    }
    let window = rules
        .lookup("exchange.window.self_service_days")
        .and_then(Value::as_i64)
        .unwrap_or(rules.return_window_days);
    if let Some(days) = v.days_since_delivery
        && days > window
    {
        return Err(ToolError::new(
            OUTSIDE_EXCHANGE_WINDOW,
            format!("Delivered {days} days ago; exchanges are accepted within {window} days."),
        )
        .detail(json!({"rule": "exchange.window", "days_since_delivery": days, "limit": window}))
        .next("get_return_options"));
    }
    let exchangeable = v.item.order_quantity - v.already_claimed;
    if exchangeable <= 0 {
        return Err(ToolError::new(
            NOT_EXCHANGEABLE,
            "Every unit of this item is already returned or exchanged.",
        )
        .detail(json!({"rule": "exchange.scope", "why": "returned"}))
        .next("get_claim"));
    }
    let q = quantity.unwrap_or(exchangeable);
    if q < 1 || q > exchangeable {
        return Err(ToolError::invalid_input(format!(
            "Up to {exchangeable} of this item can be exchanged."
        ))
        .detail(json!({"field": "quantity", "reason": "range", "max": exchangeable})));
    }
    let candidates: Vec<Candidate> = v
        .siblings
        .iter()
        .map(|s| Candidate {
            variant_id: s.id.clone(),
            size: s.size.clone(),
            color: s.color.clone(),
            availability: s
                .stock
                .as_ref()
                .map_or_else(|| "discontinued".to_owned(), |x| x.availability.clone()),
            available_quantity: s
                .is_managed
                .then(|| s.stock.as_ref().map_or(0, |x| x.inventory_level)),
            price: s.stock.as_ref().map(|x| x.price),
            sale_type: s.sale_type.clone(),
        })
        .collect();
    let reference = v.variant.stock.as_ref().map_or(v.item.price, |s| s.price);
    let list = choices(rules, &liability, &v.variant.id, reference, q, &candidates);
    if list.is_empty() {
        return Err(ToolError::new(
            REPLACEMENT_UNAVAILABLE,
            "There is no other size or color of this product to exchange for; a return is possible instead.",
        )
        .detail(json!({"rule": "exchange.replacement"}))
        .next("get_return_options"));
    }
    let fallback = default_zone(rules);
    let pickup_zone = zone_of_address(&v.zones, &v.pickup_address.1).unwrap_or(&fallback);
    let reship_zone = zone_of_address(&v.zones, &v.delivery_address.1).unwrap_or(&fallback);
    let cost = exchange_cost(rules, reason, &liability, pickup_zone, reship_zone);
    let reship_after = rules
        .lookup("exchange.reship_after")
        .and_then(Value::as_str)
        .unwrap_or("inspected")
        .to_owned();
    let (_, ships_on, arrives_by) = exchange_dates(
        rules,
        now,
        time.zone,
        reship_zone.transit_business_days,
        &reship_after,
    );
    let (start, end) = pickup_window(rules, now, time.zone);
    Ok(ExchangeQuote {
        person: needs_person(rules, reason),
        liability,
        exchangeable,
        choices: list,
        cost,
        pickup_window: Window {
            start: time.format(start),
            end: time.format(end),
        },
        ships_on,
        arrives_by,
        reship_after,
        window_days: window,
    })
}

/// Refunds the extra charge a shopper paid on a claim that will not go ahead (a withdrawn
/// exchange, or one converted to a return: `exchange.on_fallback.refund_paid_charge`). The
/// money goes back the way it came, outside the order's payments. Returns the amount, or `None`
/// when nothing was paid.
pub(crate) async fn refund_paid_charge(
    tx: &mut TxCx,
    c: &ClaimRow,
    refund_id: &str,
    instrument: &str,
) -> Result<Option<Money>, ToolError> {
    let (Some(charge), Some(_)) = (c.charge()?, c.charge_paid_at) else {
        return Ok(None);
    };
    if repo::refund_of_claim(tx, &c.id).await?.is_some() {
        return Ok(Some(charge));
    }
    repo::insert_refund(
        tx,
        &NewRefund {
            id: refund_id,
            claim_id: &c.id,
            order_id: &c.order_id,
            payment_id: None,
            amount: charge,
            refund_method: "original_payment",
            refund_type: "full_refund",
        },
    )
    .await?;
    repo::append_refund_event(
        tx,
        refund_id,
        &repo::ClaimEvent {
            action_type: "RefundAction",
            action_status: "active",
            instrument,
            from_state: None,
            to_state: Some("requested"),
            result: Some(json!({"reason": "charge_refunded", "amount": charge,
                                "payment_provider": crate::refund::retry_refund::PROVIDER_NOTE})),
        },
    )
    .await?;
    Ok(Some(charge))
}
