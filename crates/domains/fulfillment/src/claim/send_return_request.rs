//! Person-only `POST /v1/fulfillment/returns`: the shopper sends a return request (F5). Not a
//! tool; it needs `human:return`, computes the options again and refuses with
//! `estimate_changed` when the shopper saw something else.

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot,
    ToolError,
};
use acom_core::Money;
use acom_core::enums::ClaimReason;
use acom_core::ids::ClaimId;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{RefundToInput, WindowInput, changed, not_yours, options_for, return_view};
use crate::errors::{
    CLAIM_IN_PROGRESS, EVIDENCE_REQUIRED, NOT_RETURNABLE, NOT_SHIPPED_YET, NOT_YOUR_ORDER,
    OUTSIDE_RETURN_WINDOW, REPLACEMENT_UNAVAILABLE,
};
use crate::repo::{self, ClaimEvent, NewClaim};
use crate::{rules, shopper};

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
                charge: None,
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
