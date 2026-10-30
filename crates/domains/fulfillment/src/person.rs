//! Person-only endpoints (docs/api/fulfillment.md, 사람 전용 엔드포인트): the shopper's own
//! buttons. None is a tool; each needs a `human:*` scope, so no agent can call them. Each one
//! computes the options again and refuses with `estimate_changed` when the shopper saw
//! something else. Errors carry no next tool (the HTTP layer also clears it).

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot,
    ToolError,
};
use acom_core::Money;
use acom_core::enums::{CancelReason, ClaimReason};
use acom_core::ids::{ClaimId, RefundId};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::decide::{ApproveStep, PlannedRefund, apply_effects, cancel_effects};
use crate::errors::{
    ALREADY_CANCELLED, ALREADY_SHIPPED, CLAIM_IN_PROGRESS, CLAIM_NOT_FOUND, EVIDENCE_REQUIRED,
    NOT_CANCELLABLE, NOT_RETURNABLE, NOT_REVOCABLE, NOT_SHIPPED_YET, NOT_YOUR_ORDER,
    OUTSIDE_RETURN_WINDOW, QUANTITY_OVER_CANCELLABLE, REASON_NOT_SELECTABLE,
    REFUND_DESTINATION_REQUIRED, REPLACEMENT_UNAVAILABLE, WHOLE_QUANTITY_ONLY,
};
use crate::model::cancel::{CancelAsk, cancel_outcome};
use crate::options::{
    CancelItemInput, asked_items, cancel_block_error, cancel_view, options_for, return_view,
};
use crate::repo::{self, ClaimEvent, NewClaim};
use crate::{order_for, rules, shopper};

fn not_yours() -> ToolError {
    ToolError::new(
        NOT_YOUR_ORDER,
        "Only the shopper who placed the order can do this.",
    )
    .detail(json!({"reason": "owner"}))
}

fn changed(what: &str, new: Value) -> ToolError {
    ToolError::new(
        ErrorCode::ESTIMATE_CHANGED,
        format!("The {what} changed since you looked; please check the new amount."),
    )
    .detail(json!({"new_preview": new}))
}

/// One line and units to send back.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReturnItemInput {
    /// The order line.
    #[schemars(regex(pattern = r"^oln_[a-z0-9_]+$"), example = &"oln_20402_1")]
    pub order_item_id: String,
    /// Units to return.
    #[schemars(range(min = 1), example = 1)]
    pub quantity: u32,
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

/// Replacement or refund (`get_return_options.items[].kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReturnKind {
    /// Money back to the original payment.
    Refund,
    /// The same variant again, new (an exchange claim).
    Replacement,
}

/// How the item goes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SendBackMethod {
    /// A carrier picks it up in the chosen window.
    Pickup,
    /// The shopper drops it off.
    DropOff,
}

impl SendBackMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pickup => "pickup",
            Self::DropOff => "drop_off",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendReturnRequestInput {
    /// The shopper's own order.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20402")]
    pub order_id: String,
    /// Lines and units to send back.
    #[schemars(length(min = 1), example = json!([{"order_item_id": "oln_20402_1", "quantity": 1}]))]
    pub items: Vec<ReturnItemInput>,
    /// The reason given to get_return_options.
    #[schemars(example = &"defect")]
    pub reason: ClaimReason,
    /// refund or replacement, as chosen from get_return_options.
    #[schemars(example = &"refund")]
    pub kind: ReturnKind,
    /// The shopper's own words.
    #[schemars(length(max = 500), example = &"The sole is coming apart after a week.")]
    pub reason_text: Option<String>,
    /// Photos uploaded first; needed for damage in transit.
    #[schemars(inner(regex(pattern = r"^evd_[a-z0-9_]+$")), example = ["evd_877", "evd_878"])]
    pub evidence_ids: Option<Vec<String>>,
    /// pickup or drop_off.
    #[schemars(example = &"pickup")]
    pub send_back_method: SendBackMethod,
    /// The offered pickup window, when the method is pickup.
    pub pickup_window: Option<WindowInput>,
    /// The refund the shopper confirmed, when the kind is refund.
    #[schemars(example = json!({"value": 312.00, "currency": "USD"}))]
    pub expected_refund: Option<Money>,
    /// What the shopper confirmed paying (default 0.00).
    #[schemars(example = json!({"value": 0.00, "currency": "USD"}))]
    pub expected_charge: Option<Money>,
    /// Only when the original payment can no longer be reversed.
    pub refund_to: Option<RefundToInput>,
}

/// `POST /v1/fulfillment/returns`: the shopper sends a return request (F5).
pub struct SendReturnRequest;

impl Operation for SendReturnRequest {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.send_return_request",
        tool: "send_return_request",
        group: Group::Transact,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/fulfillment/returns",
        summary: "The shopper sends a return request chosen from get_return_options: a refund becomes a return claim, a replacement an exchange claim.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[],
        errors: &[
            OUTSIDE_RETURN_WINDOW,
            NOT_RETURNABLE,
            NOT_SHIPPED_YET,
            EVIDENCE_REQUIRED,
            ErrorCode::ESTIMATE_CHANGED,
            CLAIM_IN_PROGRESS,
            NOT_YOUR_ORDER,
            REPLACEMENT_UNAVAILABLE,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::RETURN,
        person_scope: "human:return",
    };
    type Input = SendReturnRequestInput;
    type Output = Value;
}

impl PersonOp<Cx> for SendReturnRequest {
    async fn run(&self, tx: &mut TxCx, input: SendReturnRequestInput) -> Result<Value, ToolError> {
        let rules = rules(&tx.policy)?;
        let me = shopper(&tx.principal).ok_or_else(not_yours)?;
        let now = tx.clock.now();
        let time = tx.time;
        let reason = input.reason.as_str();
        let evidence = input.evidence_ids.clone().unwrap_or_default();
        if reason == "damaged_in_transit" && rules.damage_claim_needs_photo && evidence.is_empty() {
            return Err(
                ToolError::new(EVIDENCE_REQUIRED, "Add a photo of the damage first.")
                    .detail(json!({"rule": "damage_claim_needs_photo", "field": "evidence_ids"})),
            );
        }
        let method = input.send_back_method.as_str();
        if !rules.send_back_methods.iter().any(|m| m == method) {
            return Err(
                ToolError::invalid_input(format!("{method} is not offered.")).detail(
                    json!({"field": "send_back_method", "allowed": rules.send_back_methods}),
                ),
            );
        }

        let mut lines = Vec::new();
        let mut refund = None::<Money>;
        let mut you_pay = None::<Money>;
        let mut offered_window = None;
        let mut confirmed_by = "store_automatic".to_owned();
        let mut replacements = Vec::new();
        let mut payment = None;
        let mut order_number = String::new();
        for item in &input.items {
            let q = i32::try_from(item.quantity).unwrap_or(i32::MAX);
            let view = return_view(
                tx,
                &input.order_id,
                &item.order_item_id,
                Some(q),
                now,
                time.zone,
            )
            .await?;
            if view.order.customer_id.as_deref() != Some(me.as_str()) {
                return Err(not_yours());
            }
            order_number = view.order.order_number.clone();
            let (q, options) = options_for(&rules, &tx.policy, &view, reason, Some(q), now, &time)?;
            let kind = match input.kind {
                ReturnKind::Refund => "refund",
                ReturnKind::Replacement => "replacement",
            };
            let chosen = options
                .into_iter()
                .find(|o| o.kind == kind)
                .ok_or_else(|| {
                    ToolError::new(
                        REPLACEMENT_UNAVAILABLE,
                        "A replacement is not offered for this item; choose a refund.",
                    )
                })?;
            if chosen.confirmed_by == "store_person" {
                confirmed_by = "store_person".into();
            }
            let add = |a: Option<Money>, b: Money| match a {
                None => Some(b),
                Some(a) => a.checked_add(b).ok(),
            };
            if let Some(r) = chosen.refund_amount {
                refund = add(refund, r);
            }
            you_pay = add(you_pay, chosen.you_pay);
            offered_window = chosen.send_back.pickup_window.clone();
            if kind == "replacement" {
                replacements.push((view.item.id.clone(), view.item.variant_id.clone(), q));
            }
            payment = view.payment.clone();
            lines.push((view.line.line.clone(), q));
        }
        let currency = lines
            .first()
            .map(|(l, _)| l.unit_price.currency)
            .ok_or_else(|| ToolError::invalid_input("Choose at least one item to return."))?;

        // The shopper confirmed amounts: refuse when they no longer hold.
        let new_preview = json!({"refund_amount": refund, "you_pay": you_pay,
                                 "pickup_window": offered_window});
        if input.kind == ReturnKind::Refund {
            match (&input.expected_refund, &refund) {
                (Some(seen), Some(now_amount)) if seen.value == now_amount.value => {}
                (None, _) => {
                    return Err(ToolError::invalid_input("Send the refund you confirmed.")
                        .detail(json!({"field": "expected_refund", "reason": "missing"})));
                }
                _ => return Err(changed("refund", new_preview)),
            }
        }
        let seen_charge = input
            .expected_charge
            .map_or(rust_decimal::Decimal::ZERO, |m| m.value);
        if you_pay.map_or(rust_decimal::Decimal::ZERO, |m| m.value) != seen_charge {
            return Err(changed("cost", new_preview));
        }
        let pickup_window = match input.send_back_method {
            SendBackMethod::Pickup => {
                let Some(w) = &input.pickup_window else {
                    return Err(ToolError::invalid_input("Pick a pickup window.")
                        .detail(json!({"field": "pickup_window", "reason": "missing"})));
                };
                let same = offered_window.as_ref().is_some_and(|o| {
                    time.parse(&w.start).ok() == time.parse(&o.start).ok()
                        && time.parse(&w.end).ok() == time.parse(&o.end).ok()
                });
                if !same {
                    return Err(changed("pickup window", new_preview));
                }
                offered_window
                    .as_ref()
                    .map(|o| json!({"start": o.start, "end": o.end}))
            }
            SendBackMethod::DropOff => None,
        };

        let liability = rules.liability_for(reason);
        let amount = lines.iter().fold(Money::zero(currency), |a, (l, _)| {
            Money::new(a.value + l.paid(rules.coupon_deducted).value, currency)
        });
        let claim_id = ClaimId::generate().to_string();
        let claim_type = if input.kind == ReturnKind::Refund {
            "return"
        } else {
            "exchange"
        };
        let evidence_type = match reason {
            "damaged_in_transit" => "damage_photo",
            "defect" | "not_as_described" | "missing_parts" | "wrong_item_sent"
            | "wrong_option_sent" => "product_defect",
            _ => "other",
        };
        repo::insert_claim(
            tx,
            &NewClaim {
                id: &claim_id,
                order_id: &input.order_id,
                customer_id: &me,
                claim_type,
                claim_status: "requested",
                claim_path: if confirmed_by == "store_person" { "store_review" } else { "self_service" },
                claim_reason: Some(reason),
                cancel_reason: None,
                reason_text: input.reason_text.as_deref().map(str::trim).filter(|t| !t.is_empty()),
                liability: &liability,
                return_fees: Some(&rules.return_fees_for(&liability)),
                send_back_method: Some(method),
                pickup_window: pickup_window.clone(),
                evidence: json!(evidence
                    .iter()
                    .map(|id| json!({"id": id, "evidence_type": evidence_type, "content_url": null}))
                    .collect::<Vec<_>>()),
                amount,
                refund_estimate: refund,
                items: lines
                    .iter()
                    .map(|(l, q)| {
                        let replacement = replacements
                            .iter()
                            .find(|(id, _, _)| id == &l.order_item_id)
                            .map(|(_, v, _)| v.clone());
                        (l.order_item_id.clone(), *q, replacement)
                    })
                    .collect(),
            },
        )
        .await?;
        // An exchange holds its replacement from the request on (`exchange.replacement`).
        let mut held = Vec::new();
        for (line, variant, q) in &replacements {
            catalog::move_stock(tx, variant, -q).await?;
            held.push(json!({"order_item_id": line, "variant_id": variant, "reserved": q}));
        }
        repo::append_event(
            tx,
            &claim_id,
            &ClaimEvent {
                action_type: "ReturnAction",
                action_status: "potential",
                instrument: "POST /v1/fulfillment/returns",
                from_state: None,
                to_state: Some("requested"),
                result: Some(json!({"order_number": order_number, "kind": claim_type,
                                    "refund_to": input.refund_to.as_ref().map(RefundToInput::wire)})),
            },
        )
        .await?;

        let mut out = json!({
            "ok": true,
            "id": claim_id,
            "claim_type": claim_type,
            "status": "requested",
            "send_back": {"method": method, "cost": you_pay.unwrap_or(Money::zero(currency)),
                          "pickup_window": pickup_window},
            "confirmed_by": confirmed_by,
            "undo": null,
        });
        if let Some(total) = refund {
            out["refund_estimate"] = json!({
                "total": total,
                "breakdown": [{"to": payment.as_ref().map(crate::payment_label)
                                         .unwrap_or_else(|| "the original payment".into()),
                               "amount": total}],
            });
        }
        if !held.is_empty() {
            out["replacement"] = json!(held);
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelMyOrderInput {
    /// The shopper's own order.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20455")]
    pub order_id: String,
    /// Lines and units; the whole order when left out.
    #[schemars(example = json!([{"order_item_id": "oln_1", "quantity": 1}]))]
    pub items: Option<Vec<CancelItemInput>>,
    /// A reason from get_cancel_options.reasons.
    #[schemars(example = &"changed_mind")]
    pub reason: CancelReason,
    /// The shopper's own words.
    #[schemars(length(max = 500), example = &"Ordered the wrong color")]
    pub reason_text: Option<String>,
    /// The refund the shopper confirmed (`estimate.refund_total`).
    #[schemars(example = json!({"value": 65.20, "currency": "USD"}))]
    pub expected_refund: Money,
    /// The extra charge the shopper confirmed (`estimate.charge_due`, default 0.00).
    #[schemars(example = json!({"value": 0.00, "currency": "USD"}))]
    pub expected_charge: Option<Money>,
    /// Only when `needs_refund_destination` was true.
    pub refund_to: Option<RefundToInput>,
}

/// `POST /v1/fulfillment/orders/{order_id}/cancel`: the shopper cancels before shipment.
pub struct CancelMyOrder;

impl Operation for CancelMyOrder {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.cancel_my_order",
        tool: "cancel_my_order",
        group: Group::Transact,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/fulfillment/orders/{order_id}/cancel",
        summary: "The shopper cancels all or part of their order before it ships, as prepared with get_cancel_options.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[],
        errors: &[
            ALREADY_SHIPPED,
            NOT_CANCELLABLE,
            QUANTITY_OVER_CANCELLABLE,
            WHOLE_QUANTITY_ONLY,
            REASON_NOT_SELECTABLE,
            ErrorCode::ESTIMATE_CHANGED,
            REFUND_DESTINATION_REQUIRED,
            ALREADY_CANCELLED,
            CLAIM_IN_PROGRESS,
            NOT_YOUR_ORDER,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::CANCEL,
        person_scope: "human:cancel",
    };
    type Input = CancelMyOrderInput;
    type Output = Value;
}

impl PersonOp<Cx> for CancelMyOrder {
    async fn run(&self, tx: &mut TxCx, input: CancelMyOrderInput) -> Result<Value, ToolError> {
        let rules = rules(&tx.policy)?;
        let me = shopper(&tx.principal).ok_or_else(not_yours)?;
        let now = tx.clock.now();
        let order = order_for(tx, &input.order_id, Some(&me), "get_my_orders").await?;
        let view = cancel_view(tx, &order, None).await?;
        let asked = asked_items(&input.items);
        let outcome = cancel_outcome(
            &rules,
            &view,
            CancelAsk {
                items: asked.as_deref(),
                reason: Some(input.reason.as_str()),
                ..CancelAsk::default()
            },
            now,
        )
        .map_err(cancel_block_error)?;
        let e = &outcome.estimate;
        let seen_charge = input
            .expected_charge
            .map_or(rust_decimal::Decimal::ZERO, |m| m.value);
        if e.refund_total.value != input.expected_refund.value || e.charge_due.value != seen_charge
        {
            return Err(changed("estimate", json!(e)));
        }
        if e.needs_refund_destination && input.refund_to.is_none() {
            return Err(ToolError::new(
                REFUND_DESTINATION_REQUIRED,
                "The original payment can no longer be refunded; choose where the money goes.",
            )
            .detail(json!({"field": "refund_to", "allowed": rules.list("refund.refund_destination_types")})));
        }

        let claim_id = ClaimId::generate().to_string();
        let status = outcome.status.as_str();
        repo::insert_claim(
            tx,
            &NewClaim {
                id: &claim_id,
                order_id: &order.id,
                customer_id: &me,
                claim_type: "cancel",
                claim_status: status,
                claim_path: if status == "requested" {
                    "store_review"
                } else {
                    "self_service"
                },
                claim_reason: None,
                cancel_reason: Some(&outcome.reason),
                reason_text: input
                    .reason_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty()),
                liability: &outcome.liability,
                return_fees: None,
                send_back_method: None,
                pickup_window: None,
                evidence: json!([]),
                amount: e.cancelled_amount,
                refund_estimate: Some(e.refund_total),
                items: outcome
                    .chosen
                    .iter()
                    .map(|(id, q)| (id.clone(), *q, None))
                    .collect(),
            },
        )
        .await?;
        let goes_through = matches!(status, "completed" | "approved");
        let mut refund_id = None;
        if goes_through {
            let fx = cancel_effects(tx, &order.id, &view, &outcome.chosen).await?;
            let part = e
                .refund_breakdown
                .iter()
                .find(|p| !p.amount.value.is_zero())
                .or(e.refund_breakdown.first());
            let method = match (&input.refund_to, e.needs_refund_destination) {
                (Some(to), true) => match to.destination {
                    RefundDestination::BankAccount => "bank_account",
                    RefundDestination::StoreCredit => "store_credit",
                },
                _ => rules.refund_to.as_str(),
            };
            let refund =
                (status == "approved" && !e.refund_total.value.is_zero()).then(|| PlannedRefund {
                    refund_id: RefundId::generate().to_string(),
                    payment_id: (method == "original_payment")
                        .then(|| part.map(|p| p.payment_id.clone()))
                        .flatten(),
                    amount: e.refund_total,
                    refund_method: method.to_owned(),
                    refund_type: rules.defect_refund_type.clone(),
                    to: part.map(|p| p.to.clone()).unwrap_or_default(),
                });
            refund_id = refund.as_ref().map(|r| r.refund_id.clone());
            let step = ApproveStep {
                claim_id: claim_id.clone(),
                claim_type: "cancel".into(),
                order_id: order.id.clone(),
                from_status: status.into(),
                to_status: status.into(),
                liability_change: None,
                lines_cancelled: fx.lines_cancelled,
                restock: fx.restock,
                deliveries_cancelled: fx.deliveries_cancelled,
                coupons_restored: e
                    .coupons_restored
                    .iter()
                    .map(|c| c.member_coupon_id.clone())
                    .collect(),
                order_cancelled_from: (e.scope == "full").then(|| order.order_status.clone()),
                refund,
            };
            apply_effects(tx, &step, "POST /v1/fulfillment/orders/{order_id}/cancel").await?;
        }
        repo::append_event(
            tx,
            &claim_id,
            &ClaimEvent {
                action_type: "CancelAction",
                action_status: if goes_through { "completed" } else { "potential" },
                instrument: "POST /v1/fulfillment/orders/{order_id}/cancel",
                from_state: None,
                to_state: Some(status),
                result: Some(json!({"refund_id": refund_id, "refund_total": e.refund_total,
                                    "refund_to": input.refund_to.as_ref().map(RefundToInput::wire)})),
            },
        )
        .await?;

        let mut out = json!({
            "ok": true,
            "id": claim_id,
            "claim_type": "cancel",
            "status": status,
            "order_id": order.id,
            "items": outcome.chosen.iter()
                .map(|(id, q)| json!({"order_item_id": id, "quantity": q}))
                .collect::<Vec<_>>(),
            "liability": outcome.liability,
            "refund": {
                "total": e.refund_total,
                "breakdown": e.refund_breakdown.iter()
                    .filter(|p| !p.amount.value.is_zero())
                    .map(|p| json!({"to": p.to, "amount": p.amount}))
                    .collect::<Vec<_>>(),
                "money_back_business_days": e.money_back_business_days,
            },
            "extra_charge": e.extra_charge.iter()
                .map(|x| json!({"kind": x.kind, "amount": x.amount,
                                "collected": if e.charge_due.value.is_zero() { "deducted_from_refund" } else { "charge_due" }}))
                .collect::<Vec<_>>(),
            "coupons_restored": e.coupons_restored,
            "points_pending_cancelled": e.points_pending_cancelled,
            "confirmed_by": e.confirmed_by,
            "undo": null,
        });
        if let Some(id) = refund_id {
            out["refund_id"] = json!(id);
        }
        if status == "charge_due" {
            out["pay_charge"] = json!({
                "amount": e.charge_due,
                "endpoint": format!("POST /v1/fulfillment/claims/{claim_id}/pay-charge"),
            });
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WithdrawClaimInput {
    /// The shopper's own return or exchange request.
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_3301")]
    pub claim_id: String,
    /// Why, in the shopper's words.
    #[schemars(length(max = 500), example = &"Found my size after all.")]
    pub reason_text: Option<String>,
}

/// `POST /v1/fulfillment/claims/{claim_id}/withdraw`: the shopper takes back a return or
/// exchange request before its pickup label exists.
pub struct WithdrawClaim;

impl Operation for WithdrawClaim {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.withdraw",
        tool: "withdraw_claim",
        group: Group::Transact,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/fulfillment/claims/{claim_id}/withdraw",
        summary: "The shopper withdraws their own return or exchange request, whole, before the pickup label is made. Cancels cannot be withdrawn.",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[],
        errors: &[
            ErrorCode::INVALID_STATE,
            NOT_REVOCABLE,
            NOT_YOUR_ORDER,
            CLAIM_NOT_FOUND,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::UPDATE,
        person_scope: "human:claim",
    };
    type Input = WithdrawClaimInput;
    type Output = Value;
}

impl PersonOp<Cx> for WithdrawClaim {
    async fn run(&self, tx: &mut TxCx, input: WithdrawClaimInput) -> Result<Value, ToolError> {
        let rules = rules(&tx.policy)?;
        let me = shopper(&tx.principal).ok_or_else(not_yours)?;
        let c = repo::claim_for_update(tx, &input.claim_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    CLAIM_NOT_FOUND,
                    format!("There is no request {}.", input.claim_id),
                )
                .detail(json!({"entity": "claim", "id": input.claim_id}))
            })?;
        if c.customer_id != me {
            return Err(not_yours());
        }
        let zero = Money::zero(c.amount()?.currency);
        if c.claim_status == "withdrawn" {
            return Ok(json!({"ok": true, "id": c.id, "status": "withdrawn",
                             "charge_refunded": zero, "undo": null}));
        }
        let allowed = match c.claim_type.as_str() {
            "cancel" => {
                return Err(ToolError::new(
                    NOT_REVOCABLE,
                    "A cancel cannot be withdrawn; place a new order instead.",
                )
                .detail(json!({"rule": "cancel.revocable"})));
            }
            "exchange" => &rules.exchange_withdraw_states,
            _ => &rules.return_withdraw_states,
        };
        if !allowed.contains(&c.claim_status) {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "This request is {} and can no longer be withdrawn.",
                    c.claim_status.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "claim", "from": c.claim_status, "allowed_to": []})));
        }
        if c.claim_type == "return" && repo::refund_of_claim(tx, &c.id).await?.is_some() {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                "The refund for this return has already been sent.",
            )
            .detail(json!({"entity": "claim", "from": c.claim_status})));
        }
        repo::move_claim(tx, &c.id, &c.claim_status, "withdrawn").await?;
        // Release a replacement the exchange was holding (`exchange.withdraw.on_withdraw`).
        if c.claim_type == "exchange" {
            for i in repo::claim_items(tx, std::slice::from_ref(&c.id)).await? {
                if let Some(v) = &i.replacement_product_id {
                    catalog::move_stock(tx, v, i.order_quantity).await?;
                }
            }
        }
        repo::append_event(
            tx,
            &c.id,
            &ClaimEvent {
                action_type: "UpdateAction",
                action_status: "completed",
                instrument: "POST /v1/fulfillment/claims/{claim_id}/withdraw",
                from_state: Some(&c.claim_status),
                to_state: Some("withdrawn"),
                result: Some(json!({"reason_text": input.reason_text})),
            },
        )
        .await?;
        // No extra charge is collected yet (pay-charge is not built), so none comes back.
        Ok(json!({"ok": true, "id": c.id, "status": "withdrawn",
                  "charge_refunded": zero, "undo": null}))
    }
}
