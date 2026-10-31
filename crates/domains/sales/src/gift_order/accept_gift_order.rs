//! The recipient's person-only `POST /v1/sales/gift-orders/{gift_order_id}/accept`
//! (`accept_gift_order`): accept a gift through its link and give the address.

use acom_catalog::service::for_fulfillment as stock;
use acom_catalog::service::for_sales::{self as catalog, VariantMatch};
use acom_changeset::LogEntry;
use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot,
    ToolError,
};
use acom_core::enums::ActionStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{GiftAddressInput, no_such_gift, same};
use crate::delivery;
use crate::repo;
use crate::rules;
use crate::{ADDRESS_NOT_DELIVERABLE, GIFT_EXPIRED, OUT_OF_STOCK};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AcceptGiftOrderInput {
    /// The gift, from the accept link.
    #[schemars(regex(pattern = r"^gift_[a-z0-9_]+$"), example = &"gift_01jc9x2m")]
    pub gift_order_id: String,
    /// Where the gift goes.
    #[schemars(example = json!({"street_address": "48 Pine Rd", "address_locality": "Lakeview",
        "address_region": "NY", "postal_code": "12801", "address_country": "US"}))]
    pub delivery_address: GiftAddressInput,
    /// Another size of the same product, at the same price. The sender's choice when left out.
    #[schemars(length(max = 20), example = &"XL")]
    pub size: Option<String>,
    /// Another color of the same product, at the same price. The sender's choice when left out.
    #[schemars(length(max = 40), example = &"Navy")]
    pub color: Option<String>,
}

/// Person-only: the recipient accepts a gift through its link.
pub(crate) struct AcceptGiftOrder;

impl Operation for AcceptGiftOrder {
    const META: OpMeta = OpMeta {
        address: "sales.gift_order.accept",
        tool: "accept_gift_order",
        group: Group::Transact,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/sales/gift-orders/{gift_order_id}/accept",
        summary: "The recipient accepts a gift through its link and gives the address, and may pick another size or color at the same price; the held payment then goes through.",
        consumes: &[Slot::one("gift_order_id")],
        produces: &[Slot::one("order_id")],
        checks: &[
            "gift.recipient_can_change",
            "gift.auto_cancel_if_no_address_days",
        ],
        errors: &[
            GIFT_EXPIRED,
            OUT_OF_STOCK,
            ADDRESS_NOT_DELIVERABLE,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::UPDATE,
        person_scope: "human:order",
    };
    type Input = AcceptGiftOrderInput;
    type Output = Value;
}

const ACCEPT_ENDPOINT: &str = "POST /v1/sales/gift-orders/{gift_order_id}/accept";

impl PersonOp<Cx> for AcceptGiftOrder {
    async fn run(&self, tx: &mut TxCx, input: AcceptGiftOrderInput) -> Result<Value, ToolError> {
        let gift_rules = rules::gift_rules(&tx.policy)?;
        let g = repo::gift_order_for_update(tx, &input.gift_order_id)
            .await?
            .ok_or_else(|| no_such_gift(&input.gift_order_id))?;
        if g.action_status == ActionStatus::Completed.as_str() {
            return Ok(json!({"ok": true, "id": g.order_id, "undo": null}));
        }
        let now = tx.clock.now();
        if g.action_status == ActionStatus::Failed.as_str() || now > g.expires {
            return Err(ToolError::new(
                GIFT_EXPIRED,
                "This gift can no longer be accepted; the time to accept it has passed.",
            )
            .detail(json!({"expires": tx.time.format(g.expires),
                           "policy_key": "gift.auto_cancel_if_no_address_days"})));
        }
        let may = |what: &str| gift_rules.recipient_can_change.iter().any(|c| c == what);
        let wants_options = input.size.is_some() || input.color.is_some();
        if wants_options && !may("options") {
            return Err(ToolError::invalid_input(
                "This store does not let a recipient change the size or color.",
            )
            .detail(json!({"field": "size", "reason": "not_allowed",
                           "policy_key": "gift.recipient_can_change"})));
        }
        let address = input.delivery_address.address(&g.recipient_name);
        let ships_to = rules::ships_to(&tx.policy);
        if !delivery::deliverable(&address.address_country, &ships_to) {
            return Err(ToolError::new(
                ADDRESS_NOT_DELIVERABLE,
                format!("The store does not ship to {}.", address.address_country),
            )
            .detail(
                json!({"country": address.address_country, "ships_to": ships_to,
                           "policy_key": "shipping.ships_to"}),
            ));
        }

        let items = repo::items(tx, std::slice::from_ref(&g.order_id)).await?;
        let mut chosen: Option<(String, Option<String>, Option<String>)> = None;
        if wants_options {
            let [item] = items.as_slice() else {
                return Err(ToolError::invalid_input(
                    "This gift has several items; it can only be accepted as chosen.",
                )
                .detail(json!({"field": "size", "reason": "several_items"})));
            };
            let current = catalog::variants(tx, std::slice::from_ref(&item.product_id))
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| ToolError::new(ErrorCode::INTERNAL, "gift item has no variant"))?;
            let size = input
                .size
                .clone()
                .or_else(|| current.size.clone())
                .unwrap_or_default();
            let color = input
                .color
                .clone()
                .or_else(|| current.color.clone())
                .unwrap_or_default();
            let next = match catalog::find_variant(tx, &current.product_id, &size, &color).await? {
                VariantMatch::Found(v) => *v,
                VariantMatch::NoSuchOption { sizes, colors } => {
                    return Err(ToolError::invalid_input(format!(
                        "{} does not come in {size} {color}.",
                        current.product_name
                    ))
                    .detail(json!({"field": "size", "reason": "enum",
                                   "allowed": {"sizes": sizes, "colors": colors}})));
                }
                VariantMatch::NoProduct => {
                    return Err(ToolError::new(
                        ErrorCode::INTERNAL,
                        "the gift's product is gone",
                    ));
                }
            };
            if next.variant_id != current.variant_id {
                // The recipient can change the option, never the price (`gift.recipient_can_change`).
                if !same(next.list_price, current.list_price) {
                    return Err(ToolError::invalid_input(format!(
                        "{size} {color} costs {} instead of {}; a gift's price can't change.",
                        next.list_price, current.list_price
                    ))
                    .detail(json!({"field": "size", "reason": "price_differs",
                                   "policy_key": "gift.recipient_can_change"})));
                }
                let quantity = item.order_quantity;
                if !catalog::take_stock(tx, &next.variant_id, i64::from(quantity)).await? {
                    return Err(ToolError::new(
                        OUT_OF_STOCK,
                        format!("{size} {color} is sold out; choose another option."),
                    )
                    .detail(json!({"variant_id": next.variant_id})));
                }
                stock::move_stock(tx, &current.variant_id, quantity).await?;
                repo::set_item_variant(tx, &item.id, &next.variant_id).await?;
            }
            chosen = Some((next.variant_id, next.size, next.color));
        }

        // The held payment goes through now; a transfer still waits for the money.
        let payment = repo::payment(tx, &g.order_id).await?;
        let card = payment
            .as_ref()
            .is_some_and(|p| p.payment_method == "credit_card");
        let status = if card { "processing" } else { "payment_due" };
        if let Some(p) = payment.as_ref().filter(|_| card) {
            repo::complete_payment(tx, &p.id, now).await?;
        }
        repo::set_order_and_items_status(tx, &g.order_id, status).await?;
        repo::finish_gift_order(tx, &g.id, ActionStatus::Completed.as_str(), Some(now)).await?;
        let calendar = rules::ship_calendar(&tx.policy)?;
        let arrives = delivery::arrival(now.with_timezone(&tx.time.zone).naive_local(), &calendar);
        let record = json!({
            "gift_order_id": g.id,
            "delivery": {
                "address_id": null,
                "delivery_address": address,
                "address_label": delivery::full_label(address.name.as_deref(), &address),
                "arrives": arrives.format("%Y-%m-%d").to_string(),
                "address_saved": false,
            },
            "options": chosen.as_ref().map(|(v, s, c)| json!({"variant_id": v, "size": s, "color": c})),
        });
        repo::append_event(
            tx,
            &g.order_id,
            "AcceptAction",
            ACCEPT_ENDPOINT,
            Some("payment_due"),
            Some(status),
            &record,
        )
        .await?;
        acom_changeset::log_action(
            tx,
            LogEntry {
                object: Some(("order", &g.order_id)),
                result: Some(json!({"gift_order_id": g.id, "order_status": status})),
                ..LogEntry::new("AcceptAction", ActionStatus::Completed, Self::META.tool)
            },
        )
        .await?;
        Ok(json!({"ok": true, "id": g.order_id, "undo": null}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gift_order::gift_view;
    use acom_contract::schema::{closed_schema_for, violations};
    use chrono::Duration;
    use rust_decimal::Decimal;
    use std::sync::Arc;

    #[test]
    fn input_schema_is_closed() {
        let a = closed_schema_for::<AcceptGiftOrderInput>();
        assert!(violations(&a).is_empty(), "{a:#}");
        assert_eq!(a["required"], json!(["gift_order_id", "delivery_address"]));
    }

    type CategoryRead<'a> = std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        Vec<acom_promotion::service::for_merchandising::CategoryProducts>,
                        ToolError,
                    >,
                > + Send
                + 'a,
        >,
    >;

    /// Stands in for merchandising's category placement (sales cannot depend on merchandising):
    /// the sale below names its products, so categories only need to exist.
    fn no_category_products<'a>(_: &'a mut TxCx, ids: &'a [String]) -> CategoryRead<'a> {
        Box::pin(async move {
            Ok(ids
                .iter()
                .map(
                    |id| acom_promotion::service::for_merchandising::CategoryProducts {
                        category_id: id.clone(),
                        name: id.clone(),
                        product_ids: Vec::new(),
                    },
                )
                .collect())
        })
    }

    /// What the F4 integration run saw, gathered before cleaning up so a failed step still
    /// leaves the store as it was.
    #[derive(Debug, Default)]
    struct F4Seen {
        cart_total: Value,
        gift: Value,
        failed_checks: Vec<String>,
        placed: Value,
        is_gift: Option<bool>,
        wrap: Option<Decimal>,
        accepts: Value,
        accept_url: Value,
        placed_again: bool,
        late: Option<String>,
        accepted_again: bool,
        accepted_size: Option<String>,
        view: Value,
        view_after: Value,
    }

    /// Integration: scene F4 on the demo store (`DATABASE_URL`). The VIP weekend price needs
    /// change set #1042 running, so the test runs B1's sale prices and coupon through the
    /// change-set engine first (as `b1_vip_weekend_sale_end_to_end` does, without its banner and
    /// for the products by name),
    /// then, as Jamie on Sat Nov 7 10:00, puts the windbreaker in L in her cart and prepares
    /// the gift for Dad: $71.20 + $5.00 wrap = $76.20, arriving Tue Nov 10, address not saved.
    /// The gift is placed inside a transaction that is rolled back, and a `recipient_accepts`
    /// gift is placed the same way. Afterwards the cart line is removed and the change set
    /// undone, whatever happened in between.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn f4_jamie_prepares_a_gift_for_dad_at_the_vip_price() {
        use acom_changeset::NewChangeSet;
        use acom_contract::{Registry, RegistryBuilder};

        use crate::order::place_order::{PlaceOrder, PlaceOrderInput};

        acom_promotion::service::for_merchandising::provide_category_products(no_category_products);
        let registry: Registry<Cx> = {
            let mut b = RegistryBuilder::default();
            acom_promotion::register(&mut b);
            crate::register(&mut b);
            b.build().unwrap()
        };
        let sj = crate::testdb::cx_at("adm_sj", "2026-11-06T11:00").await;
        let jamie = crate::testdb::cx("cus_jamie").await;

        let mut change_set: Option<String> = None;
        let mut cart_line: Option<String> = None;
        let mut seen = F4Seen::default();
        let run: Result<(), String> = async {
            let created = acom_changeset::change_set::create(
                &sj,
                NewChangeSet {
                    title: "VIP weekend sale".into(),
                    request_text: vec!["Run a 20% weekend sale on summer outerwear for VIP members.".into()],
                    interpretation: vec![],
                    conversation_id: None,
                    task_id: None,
                },
            )
            .await
            .map_err(|e| e.message)?;
            let cs = created["id"].as_str().unwrap_or_default().to_owned();
            change_set = Some(cs.clone());
            // B1 prices 40 products through the category, which needs merchandising's category
            // placement; the windbreaker's VIP price is the same when it is named directly.
            let sale = json!({"name": "VIP weekend", "valid_for_member_tier": "vip",
                "valid_from": "2026-11-07T00:00", "valid_through": "2026-11-08T23:59",
                "scope_mode": "products", "product_ids": ["prd_windbreaker", "prd_field_jacket"],
                "percent_off": 20, "exclude_low_stock": true});
            acom_changeset::preview(&registry, &sj, "set_sale_prices", sale, Some(&cs))
                .await
                .map_err(|e| e.message)?;
            acom_changeset::preview(
                &registry,
                &sj,
                "create_coupon",
                json!({"code": "VIPWEEKEND20", "promotion_id": "$item[1].promotion_id",
                       "valid_for_member_tier": "vip", "application": "automatic_at_checkout",
                       "stacking": "no_tier_coupons"}),
                Some(&cs),
            )
            .await
            .map_err(|e| e.message)?;
            let head = acom_changeset::read::get(&sj, &cs, &[])
                .await
                .map_err(|e| e.message)?;
            let version = head["version"].as_i64().unwrap_or(1) as i32;
            acom_changeset::change_set::submit(&sj, &cs, version)
                .await
                .map_err(|e| e.message)?;
            let done = acom_changeset::change_set::approve(&registry, &sj, &cs, version)
                .await
                .map_err(|e| e.message)?;
            if done["state"] != "live" {
                return Err(format!("the sale did not go live: {done}"));
            }

            let added = acom_changeset::preview(
                &registry,
                &jamie,
                "add_to_cart",
                json!({"mode": "variant", "variant_id": "var_windbreaker_l_slate", "order_quantity": 1}),
                None,
            )
            .await
            .map_err(|e| e.message)?;
            let line = acom_changeset::commit(&registry, &jamie, "add_to_cart", &added.preview_id)
                .await
                .map_err(|e| e.message)?;
            cart_line = line.produced["cart_line_id"].as_str().map(str::to_owned);
            let cart = registry
                .get("get_cart")
                .ok_or("no get_cart")?
                .read(&jamie, json!({}))
                .await
                .map_err(|e| e.message)?;
            seen.cart_total = cart["total"].clone();

            let p = acom_changeset::preview(
                &registry,
                &jamie,
                "prepare_gift_order",
                json!({"recipient_mode": "address_known", "recipient_name": "Dad",
                       "delivery_address": {"street_address": "48 Pine Rd", "address_locality": "Lakeview",
                                            "address_country": "US"},
                       "card_message": "For the next trail. Love, J.", "gift_wrap": true,
                       "gift_receipt": true, "client_total": {"value": 76.20, "currency": "USD"}}),
                None,
            )
            .await
            .map_err(|e| e.message)?;
            seen.gift = p.changes.first().map(|c| c.to.clone()).unwrap_or_default();
            seen.failed_checks = p
                .checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| format!("{}: {:?}", c.rule, c.detail))
                .collect();
            {
                let mut tx = jamie.begin().await.map_err(|e| e.to_string())?;
                seen.placed = PlaceOrder
                    .run(&mut tx, PlaceOrderInput { preview_id: p.preview_id.clone() })
                    .await
                    .map_err(|e| e.message)?;
                let id = seen.placed["id"].as_str().unwrap_or_default().to_owned();
                // The same button pressed twice places one order.
                let again = PlaceOrder
                    .run(&mut tx, PlaceOrderInput { preview_id: p.preview_id.clone() })
                    .await
                    .map_err(|e| e.message)?;
                seen.placed_again = again["id"].as_str() == Some(id.as_str());
                let row: (bool, Option<Decimal>) = sqlx::query_as(
                    "SELECT is_gift, gift_wrap_value FROM sales.sales_order
                     WHERE tenant_id = $1 AND id = $2",
                )
                .bind("store_northwind")
                .bind(&id)
                .fetch_one(&mut *tx.tx)
                .await
                .map_err(|e| e.to_string())?;
                seen.is_gift = Some(row.0);
                seen.wrap = row.1;
            } // rolled back

            // The same gift through a link Dad accepts: payment waits for him.
            let later = acom_changeset::preview(
                &registry,
                &jamie,
                "prepare_gift_order",
                json!({"recipient_mode": "recipient_accepts", "recipient_name": "Dad",
                       "recipient_contact": "dad@example.com", "gift_wrap": true,
                       "client_total": cart["total"]}),
                None,
            )
            .await
            .map_err(|e| e.message)?;
            let mut tx = jamie.begin().await.map_err(|e| e.to_string())?;
            let placed = PlaceOrder
                .run(&mut tx, PlaceOrderInput { preview_id: later.preview_id.clone() })
                .await
                .map_err(|e| e.message)?;
            seen.accept_url = placed["accept_url"].clone();
            let gift_order_id = placed["gift_order_id"].as_str().unwrap_or_default().to_owned();
            let accept = || -> Result<AcceptGiftOrderInput, String> {
                serde_json::from_value(json!({"gift_order_id": gift_order_id,
                    "delivery_address": {"street_address": "48 Pine Rd", "address_locality": "Lakeview",
                                         "address_region": "NY", "postal_code": "12801", "address_country": "US"},
                    "size": "XL"}))
                .map_err(|e| e.to_string())
            };
            seen.view = gift_view(&mut tx, &gift_order_id)
                .await
                .map_err(|e| e.message)?;
            // Too late: four days after the order, past `gift.auto_cancel_if_no_address_days`.
            let now = tx.clock.clone();
            tx.clock = Arc::new(acom_core::FixedClock::at(now.now() + Duration::days(4)));
            seen.late = AcceptGiftOrder
                .run(&mut tx, accept()?)
                .await
                .err()
                .map(|e| e.code.to_string());
            tx.clock = now;
            seen.accepts = AcceptGiftOrder
                .run(&mut tx, accept()?)
                .await
                .map_err(|e| e.message)?;
            let again = AcceptGiftOrder
                .run(&mut tx, accept()?)
                .await
                .map_err(|e| e.message)?;
            seen.accepted_again = again == seen.accepts;
            seen.view_after = gift_view(&mut tx, &gift_order_id)
                .await
                .map_err(|e| e.message)?;
            let size: String = sqlx::query_scalar(
                "SELECT v.size FROM sales.order_item i JOIN catalog.product v
                   ON v.tenant_id = i.tenant_id AND v.id = i.product_id
                 WHERE i.tenant_id = $1 AND i.order_id = $2",
            )
            .bind("store_northwind")
            .bind(placed["id"].as_str().unwrap_or_default())
            .fetch_one(&mut *tx.tx)
            .await
            .map_err(|e| e.to_string())?;
            seen.accepted_size = Some(size);
            let status: String = sqlx::query_scalar(
                "SELECT order_status FROM sales.sales_order WHERE tenant_id = $1 AND id = $2",
            )
            .bind("store_northwind")
            .bind(placed["id"].as_str().unwrap_or_default())
            .fetch_one(&mut *tx.tx)
            .await
            .map_err(|e| e.to_string())?;
            seen.accepts["order_status_after"] = json!(status);
            seen.accepts["placed_status"] = placed["order_status"].clone();
            drop(tx); // rolled back
            Ok(())
        }
        .await;

        // Leave the store as it was: the cart line out, the sale undone.
        if let Some(id) = &cart_line
            && let Ok(removed) = acom_changeset::preview(
                &registry,
                &jamie,
                "remove_cart_item",
                json!({"cart_line_id": id}),
                None,
            )
            .await
        {
            let _ =
                acom_changeset::commit(&registry, &jamie, "remove_cart_item", &removed.preview_id)
                    .await;
        }
        if let Some(cs) = &change_set {
            let _ = acom_changeset::change_set::undo(&registry, &sj, cs).await;
        }

        run.unwrap_or_else(|e| panic!("F4 run failed: {e}"));
        assert_eq!(
            seen.cart_total["value"].to_string(),
            "71.20",
            "the VIP weekend price"
        );
        assert!(seen.failed_checks.is_empty(), "{:?}", seen.failed_checks);
        let g = &seen.gift;
        assert_eq!(g["order_type"], "gift");
        assert_eq!(g["subtotal"]["value"].to_string(), "89.00");
        assert_eq!(g["discounts"][0]["amount"]["value"].to_string(), "17.80");
        assert_eq!(g["gift_wrap"]["value"].to_string(), "5.00");
        assert_eq!(g["shipping"]["value"].to_string(), "0.00");
        assert_eq!(g["total"]["value"].to_string(), "76.20");
        assert_eq!(g["lines"][0]["size"], "L");
        assert_eq!(g["delivery"]["address_label"], "Dad · 48 Pine Rd, Lakeview");
        assert_eq!(g["delivery"]["arrives"], "2026-11-10");
        assert_eq!(g["delivery"]["address_saved"], false);
        assert_eq!(
            g["gift_options"]["card_message"],
            "For the next trail. Love, J."
        );
        assert_eq!(g["gift_options"]["receipt_hides_prices"], true);
        assert_eq!(g["gift_options"]["exchange_until"], "2026-12-31");
        assert_eq!(seen.placed["order_status"], "processing");
        assert_eq!(seen.is_gift, Some(true));
        assert_eq!(seen.wrap.map(|w| w.to_string()).as_deref(), Some("5.00"));
        assert!(
            seen.accept_url
                .as_str()
                .is_some_and(|u| u.starts_with("/gifts/gift_")),
            "{:?}",
            seen.accept_url
        );
        assert!(seen.placed_again, "one order for one preview");
        assert_eq!(
            seen.accepts["placed_status"], "payment_due",
            "held until Dad accepts"
        );
        assert_eq!(seen.late.as_deref(), Some("gift_expired"));
        assert_eq!(seen.accepts["ok"], true);
        assert!(
            seen.accepted_again,
            "accepting twice gives the first result"
        );
        assert_eq!(
            seen.accepted_size.as_deref(),
            Some("XL"),
            "Dad picked XL at the same price"
        );
        assert_eq!(seen.accepts["order_status_after"], "processing");

        // What Dad saw through the link before and after accepting: no prices anywhere.
        let v = &seen.view;
        assert_eq!(v["action_status"], "potential");
        assert_eq!(v["recipient_name"], "Dad");
        assert_eq!(v["from"], "Jamie");
        assert_eq!(v["gift_wrap"], true);
        assert_eq!(v["items"][0]["size"], "L");
        let options = v["items"][0]["options"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            options
                .iter()
                .any(|o| o["size"] == "XL" && o["available"] == true),
            "{options:?}"
        );
        assert!(!v.to_string().contains("\"value\""), "no prices: {v}");
        assert_eq!(seen.view_after["action_status"], "completed");
        assert_eq!(seen.view_after["items"][0]["size"], "XL");
    }
}
