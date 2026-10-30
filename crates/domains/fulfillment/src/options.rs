//! The shopper's choices before sending anything: `get_return_options` and
//! `get_cancel_options`. They change nothing; the person-only endpoints in `person` compute the
//! same thing again when the shopper presses the button.

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::Money;
use acom_core::enums::{CancelReason, ClaimReason};
use acom_promotion::service::for_fulfillment as promotion;
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::errors::{
    ALREADY_CANCELLED, ALREADY_SHIPPED, CLAIM_IN_PROGRESS, NOT_RETURNABLE, NOT_SHIPPED_YET,
    NOT_YOUR_ORDER, ORDER_ITEM_NOT_FOUND, ORDER_NOT_FOUND, OUTSIDE_RETURN_WINDOW,
    QUANTITY_OVER_CANCELLABLE, REASON_NOT_SELECTABLE, WHOLE_QUANTITY_ONLY,
};
use crate::model::cancel::{
    CancelAsk, CancelBlock, CancelLine, CancelOrder, CouponLine, PaymentLine, cancel_outcome,
};
use crate::model::delivery::{days_since, estimate, local_date, pickup_window};
use crate::model::returns::{
    ClaimLine, OptionInputs, ReturnBlock, ReturnOption, ReturnableLine, Window, return_options,
    returnable,
};
use crate::model::{Rules, date_text};
use crate::repo::{self, ClaimRow};
use crate::{delivered_at, order_for, original_payment, payment_label, rules, shopper};

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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetReturnOptionsInput {
    /// The shopper's own order, from `get_my_orders`.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20402")]
    pub order_id: String,
    /// The order line to return.
    #[schemars(regex(pattern = r"^oln_[a-z0-9_]+$"), example = &"oln_20402_1")]
    pub order_item_id: String,
    /// Why. Store fault: defect, damaged_in_transit, not_as_described, wrong_item_sent,
    /// wrong_option_sent, missing_parts. Shopper's choice: wrong_size,
    /// different_option_wanted, changed_mind.
    #[schemars(example = &"defect")]
    pub reason: ClaimReason,
    /// Units to return, 1 or more. Every unit of the line when left out.
    #[schemars(range(min = 1), example = 1)]
    pub quantity: Option<u32>,
}

pub struct GetReturnOptions;

impl Operation for GetReturnOptions {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.get_return_options",
        tool: "get_return_options",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/orders/{order_id}/items/{order_item_id}/return-options",
        summary: "The shopper's choices for returning one of their order lines: a replacement or a refund, each with when it happens, what they pay, who confirms it, and how the item goes back. Only the shopper's button sends the request.",
        consumes: &[Slot::one("order_id"), Slot::one("order_item_id")],
        produces: &[],
        checks: &[],
        errors: &[
            ORDER_NOT_FOUND,
            ORDER_ITEM_NOT_FOUND,
            NOT_YOUR_ORDER,
            OUTSIDE_RETURN_WINDOW,
            NOT_RETURNABLE,
            NOT_SHIPPED_YET,
            CLAIM_IN_PROGRESS,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetReturnOptionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetReturnOptions {
    async fn read(&self, cx: &Cx, input: GetReturnOptionsInput) -> Result<Value, ToolError> {
        let rules = rules(&cx.policy)?;
        let now = cx.clock.now();
        let quantity = input.quantity.map(|q| i32::try_from(q).unwrap_or(i32::MAX));
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let view = return_view(
            &mut tx,
            &input.order_id,
            &input.order_item_id,
            quantity,
            now,
            cx.time.zone,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let (_, options) = options_for(
            &rules,
            &cx.policy,
            &view,
            input.reason.as_str(),
            quantity,
            now,
            &cx.time,
        )?;
        let total = options.len();
        Ok(json!({"items": options, "total": total}))
    }
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCancelOptionsInput {
    /// The shopper's own order.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20455")]
    pub order_id: String,
    /// Lines and units to try cancelling. The whole order when left out.
    #[schemars(example = json!([{"order_item_id": "oln_1", "quantity": 1}]))]
    pub items: Option<Vec<CancelItemInput>>,
    /// Why; decides who is at fault and so shipping and restores. changed_mind (the most
    /// careful estimate) when left out.
    #[schemars(example = &"changed_mind")]
    pub reason: Option<CancelReason>,
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

pub struct GetCancelOptions;

impl Operation for GetCancelOptions {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.get_cancel_options",
        tool: "get_cancel_options",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/orders/{order_id}/cancel-options",
        summary: "Prepare a cancel of the shopper's own order: which lines and units can be cancelled, the reasons they may pick, the refund estimate by payment method, restored coupons, any extra shipping charge, and who confirms. Only the shopper's button sends it.",
        consumes: &[Slot::one("order_id")],
        produces: &[],
        checks: &[],
        errors: &[
            ORDER_NOT_FOUND,
            NOT_YOUR_ORDER,
            ORDER_ITEM_NOT_FOUND,
            QUANTITY_OVER_CANCELLABLE,
            WHOLE_QUANTITY_ONLY,
            ALREADY_SHIPPED,
            ALREADY_CANCELLED,
            CLAIM_IN_PROGRESS,
            REASON_NOT_SELECTABLE,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::CANCEL,
        person_scope: "",
    };
    type Input = GetCancelOptionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCancelOptions {
    async fn read(&self, cx: &Cx, input: GetCancelOptionsInput) -> Result<Value, ToolError> {
        let rules = rules(&cx.policy)?;
        let me = shopper(&cx.principal);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let order = order_for(&mut tx, &input.order_id, me.as_deref(), "get_my_orders").await?;
        let view = cancel_view(&mut tx, &order, None).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let asked = asked_items(&input.items);
        let outcome = cancel_outcome(
            &rules,
            &view,
            CancelAsk {
                items: asked.as_deref(),
                reason: input.reason.map(CancelReason::as_str),
                ..CancelAsk::default()
            },
            cx.clock.now(),
        )
        .map_err(cancel_block_error)?;
        let total = outcome.items.len();
        let mut out = serde_json::to_value(&outcome)
            .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
        out["total"] = json!(total);
        Ok(out)
    }
}
