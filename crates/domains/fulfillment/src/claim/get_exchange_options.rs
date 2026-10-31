//! `get_exchange_options`: prepares an exchange of one of the shopper's order lines: the
//! variants it can be swapped for, the pickup and reshipment, and what the shopper pays.
//! Changes nothing; `POST /v1/fulfillment/exchanges` computes the same again.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::ClaimReason;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ExchangeQuote, ExchangeView, exchange_quote, exchange_view};
use crate::errors::{
    CLAIM_IN_PROGRESS, NOT_EXCHANGEABLE, NOT_SHIPPED_YET, NOT_YOUR_ORDER, ORDER_ITEM_NOT_FOUND,
    ORDER_NOT_FOUND, OUTSIDE_EXCHANGE_WINDOW, REPLACEMENT_UNAVAILABLE,
};
use crate::rules;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetExchangeOptionsInput {
    /// The shopper's own order, from `get_my_orders`.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20447")]
    pub order_id: String,
    /// The order line to exchange.
    #[schemars(regex(pattern = r"^oln_[a-z0-9_]+$"), example = &"oln_2")]
    pub order_item_id: String,
    /// Why. The shopper's choice: wrong_size, different_option_wanted. Store fault (the same
    /// variant, new, is allowed and the store pays shipping): defect, damaged_in_transit,
    /// missing_parts, wrong_item_sent, wrong_option_sent.
    #[schemars(example = &"wrong_size")]
    pub reason: ClaimReason,
    /// Units to exchange, 1 or more. Every unit of the line when left out.
    #[schemars(range(min = 1), example = 1)]
    pub quantity: Option<u32>,
    /// A saved address the carrier collects from; the delivery address when left out.
    #[schemars(regex(pattern = r"^adr_[a-z0-9_]+$"), example = &"adr_home")]
    pub pickup_address_id: Option<String>,
    /// A saved address the replacement goes to; the delivery address when left out.
    #[schemars(regex(pattern = r"^adr_[a-z0-9_]+$"), example = &"adr_home")]
    pub delivery_address_id: Option<String>,
}

pub struct GetExchangeOptions;

impl Operation for GetExchangeOptions {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.get_exchange_options",
        tool: "get_exchange_options",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/orders/{order_id}/items/{order_item_id}/exchange-options",
        summary: "Prepare an exchange of one of the shopper's order lines: the sizes and colors it can be swapped for (stock, price difference), the pickup window, when the replacement ships and arrives, and what the shopper pays. Only the shopper's button sends it.",
        consumes: &[Slot::one("order_id"), Slot::one("order_item_id")],
        produces: &[],
        checks: &[],
        errors: &[
            ORDER_NOT_FOUND,
            NOT_YOUR_ORDER,
            ORDER_ITEM_NOT_FOUND,
            NOT_SHIPPED_YET,
            OUTSIDE_EXCHANGE_WINDOW,
            NOT_EXCHANGEABLE,
            REPLACEMENT_UNAVAILABLE,
            CLAIM_IN_PROGRESS,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetExchangeOptionsInput;
    type Output = Value;
}

/// The options as the shopper sees them (also the `new_preview` of `estimate_changed`).
pub(crate) fn options_json(v: &ExchangeView, q: &ExchangeQuote) -> Value {
    let street = v
        .pickup_address
        .1
        .get("street_address")
        .cloned()
        .unwrap_or(Value::Null);
    let you_pay = q.cost.you_pay;
    let total = q.choices.len();
    json!({
        "items": q.choices,
        "order_item": {
            "order_item_id": v.item.id,
            "name": v.item.name,
            "exchangeable_quantity": q.exchangeable,
            "days_since_delivery": v.days_since_delivery,
            "window_days": q.window_days,
            "path": if q.person { "store_review" } else { "self_service" },
        },
        "liability": q.liability,
        "pickup": {"window": q.pickup_window, "address": street, "cost": q.cost.pickup},
        "reship": {"ships_after": q.reship_after, "ships_on": q.ships_on.to_string(),
                   "arrives_by": q.arrives_by.to_string(), "cost": q.cost.reship},
        "fee": q.cost.fee,
        "you_pay": you_pay,
        "pay_when": (!you_pay.value.is_zero()).then_some("before_accept"),
        "confirmed_by": if q.person { "store_person" } else { "store_automatic" },
        "if_not_possible": {"kind": "return", "next_tool": "get_return_options",
                            "why": "Used if the new size runs out before it ships, or you decline it"},
        "total": total,
    })
}

impl ReadOp<Cx> for GetExchangeOptions {
    async fn read(&self, cx: &Cx, input: GetExchangeOptionsInput) -> Result<Value, ToolError> {
        let rules = rules(&cx.policy)?;
        let now = cx.clock.now();
        let quantity = input.quantity.map(|q| i32::try_from(q).unwrap_or(i32::MAX));
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let view = exchange_view(
            &mut tx,
            &input.order_id,
            &input.order_item_id,
            (
                input.pickup_address_id.as_deref(),
                input.delivery_address_id.as_deref(),
            ),
            now,
            &rules,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let quote = exchange_quote(
            &rules,
            &view,
            input.reason.as_str(),
            quantity,
            now,
            &cx.time,
        )?;
        Ok(options_json(&view, &quote))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_take_reason_words_and_saved_address_ids() {
        assert!(
            serde_json::from_value::<GetExchangeOptionsInput>(json!({
                "order_id": "ord_20447", "order_item_id": "oln_2", "reason": "too_big"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<GetExchangeOptionsInput>(json!({
                "order_id": "ord_20447", "order_item_id": "oln_2", "reason": "wrong_size",
                "quantity": 1, "pickup_address_id": "adr_home"
            }))
            .is_ok()
        );
    }
}
