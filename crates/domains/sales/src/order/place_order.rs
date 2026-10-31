//! The person-only `POST /v1/sales/orders` (`place_order`): the shopper's Place order button
//! orders what a `prepare_order` or `prepare_gift_order` preview describes.

use acom_catalog::service::for_sales as catalog;
use acom_changeset::LogEntry;
use acom_contract::{
    ActionType, Audience, Committed, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp,
    Slot, ToolError,
};
use acom_core::enums::ActionStatus;
use acom_core::ids::{OrderItemId, PaymentId};
use acom_promotion::service::for_sales as promotion;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::prepare_order::PrepareOrder;
use super::{PreparedOrder, checkout_view, create_order_change, floor_multiplier, planned_lines};
use crate::checkout;
use crate::delivery;
use crate::gift_order as gift;
use crate::repo;
use crate::rules;
use crate::{
    ADDRESS_NOT_FOUND, CART_EMPTY, NO_COMMON_PAYMENT_METHOD, NOT_SIGNED_IN, OUT_OF_STOCK,
    PAYMENT_DECLINED,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaceOrderInput {
    /// The `prepare_order` or `prepare_gift_order` preview the shopper confirmed on the card.
    #[schemars(regex(pattern = r"^pv_[a-z0-9_]+$"), example = &"pv_7731")]
    pub preview_id: String,
}

/// Person-only: the shopper's Place order button.
pub(crate) struct PlaceOrder;

impl Operation for PlaceOrder {
    const META: OpMeta = OpMeta {
        address: "sales.order.place",
        tool: "place_order",
        group: Group::Transact,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/sales/orders",
        summary: "Place the order a prepare_order or prepare_gift_order preview describes: re-price, take stock, record coupons, start payment (a gift the recipient accepts waits for them), empty the cart.",
        consumes: &[Slot::one("preview_id")],
        produces: &[Slot::one("order_id")],
        checks: &[
            "inventory.reserve_on",
            "inventory.unpaid_ttl_minutes",
            "checkout_validation",
        ],
        errors: &[
            ErrorCode::PREVIEW_EXPIRED,
            ErrorCode::ESTIMATE_CHANGED,
            OUT_OF_STOCK,
            PAYMENT_DECLINED,
            CART_EMPTY,
            NO_COMMON_PAYMENT_METHOD,
            ADDRESS_NOT_FOUND,
            NOT_SIGNED_IN,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::ORDER,
        person_scope: "human:order",
    };
    type Input = PlaceOrderInput;
    type Output = Value;
}

fn placed(done: &Committed) -> Value {
    let mut out = json!({"ok": true, "id": done.id, "order_number": done.produced.get("order_number"),
           "order_status": done.produced.get("order_status"), "undo": null});
    // A gift the recipient accepts: where they accept it.
    for key in ["gift_order_id", "accept_url"] {
        if let Some(v) = done.produced.get(key) {
            out[key] = v.clone();
        }
    }
    out
}

/// The error for a checkout check that fails at the moment of ordering.
fn failed_check(rule: &str, detail: Option<&str>) -> ToolError {
    let detail_text = detail.unwrap_or("A checkout check failed.").to_owned();
    match rule.trim_start_matches("checkout_validation.") {
        "stock_available" | "product_sellable" => ToolError::new(OUT_OF_STOCK, detail_text),
        "common_payment_method_exists" => ToolError::new(NO_COMMON_PAYMENT_METHOD, detail_text),
        _ => ToolError::new(ErrorCode::POLICY_VIOLATION, detail_text),
    }
    .detail(json!({"rule": rule}))
}

/// The preview behind a Place order button: an order (`prepare_order`) or a gift
/// (`prepare_gift_order`); both are placed by this one endpoint.
async fn claim_order_preview(
    tx: &mut TxCx,
    preview_id: &str,
) -> Result<(acom_changeset::ClaimedPreview, Option<gift::PreparedGift>), ToolError> {
    match acom_changeset::claim_preview(tx, preview_id, PrepareOrder::META.tool).await {
        Ok(c) => Ok((c, None)),
        Err(e) if e.code == ErrorCode::INVALID_INPUT.code => {
            let c = acom_changeset::claim_preview(tx, preview_id, gift::PREPARE_TOOL)
                .await
                .map_err(|_| e)?;
            if c.already.is_some() {
                return Ok((c, None));
            }
            let g: gift::PreparedGift = serde_json::from_value(c.plan.clone())
                .map_err(|e| ToolError::new(ErrorCode::INTERNAL, format!("prepared gift: {e}")))?;
            Ok((c, Some(g)))
        }
        Err(e) => Err(e),
    }
}

impl PersonOp<Cx> for PlaceOrder {
    async fn run(&self, tx: &mut TxCx, input: PlaceOrderInput) -> Result<Value, ToolError> {
        let (claimed, gift) = claim_order_preview(tx, &input.preview_id).await?;
        if let Some(done) = &claimed.already {
            return Ok(placed(done));
        }
        let plan: PreparedOrder = match &gift {
            Some(g) => g.order.clone(),
            None => serde_json::from_value(claimed.plan.clone())
                .map_err(|e| ToolError::new(ErrorCode::INTERNAL, format!("prepared order: {e}")))?,
        };
        if tx.principal.customer_id() != Some(plan.customer_id.as_str()) {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                "This order was prepared for someone else.",
            )
            .detail(json!({"reason": "owner"})));
        }
        if let Some(rule) = plan.failed_checks.first() {
            return Err(failed_check(
                rule,
                Some("The checkout card showed a failed check; prepare the order again."),
            ));
        }

        // Re-price now: what is charged is what the shopper saw, or nothing.
        let (carts, view) = checkout_view(tx, &plan.delivery, Some(&plan.payment_method)).await?;
        let required = rules::checkout_checks(&tx.policy);
        let floor = floor_multiplier(&tx.policy);
        let (checks, total, change) = match &gift {
            Some(g) => {
                let total = gift::total_of(&view, g.gift_wrap)?;
                let checks = gift::checks(&view, &required, g, plan.total, floor, total);
                (checks, total, g.change(&view, total))
            }
            None => (
                checkout::checkout_checks(&view, &required, plan.total, floor),
                view.breakdown.total,
                create_order_change(&view),
            ),
        };
        if planned_lines(&view) != plan.lines || total != plan.total {
            return Err(ToolError::new(
                ErrorCode::ESTIMATE_CHANGED,
                format!(
                    "The order now comes to {total} instead of {}; prepare it again.",
                    plan.total
                ),
            )
            .detail(json!({"new_preview": {"changes": [change], "checks": checks}})));
        }
        if let Some(c) = checks.iter().find(|c| !c.passed) {
            return Err(failed_check(&c.rule, c.detail.as_deref()));
        }
        let payment = view.payment.clone().ok_or_else(|| {
            failed_check("checkout_validation.common_payment_method_exists", None)
        })?;
        let card = view.card_last4.clone();
        if payment.payment_method == "credit_card" && card.is_none() {
            return Err(
                ToolError::new(PAYMENT_DECLINED, "There is no saved card to charge.")
                    .detail(json!({"payment_method": "credit_card"})),
            );
        }

        let now = tx.clock.now();
        let b = &view.breakdown;
        // A saved card is charged at once; transfers wait for the money (payment_due), and so
        // does a gift until its recipient accepts it.
        let held = gift
            .as_ref()
            .is_some_and(gift::PreparedGift::waits_for_recipient);
        let (order_status, payment_status, paid_at) =
            if payment.payment_method == "credit_card" && !held {
                ("processing", "payment_complete", Some(now))
            } else {
                ("payment_due", "payment_due", None)
            };
        let order_id = plan.order_id.clone();
        let customer_id = plan.customer_id.clone();
        let order_number = repo::next_order_number(tx).await?;
        let coupon = b.coupons_recorded.first();
        let gift_options = gift.as_ref().map(gift::PreparedGift::options_json);
        repo::insert_order(
            tx,
            &repo::NewOrder {
                id: &order_id,
                order_number: &order_number,
                order_date: now,
                order_status,
                customer_id: &customer_id,
                sales_channel: "web",
                discount_code: coupon.map(|c| c.code.as_str()),
                coupon_id: coupon.map(|c| c.coupon_id.as_str()),
                subtotal: b.subtotal,
                discount: b.discount_total,
                shipping: b.shipping,
                total,
                preview_id: &claimed.id,
                is_gift: gift.is_some(),
                gift_options: gift_options.as_ref(),
                gift_wrap: gift.as_ref().and_then(|g| g.gift_wrap),
            },
        )
        .await?;
        for (n, l) in view.lines.iter().enumerate() {
            let r = b
                .lines
                .iter()
                .find(|r| r.cart_line_id == l.line_id)
                .ok_or_else(|| ToolError::new(ErrorCode::INTERNAL, "line without benefits"))?;
            let item_id = OrderItemId::generate().to_string();
            repo::insert_item(
                tx,
                &repo::NewItem {
                    id: &item_id,
                    order_id: &order_id,
                    position: i32::try_from(n + 1).unwrap_or(i32::MAX),
                    variant_id: &l.variant.variant_id,
                    name: &l.variant.product_name,
                    quantity: i32::try_from(l.quantity).unwrap_or(i32::MAX),
                    status: order_status,
                    price: promotion::money(
                        l.variant.list_price.value,
                        l.variant.list_price.currency,
                    ),
                    discount: promotion::money(
                        r.item_discount.value + r.order_discount.value,
                        b.currency,
                    ),
                    promotion_id: r.promotion_id.as_deref(),
                },
            )
            .await?;
            // `inventory.reserve_on: place_order`: stock is taken now.
            if !catalog::take_stock(tx, &l.variant.variant_id, i64::from(l.quantity)).await? {
                return Err(ToolError::new(
                    OUT_OF_STOCK,
                    format!("{} sold out while you were ordering.", l.title()),
                )
                .detail(json!({"variant_id": l.variant.variant_id})));
            }
        }
        let payment_id = PaymentId::generate().to_string();
        repo::insert_payment(
            tx,
            &repo::NewPayment {
                id: &payment_id,
                order_id: &order_id,
                method: &payment.payment_method,
                status: payment_status,
                amount: total,
                card_last4: if payment.payment_method == "credit_card" {
                    card.as_deref()
                } else {
                    None
                },
                paid_at,
            },
        )
        .await?;
        let member_coupons =
            promotion::redeem_coupons(tx, &customer_id, &order_id, &b.coupons_recorded).await?;
        let d = view.destination.as_ref();
        let mut record = json!({
            "preview_id": claimed.id,
            "delivery": d.map(|d| json!({
                "address_id": d.address_id,
                "delivery_address": d.address,
                "address_label": delivery::full_label(d.label.as_deref(), &d.address),
                "arrives": d.arrives.format("%Y-%m-%d").to_string(),
            })),
            "payment_id": payment_id,
            "discounts": b.discounts(),
            "coupons_recorded": b.coupons_recorded.iter().map(|c| json!({
                "coupon_id": c.coupon_id, "code": c.code, "promotion_id": c.promotion_id,
            })).collect::<Vec<_>>(),
            "member_coupon_ids": member_coupons,
        });
        let mut produced = Map::new();
        if let Some(g) = &gift {
            record["gift"] = g.record_json();
            if let Some(delivery) = record.get_mut("delivery").filter(|v| v.is_object()) {
                // The recipient's address is for this order only.
                delivery["address_saved"] = json!(false);
            }
            if let Some(accept) = g.place_acceptance(tx, &order_id).await? {
                record["gift"]["gift_order_id"] = json!(accept.gift_order_id);
                record["gift"]["expires"] = json!(tx.time.format(accept.expires));
                produced.insert("gift_order_id".into(), json!(accept.gift_order_id));
                produced.insert("accept_url".into(), json!(accept.accept_url));
            }
        }
        repo::append_event(
            tx,
            &order_id,
            ActionType::ORDER.0,
            "POST /v1/sales/orders",
            None,
            Some(order_status),
            &record,
        )
        .await?;

        // The ordered lines leave the cart (with guest lines folded into them).
        let mut ordered: Vec<String> = Vec::new();
        for l in &plan.lines {
            if let Some(m) = carts.line(&l.cart_line_id) {
                ordered.push(m.id.clone());
                ordered.extend(m.absorbs.clone());
            }
        }
        repo::delete_lines(tx, &ordered).await?;
        for c in [&carts.main, &carts.guest].into_iter().flatten() {
            repo::touch_cart(tx, &c.id).await?;
        }

        produced.insert("order_id".into(), json!(order_id));
        produced.insert("order_number".into(), json!(order_number));
        produced.insert("order_status".into(), json!(order_status));
        produced.insert("payment_id".into(), json!(payment_id));
        let done = Committed {
            id: order_id.clone(),
            produced,
        };
        acom_changeset::mark_committed(tx, &claimed.id, &claimed.plan, &done).await?;
        acom_changeset::log_action(
            tx,
            LogEntry {
                object: Some(("order", &order_id)),
                result: Some(json!({"order_number": order_number, "total": total,
                                    "order_status": order_status, "is_gift": gift.is_some()})),
                preview_id: Some(&claimed.id),
                ..LogEntry::new(
                    ActionType::ORDER.0,
                    ActionStatus::Completed,
                    Self::META.tool,
                )
            },
        )
        .await?;
        Ok(placed(&done))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cart::get_cart::{GetCart, GetCartInput};
    use crate::order::prepare_order::PrepareOrderInput;
    use crate::order::{DeliveryMode, PlannedLine};
    use crate::pricing::DeliveryChoice;
    use acom_contract::schema::{closed_schema_for, violations};
    use acom_contract::{ReadOp, WriteOp};
    use acom_core::Money;
    use rust_decimal::Decimal;

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<PlaceOrderInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
    }

    #[test]
    fn a_failed_check_at_ordering_time_names_its_rule() {
        let e = failed_check("checkout_validation.stock_available", Some("Sold out"));
        assert_eq!(e.code, "out_of_stock");
        assert_eq!(
            e.detail.unwrap()["rule"],
            "checkout_validation.stock_available"
        );
        assert_eq!(
            failed_check("checkout_validation.price_floor_respected", None).code,
            "policy_violation"
        );
    }

    #[test]
    fn prepared_orders_survive_the_preview_store() {
        let plan = PreparedOrder {
            order_id: "ord_01".into(),
            customer_id: "cus_jamie".into(),
            lines: vec![PlannedLine {
                cart_line_id: "cln_1".into(),
                variant_id: "var_windbreaker_m_slate".into(),
                quantity: 1,
            }],
            delivery: DeliveryChoice::Saved("adr_home".into()),
            payment_method: "credit_card".into(),
            total: promotion::money(Decimal::new(7120, 2), acom_core::Currency::USD),
            failed_checks: vec![],
        };
        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["total"]["value"].to_string(), "71.20");
        let back: PreparedOrder = serde_json::from_value(json).unwrap();
        assert_eq!(back, plan);
    }

    /// Integration: Jamie puts the windbreaker in her cart, prepares the order for exactly the
    /// cart's total, and places it; the order is rolled back and the cart line removed again
    /// (`DATABASE_URL`, demo seed). The $71.20 of scene F2, which needs change set #1042 run
    /// first, is checked by `services/commerce/tests/scenes.rs`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_prepares_and_places_her_order() {
        let cx = crate::testdb::cx("cus_jamie").await;
        let registry = crate::testdb::registry();
        let added = acom_changeset::preview(
            &registry,
            &cx,
            "add_to_cart",
            json!({"mode": "variant", "variant_id": "var_windbreaker_m_slate", "order_quantity": 1}),
            None,
        )
        .await
        .unwrap();
        let line = acom_changeset::commit(&registry, &cx, "add_to_cart", &added.preview_id)
            .await
            .unwrap();
        let cart = GetCart
            .read(&cx, GetCartInput { fields: None })
            .await
            .unwrap();
        let total: Money = serde_json::from_value(cart["total"].clone()).unwrap();
        let planned = PrepareOrder
            .preview(
                &cx,
                PrepareOrderInput {
                    delivery_mode: DeliveryMode::Saved,
                    delivery_address_id: Some("adr_home".into()),
                    delivery_address: None,
                    payment_method: None,
                    client_total: total,
                },
            )
            .await
            .unwrap();
        assert!(
            planned.checks.iter().all(|c| c.passed),
            "{:?}",
            planned.checks
        );
        assert_eq!(planned.changes[0].to["total"], cart["total"]);
        let p = acom_changeset::preview(
            &registry,
            &cx,
            "prepare_order",
            json!({"delivery_mode": "saved", "delivery_address_id": "adr_home",
                   "client_total": cart["total"]}),
            None,
        )
        .await
        .unwrap();
        let mut tx = cx.begin().await.unwrap();
        let out = PlaceOrder
            .run(
                &mut tx,
                PlaceOrderInput {
                    preview_id: p.preview_id.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(out["order_status"], "processing");
        drop(tx); // rolled back
        let cart_line_id = line.produced["cart_line_id"].as_str().unwrap().to_owned();
        let removed = acom_changeset::preview(
            &registry,
            &cx,
            "remove_cart_item",
            json!({"cart_line_id": cart_line_id}),
            None,
        )
        .await
        .unwrap();
        acom_changeset::commit(&registry, &cx, "remove_cart_item", &removed.preview_id)
            .await
            .unwrap();
    }
}
