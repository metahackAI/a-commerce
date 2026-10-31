//! `prepare_gift_order` (`sales.gift_order.prepare`): a gift from the caller's own cart, for
//! the sender's Place order button. Preview only.

use acom_contract::{
    ActionType, Audience, Committed, Group, Kind, Method, OpMeta, Operation, Planned, Slot,
    ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::ids::OrderId;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    CARD_MESSAGE_CHARS, GiftAddressInput, PREPARE_TOOL, PreparedGift, RecipientMode, checks,
    exchange_date, total_of,
};
use crate::order::{self, PreparedOrder};
use crate::pricing::DeliveryChoice;
use crate::rules::{self, GiftRules};
use crate::{
    ADDRESS_NOT_DELIVERABLE, CART_EMPTY, MIXED_ORDER_TYPE, NO_COMMON_PAYMENT_METHOD, NOT_SIGNED_IN,
    OUT_OF_STOCK, approval,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrepareGiftOrderInput {
    /// `address_known`: ship to `delivery_address`; `recipient_accepts`: the recipient gets a
    /// link at `recipient_contact`, gives the address and options, and payment waits for them.
    #[schemars(example = &"address_known")]
    pub recipient_mode: RecipientMode,
    /// Who receives it, as printed on the card and the parcel.
    #[schemars(length(min = 1, max = 100), example = &"Dad")]
    pub recipient_name: String,
    /// Email or phone the accept link goes to. Needed with `recipient_accepts`.
    #[schemars(length(max = 200), example = &"dad@example.com")]
    pub recipient_contact: Option<String>,
    /// Where it goes, for this order only (never saved). Needed with `address_known`.
    #[schemars(example = json!({"street_address": "48 Pine Rd", "address_locality": "Lakeview",
        "address_country": "US"}))]
    pub delivery_address: Option<GiftAddressInput>,
    /// The card's message, up to 300 characters.
    #[schemars(length(max = 300), example = &"For the next trail. Love, J.")]
    pub card_message: Option<String>,
    /// Wrap the gift (`gift.wrap_fee`). Default false.
    #[schemars(example = true)]
    pub gift_wrap: Option<bool>,
    /// Put in a gift receipt without prices. Default true.
    #[schemars(example = true)]
    pub gift_receipt: Option<bool>,
    /// The total the sender saw: the cart's total, or the gift's total with the wrap. The
    /// server recalculates and compares.
    #[schemars(example = json!({"value": 76.20, "currency": "USD"}))]
    pub client_total: Money,
}

fn bad(message: &str, field: &str, reason: &str) -> ToolError {
    ToolError::invalid_input(message)
        .detail(json!({"field": field, "reason": reason}))
        .next(PREPARE_TOOL)
}

fn trimmed(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub(crate) struct PrepareGiftOrder;

impl Operation for PrepareGiftOrder {
    const META: OpMeta = OpMeta {
        address: "sales.gift_order.prepare",
        tool: "prepare_gift_order",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/sales/gift-orders/prepare",
        summary: "Prepare a gift from the caller's own cart: the recipient, the card, the wrap, a gift receipt without prices, and the address for this order only (or a link the recipient accepts). Preview only; the sender places it with the Place order button. Nothing about the recipient is remembered.",
        consumes: &[Slot::one("cart_id")],
        produces: &[Slot::one("preview_id")],
        checks: &[
            "gift.recipient_per_order",
            "gift.recipient_modes",
            "gift.wrap_fee",
            "checkout_validation.common_payment_method_exists",
            "checkout_validation.stock_available",
            "checkout_validation.product_sellable",
            "checkout_validation.no_domestic_international_mix",
            "checkout_validation.server_total_matches_client",
            "checkout_validation.price_floor_respected",
        ],
        errors: &[
            CART_EMPTY,
            OUT_OF_STOCK,
            ADDRESS_NOT_DELIVERABLE,
            NO_COMMON_PAYMENT_METHOD,
            MIXED_ORDER_TYPE,
            NOT_SIGNED_IN,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::ORDER,
        person_scope: "",
    };
    type Input = PrepareGiftOrderInput;
    type Output = Value;
}

impl WriteOp<Cx> for PrepareGiftOrder {
    type Plan = PreparedGift;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: PrepareGiftOrderInput,
    ) -> Result<Planned<PreparedGift>, ToolError> {
        let gift_rules: GiftRules = rules::gift_rules(&cx.policy)?;
        let mode = input.recipient_mode;
        if !gift_rules.recipient_modes.iter().any(|m| m == mode.word()) {
            return Err(ToolError::invalid_input(format!(
                "This store does not send gifts as `{}`.",
                mode.word()
            ))
            .detail(json!({"field": "recipient_mode", "reason": "enum",
                           "allowed": gift_rules.recipient_modes,
                           "policy_key": "gift.recipient_modes"}))
            .next(PREPARE_TOOL));
        }
        let recipient_name = trimmed(Some(&input.recipient_name))
            .ok_or_else(|| bad("Say who the gift is for.", "recipient_name", "missing"))?;
        let card_message = trimmed(input.card_message.as_deref());
        if card_message
            .as_deref()
            .is_some_and(|m| m.chars().count() > CARD_MESSAGE_CHARS)
        {
            return Err(bad(
                "The card message is longer than 300 characters.",
                "card_message",
                "range",
            ));
        }
        let recipient_contact = trimmed(input.recipient_contact.as_deref());
        let delivery = match mode {
            RecipientMode::AddressKnown => {
                let a = input.delivery_address.as_ref().ok_or_else(|| {
                    bad(
                        "`address_known` needs `delivery_address`.",
                        "delivery_address",
                        "missing",
                    )
                })?;
                DeliveryChoice::OneTime(a.address(&recipient_name))
            }
            RecipientMode::RecipientAccepts => {
                if recipient_contact.is_none() {
                    return Err(bad(
                        "`recipient_accepts` needs `recipient_contact` for the accept link.",
                        "recipient_contact",
                        "missing",
                    ));
                }
                DeliveryChoice::Later
            }
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (carts, view) = order::checkout_view(&mut tx, &delivery, None).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let wrap = input
            .gift_wrap
            .unwrap_or(false)
            .then_some(gift_rules.wrap_fee);
        let total = total_of(&view, wrap)?;
        let today = cx.clock.now().with_timezone(&cx.time.zone).date_naive();
        let mut gift = PreparedGift {
            order: PreparedOrder {
                order_id: OrderId::generate().to_string(),
                customer_id: view.customer_id.clone().unwrap_or_default(),
                lines: order::planned_lines(&view),
                delivery,
                payment_method: view
                    .payment
                    .as_ref()
                    .map(|p| p.payment_method.clone())
                    .unwrap_or_default(),
                total,
                failed_checks: Vec::new(),
            },
            mode,
            recipient_name,
            recipient_contact,
            card_message,
            gift_wrap: wrap,
            gift_receipt: input.gift_receipt.unwrap_or(true),
            receipt_hides_prices: gift_rules.receipt_hides_prices,
            exchange_until: exchange_date(today, gift_rules.exchange_until),
            accept_days: gift_rules.accept_days,
        };
        let required = rules::checkout_checks(&cx.policy);
        let checks = checks(
            &view,
            &required,
            &gift,
            input.client_total,
            order::floor_multiplier(&cx.policy),
            total,
        );
        gift.order.failed_checks = checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.rule.clone())
            .collect();
        Ok(Planned {
            changes: vec![gift.change(&view, total)],
            checks,
            basis: carts.basis(),
            approval: approval(cx, Self::META.tool, &json!({"amount": total})),
            plan: gift,
        })
    }

    /// Never run by the preview engine (`approval.preview_only`): the sender's Place order
    /// button places the gift through `POST /v1/sales/orders`.
    async fn commit(&self, _: &mut TxCx, _: &PreparedGift) -> Result<Committed, ToolError> {
        Err(preview_only())
    }

    async fn verify(
        &self,
        _: &Cx,
        _: &PreparedGift,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        Err(preview_only())
    }

    async fn undo(&self, _: &mut TxCx, _: &PreparedGift, _: &Committed) -> Result<(), ToolError> {
        Err(preview_only())
    }
}

fn preview_only() -> ToolError {
    ToolError::invalid_input(
        "`prepare_gift_order` only prepares; the sender places the gift with POST /v1/sales/orders.",
    )
    .detail(json!({"field": "preview_id", "reason": "preview_only"}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gift_order::accept_gift_order::AcceptGiftOrder;
    use acom_contract::schema::{closed_schema_for, violations};
    use acom_core::Currency;
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    #[test]
    fn input_schema_is_closed_and_flat() {
        let p = closed_schema_for::<PrepareGiftOrderInput>();
        assert!(violations(&p).is_empty(), "{p:#}");
        assert_eq!(
            p["required"],
            json!(["recipient_mode", "recipient_name", "client_total"])
        );
        assert_eq!(
            p["properties"]["recipient_mode"]["enum"],
            json!(["address_known", "recipient_accepts"])
        );
        assert_eq!(
            p["properties"]["delivery_address"]["required"],
            json!(["street_address", "address_locality", "address_country"])
        );
    }

    fn offline() -> Cx {
        crate::testdb::offline("cus_jamie")
    }

    fn prepare(v: Value) -> PrepareGiftOrderInput {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn prepare_refuses_a_gift_it_cannot_send_before_reading_anything() {
        let cx = offline();
        let total = json!({"value": 76.20, "currency": "USD"});
        let e = PrepareGiftOrder
            .preview(
                &cx,
                prepare(
                    json!({"recipient_mode": "address_known", "recipient_name": "Dad",
                                        "client_total": total}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.detail.unwrap()["field"].clone()),
            ("invalid_input", json!("delivery_address"))
        );
        let e = PrepareGiftOrder
            .preview(
                &cx,
                prepare(
                    json!({"recipient_mode": "recipient_accepts", "recipient_name": "Dad",
                                        "client_total": total}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["field"], "recipient_contact");
        let e = PrepareGiftOrder
            .preview(
                &cx,
                prepare(
                    json!({"recipient_mode": "address_known", "recipient_name": " ",
                                        "client_total": total}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["field"], "recipient_name");
        let e = PrepareGiftOrder
            .preview(
                &cx,
                prepare(
                    json!({"recipient_mode": "address_known", "recipient_name": "Dad",
                "card_message": "x".repeat(301), "client_total": total,
                "delivery_address": {"street_address": "48 Pine Rd", "address_locality": "Lakeview",
                                     "address_country": "US"}}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["reason"], "range");
        assert_eq!(e.next_tool.as_deref(), Some("prepare_gift_order"));
        assert!(
            serde_json::from_value::<PrepareGiftOrderInput>(json!({"recipient_mode": "by_drone",
                "recipient_name": "Dad", "client_total": total}))
            .is_err(),
            "modes are words from the policy's list"
        );
    }

    #[tokio::test]
    async fn a_prepared_gift_is_only_placed_by_the_shoppers_button() {
        let cx = offline();
        let plan = PreparedGift {
            order: PreparedOrder {
                order_id: "ord_01".into(),
                customer_id: "cus_jamie".into(),
                lines: vec![],
                delivery: DeliveryChoice::Later,
                payment_method: "credit_card".into(),
                total: usd("76.20"),
                failed_checks: vec![],
            },
            mode: RecipientMode::AddressKnown,
            recipient_name: "Dad".into(),
            recipient_contact: None,
            card_message: None,
            gift_wrap: None,
            gift_receipt: true,
            receipt_hides_prices: true,
            exchange_until: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            accept_days: 3,
        };
        let done = Committed {
            id: "ord_01".into(),
            produced: serde_json::Map::new(),
        };
        let e = PrepareGiftOrder
            .verify(&cx, &plan, &done)
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["reason"], "preview_only");
        assert_eq!(PrepareGiftOrder::META.kind, Kind::Write);
        assert_eq!(AcceptGiftOrder::META.kind, Kind::Person);
        assert_eq!(AcceptGiftOrder::META.person_scope, "human:order");
    }
}
