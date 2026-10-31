//! `prepare_order` (`sales.order.prepare`): the checkout card's order, from the caller's own
//! cart. Preview only; the shopper's Place order button orders (`place_order`).

use acom_contract::{
    ActionType, Audience, Committed, Group, Kind, Method, OpMeta, Operation, Planned, Slot,
    ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::PaymentMethod;
use acom_core::ids::OrderId;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::{
    DeliveryMode, PostalAddressInput, PreparedOrder, checkout_view, create_order_change,
    delivery_choice, floor_multiplier, planned_lines,
};
use crate::checkout;
use crate::rules;
use crate::{
    ADDRESS_NOT_DELIVERABLE, ADDRESS_NOT_FOUND, CART_EMPTY, MIXED_ORDER_TYPE,
    NO_COMMON_PAYMENT_METHOD, NOT_SIGNED_IN, OUT_OF_STOCK, approval,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrepareOrderInput {
    /// `saved`: a saved address (`delivery_address_id`); `one_time`: an address for this order
    /// only (`delivery_address`, not saved).
    #[schemars(example = &"saved")]
    pub delivery_mode: DeliveryMode,
    /// A saved address (`get_my_profile`, `list_addresses`). Needed with `saved`.
    #[schemars(regex(pattern = r"^adr_[a-z0-9_]+$"), example = &"adr_home")]
    pub delivery_address_id: Option<String>,
    /// The address for this order. Needed with `one_time`.
    #[schemars(example = json!({"name": "Jamie", "street_address": "12 Harbor St, Apt 3",
        "address_locality": "Brooklyn", "address_region": "NY", "postal_code": "11201",
        "address_country": "US"}))]
    pub delivery_address: Option<PostalAddressInput>,
    /// How to pay. The saved default when left out.
    #[schemars(example = &"credit_card")]
    pub payment_method: Option<PaymentMethod>,
    /// The total the shopper saw on screen; the server recalculates and compares.
    #[schemars(example = json!({"value": 71.20, "currency": "USD"}))]
    pub client_total: Money,
}

pub(crate) struct PrepareOrder;

impl Operation for PrepareOrder {
    const META: OpMeta = OpMeta {
        address: "sales.order.prepare",
        tool: "prepare_order",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/sales/orders/prepare",
        summary: "Prepare an order from the caller's own cart for the checkout card: lines, discounts, shipping, total, delivery and payment, re-checked by the server. Preview only; only the shopper's Place order button orders.",
        consumes: &[Slot::one("cart_id")],
        produces: &[Slot::one("preview_id")],
        checks: &[
            "checkout_validation.common_payment_method_exists",
            "checkout_validation.stock_available",
            "checkout_validation.product_sellable",
            "checkout_validation.no_domestic_international_mix",
            "checkout_validation.server_total_matches_client",
            "checkout_validation.price_floor_respected",
        ],
        errors: &[
            CART_EMPTY,
            NO_COMMON_PAYMENT_METHOD,
            OUT_OF_STOCK,
            ADDRESS_NOT_FOUND,
            ADDRESS_NOT_DELIVERABLE,
            MIXED_ORDER_TYPE,
            NOT_SIGNED_IN,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::ORDER,
        person_scope: "",
    };
    type Input = PrepareOrderInput;
    type Output = serde_json::Value;
}

fn choice(input: &PrepareOrderInput) -> Result<crate::pricing::DeliveryChoice, ToolError> {
    delivery_choice(
        input.delivery_mode,
        input.delivery_address_id.as_ref(),
        input.delivery_address.as_ref(),
    )
}

impl WriteOp<Cx> for PrepareOrder {
    type Plan = PreparedOrder;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: PrepareOrderInput,
    ) -> Result<Planned<PreparedOrder>, ToolError> {
        let delivery = choice(&input)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (carts, view) =
            checkout_view(&mut tx, &delivery, input.payment_method.map(|m| m.as_str())).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let required = rules::checkout_checks(&cx.policy);
        let checks = checkout::checkout_checks(
            &view,
            &required,
            input.client_total,
            floor_multiplier(&cx.policy),
        );
        let payment_method = view
            .payment
            .as_ref()
            .map(|p| p.payment_method.clone())
            .unwrap_or_default();
        let plan = PreparedOrder {
            order_id: OrderId::generate().to_string(),
            customer_id: view.customer_id.clone().unwrap_or_default(),
            lines: planned_lines(&view),
            delivery,
            payment_method,
            total: view.breakdown.total,
            failed_checks: checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| c.rule.clone())
                .collect(),
        };
        Ok(Planned {
            changes: vec![create_order_change(&view)],
            checks,
            basis: carts.basis(),
            approval: approval(
                cx,
                Self::META.tool,
                &json!({"amount": view.breakdown.total}),
            ),
            plan,
        })
    }

    /// Never run by the preview engine (`approval.preview_only`): the shopper's button places
    /// the order through [`PlaceOrder`].
    async fn commit(&self, _: &mut TxCx, _: &PreparedOrder) -> Result<Committed, ToolError> {
        Err(preview_only())
    }

    async fn verify(
        &self,
        _: &Cx,
        _: &PreparedOrder,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        Err(preview_only())
    }

    async fn undo(&self, _: &mut TxCx, _: &PreparedOrder, _: &Committed) -> Result<(), ToolError> {
        Err(preview_only())
    }
}

fn preview_only() -> ToolError {
    ToolError::invalid_input(
        "`prepare_order` only prepares; the shopper places the order with POST /v1/sales/orders.",
    )
    .detail(json!({"field": "preview_id", "reason": "preview_only"}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed_and_flat() {
        let p = closed_schema_for::<PrepareOrderInput>();
        assert!(violations(&p).is_empty(), "{p:#}");
        assert_eq!(p["required"], json!(["delivery_mode", "client_total"]));
        assert_eq!(
            p["properties"]["delivery_address"]["additionalProperties"],
            json!(false)
        );
    }
}
