//! Person-only `POST /v1/fulfillment/exchanges`: the shopper sends an exchange chosen from
//! `get_exchange_options`. Not a tool; it needs `human:exchange`, computes the options again and
//! refuses with `estimate_changed` when the shopper saw something else. The replacement is held
//! from now on (`exchange.replacement.reserve_stock_on: request`).

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

use super::get_exchange_options::options_json;
use super::{WindowInput, changed, exchange_quote, exchange_view, not_yours};
use crate::errors::{
    CLAIM_IN_PROGRESS, EVIDENCE_REQUIRED, NOT_EXCHANGEABLE, NOT_SHIPPED_YET, NOT_YOUR_ORDER,
    OUTSIDE_EXCHANGE_WINDOW, PRICE_DIFFERS, QUANTITY_MISMATCH, REPLACEMENT_UNAVAILABLE,
};
use crate::model::exchange::first_status;
use crate::model::money_text;
use crate::model::returns::ClaimLine;
use crate::repo::{self, ClaimEvent, NewClaim};
use crate::{rules, shopper};

/// One line to exchange and what it becomes.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExchangeItemInput {
    /// The order line.
    #[schemars(regex(pattern = r"^oln_[a-z0-9_]+$"), example = &"oln_2")]
    pub order_item_id: String,
    /// Units to exchange; the replacement is the same number of units.
    #[schemars(range(min = 1), example = 1)]
    pub quantity: u32,
    /// The variant to receive instead: one `get_exchange_options` marked selectable.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_trail_runner_10_slate")]
    pub replacement_variant_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendExchangeRequestInput {
    /// The shopper's own order.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20447")]
    pub order_id: String,
    /// The lines to exchange.
    #[schemars(length(min = 1, max = 20), example = json!([{"order_item_id": "oln_2", "quantity": 1, "replacement_variant_id": "var_trail_runner_10_slate"}]))]
    pub items: Vec<ExchangeItemInput>,
    /// The reason given to get_exchange_options.
    #[schemars(example = &"wrong_size")]
    pub reason: ClaimReason,
    /// The shopper's own words.
    #[schemars(length(max = 500), example = &"Runs half a size small")]
    pub reason_text: Option<String>,
    /// Photos uploaded first; needed for damage or a defect when the policy asks for photos.
    #[schemars(inner(regex(pattern = r"^evd_[a-z0-9_]+$")), example = ["evd_901"])]
    pub evidence_ids: Option<Vec<String>>,
    /// A saved address the carrier collects from; the delivery address when left out.
    #[schemars(regex(pattern = r"^adr_[a-z0-9_]+$"), example = &"adr_home")]
    pub pickup_address_id: Option<String>,
    /// A saved address the replacement goes to; the delivery address when left out.
    #[schemars(regex(pattern = r"^adr_[a-z0-9_]+$"), example = &"adr_home")]
    pub delivery_address_id: Option<String>,
    /// The offered pickup window (`get_exchange_options.pickup.window`).
    pub pickup_window: Option<WindowInput>,
    /// What the shopper confirmed paying (`you_pay`).
    #[schemars(example = json!({"value": 0.00, "currency": "USD"}))]
    pub expected_charge: Money,
}

/// `POST /v1/fulfillment/exchanges`.
pub struct SendExchangeRequest;

impl Operation for SendExchangeRequest {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.send_exchange_request",
        tool: "send_exchange_request",
        group: Group::Transact,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/fulfillment/exchanges",
        summary: "The shopper sends an exchange chosen from get_exchange_options: the replacement is held, the shopper pays any shipping first, then the carrier collects the item and the replacement ships after inspection.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[],
        errors: &[
            NOT_SHIPPED_YET,
            OUTSIDE_EXCHANGE_WINDOW,
            NOT_EXCHANGEABLE,
            REPLACEMENT_UNAVAILABLE,
            PRICE_DIFFERS,
            QUANTITY_MISMATCH,
            EVIDENCE_REQUIRED,
            ErrorCode::ESTIMATE_CHANGED,
            CLAIM_IN_PROGRESS,
            NOT_YOUR_ORDER,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::RETURN,
        person_scope: "human:exchange",
    };
    type Input = SendExchangeRequestInput;
    type Output = Value;
}

const INSTRUMENT: &str = "POST /v1/fulfillment/exchanges";

impl PersonOp<Cx> for SendExchangeRequest {
    async fn run(
        &self,
        tx: &mut TxCx,
        input: SendExchangeRequestInput,
    ) -> Result<Value, ToolError> {
        let rules = rules(&tx.policy)?;
        let me = shopper(&tx.principal).ok_or_else(not_yours)?;
        let now = tx.clock.now();
        let time = tx.time;
        let reason = input.reason.as_str();
        let evidence = input.evidence_ids.clone().unwrap_or_default();
        if matches!(reason, "damaged_in_transit" | "defect")
            && rules.damage_claim_needs_photo
            && evidence.is_empty()
        {
            return Err(
                ToolError::new(EVIDENCE_REQUIRED, "Add a photo of the problem first.")
                    .detail(json!({"rule": "damage_claim_needs_photo", "field": "evidence_ids"})),
            );
        }
        let mut lines = Vec::new();
        let mut held = Vec::new();
        let mut first = None;
        let mut person = false;
        for item in &input.items {
            if held
                .iter()
                .any(|(l, _, _): &(String, String, i32)| l == &item.order_item_id)
            {
                return Err(ToolError::invalid_input("Name each line once.")
                    .detail(json!({"field": "items", "reason": "format"})));
            }
            let q = i32::try_from(item.quantity).unwrap_or(i32::MAX);
            let view = exchange_view(
                tx,
                &input.order_id,
                &item.order_item_id,
                (
                    input.pickup_address_id.as_deref(),
                    input.delivery_address_id.as_deref(),
                ),
                now,
                &rules,
            )
            .await?;
            if view.order.customer_id.as_deref() != Some(me.as_str()) {
                return Err(not_yours());
            }
            let quote = match exchange_quote(&rules, &view, reason, Some(q), now, &time) {
                Ok(quote) => quote,
                Err(e)
                    if e.code == "invalid_input"
                        && e.detail.as_ref().is_some_and(|d| d["field"] == "quantity") =>
                {
                    return Err(ToolError::new(
                        QUANTITY_MISMATCH,
                        format!("{} cannot be exchanged in that quantity.", view.item.name),
                    )
                    .detail(e.detail.map(|d| *d).unwrap_or(Value::Null)));
                }
                Err(e) => return Err(e),
            };
            let choice = quote
                .choices
                .iter()
                .find(|c| c.variant_id == item.replacement_variant_id);
            match choice {
                Some(c) if c.selectable => {}
                Some(c) if c.why.as_deref() == Some("price_differs") => {
                    return Err(ToolError::new(
                        PRICE_DIFFERS,
                        format!(
                            "{} costs {} more; exchanges are for the same price only.",
                            item.replacement_variant_id,
                            c.price_difference.map(money_text).unwrap_or_default()
                        ),
                    )
                    .detail(json!({"rule": "exchange.replacement.price_difference",
                                   "variant_id": item.replacement_variant_id})));
                }
                _ => {
                    return Err(ToolError::new(
                        REPLACEMENT_UNAVAILABLE,
                        "That size or color cannot be sent right now; choose another or a return.",
                    )
                    .detail(json!({"variant_id": item.replacement_variant_id,
                                   "new_preview": options_json(&view, &quote)})));
                }
            }
            person |= quote.person;
            lines.push(ClaimLine {
                order_item_id: view.item.id.clone(),
                name: view.item.name.clone(),
                unit_price: view.item.price,
                ordered: view.item.order_quantity,
                line_discount: view.item.discount,
                quantity: q,
            });
            held.push((view.item.id.clone(), item.replacement_variant_id.clone(), q));
            if first.is_none() {
                first = Some((view, quote));
            }
        }
        let Some((view, quote)) = first else {
            return Err(ToolError::invalid_input(
                "Choose at least one item to exchange.",
            ));
        };
        // Shipping is charged once per exchange: one pickup, one reshipment.
        let you_pay = quote.cost.you_pay;
        if you_pay.value != input.expected_charge.value
            || you_pay.currency != input.expected_charge.currency
        {
            return Err(changed("cost", options_json(&view, &quote)));
        }
        let window = &quote.pickup_window;
        if let Some(w) = &input.pickup_window {
            let same = time.parse(&w.start).ok() == time.parse(&window.start).ok()
                && time.parse(&w.end).ok() == time.parse(&window.end).ok();
            if !same {
                return Err(changed("pickup window", options_json(&view, &quote)));
            }
        }
        let status = first_status(&rules, you_pay, person);
        let currency = you_pay.currency;
        let amount = lines.iter().fold(Money::zero(currency), |a, l| {
            Money::new(a.value + l.paid(rules.coupon_deducted).value, currency)
        });
        let evidence_type = if reason == "damaged_in_transit" {
            "damage_photo"
        } else {
            "product_defect"
        };
        let claim_id = ClaimId::generate().to_string();
        let first_state = if status == "pickup_instructed" {
            "requested"
        } else {
            status
        };
        repo::insert_claim(
            tx,
            &NewClaim {
                id: &claim_id,
                order_id: &input.order_id,
                customer_id: &me,
                claim_type: "exchange",
                claim_status: first_state,
                claim_path: if person { "store_review" } else { "self_service" },
                claim_reason: Some(reason),
                cancel_reason: None,
                reason_text: input
                    .reason_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty()),
                liability: &quote.liability,
                return_fees: Some(&rules.return_fees_for(&quote.liability)),
                send_back_method: Some("pickup"),
                pickup_window: Some(json!({"start": window.start, "end": window.end})),
                evidence: json!(evidence
                    .iter()
                    .map(|id| json!({"id": id, "evidence_type": evidence_type, "content_url": null}))
                    .collect::<Vec<_>>()),
                amount,
                refund_estimate: None,
                charge: (!you_pay.value.is_zero()).then_some(you_pay),
                items: held
                    .iter()
                    .map(|(line, variant, q)| (line.clone(), *q, Some(variant.clone())))
                    .collect(),
            },
        )
        .await?;
        let mut replacement = Vec::new();
        for (line, variant, q) in &held {
            catalog::move_stock(tx, variant, -q).await?;
            replacement.push(json!({"order_item_id": line, "variant_id": variant, "reserved": q}));
        }
        repo::append_event(
            tx,
            &claim_id,
            &ClaimEvent {
                action_type: "ReturnAction",
                action_status: "potential",
                instrument: INSTRUMENT,
                from_state: None,
                to_state: Some(first_state),
                result: Some(
                    json!({"order_number": view.order.order_number, "kind": "exchange",
                                    "replacement": replacement,
                                    "pickup_address_id": view.pickup_address.0,
                                    "pickup_address": view.pickup_address.1,
                                    "delivery_address_id": view.delivery_address.0,
                                    "delivery_address": view.delivery_address.1,
                                    "charge": you_pay}),
                ),
            },
        )
        .await?;
        if status == "pickup_instructed" {
            // `exchange.pickup.instruction: automatic`: nothing to pay and nothing to confirm.
            repo::move_claim(tx, &claim_id, "requested", "pickup_instructed").await?;
            repo::append_event(
                tx,
                &claim_id,
                &ClaimEvent {
                    action_type: "AuthorizeAction",
                    action_status: "active",
                    instrument: INSTRUMENT,
                    from_state: Some("requested"),
                    to_state: Some("pickup_instructed"),
                    result: Some(json!({"rule": "exchange.pickup.instruction"})),
                },
            )
            .await?;
        }
        let mut out = json!({
            "ok": true,
            "id": claim_id,
            "claim_type": "exchange",
            "status": status,
            "liability": quote.liability,
            "replacement": replacement,
            "pickup": {"window": window, "cost": quote.cost.pickup},
            "reship": {"ships_after": quote.reship_after, "arrives_by": quote.arrives_by.to_string(),
                       "cost": quote.cost.reship},
            "you_pay": you_pay,
            "withdraw_until": "pickup_instructed",
            "confirmed_by": if person { "store_person" } else { "store_automatic" },
            "undo": null,
        });
        if status == "charge_due" {
            out["pay_charge"] = json!({
                "amount": you_pay,
                "endpoint": format!("POST /v1/fulfillment/claims/{claim_id}/pay-charge"),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_names_a_replacement_for_each_line() {
        assert!(
            serde_json::from_value::<SendExchangeRequestInput>(json!({
                "order_id": "ord_20447",
                "items": [{"order_item_id": "oln_2", "quantity": 1}],
                "reason": "wrong_size", "expected_charge": {"value": 0.00, "currency": "USD"}
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<SendExchangeRequestInput>(json!({
                "order_id": "ord_20447",
                "items": [{"order_item_id": "oln_2", "quantity": 1,
                           "replacement_variant_id": "var_trail_runner_10_slate"}],
                "reason": "wrong_size", "expected_charge": {"value": 0.00, "currency": "USD"}
            }))
            .is_ok()
        );
    }
}
