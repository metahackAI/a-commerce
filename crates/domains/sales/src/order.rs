//! `prepare_order`, the person-only `POST /v1/sales/orders` that places it, `get_my_orders`,
//! `get_order`, `search_orders`.

use std::collections::HashMap;

use acom_catalog::service::for_sales as catalog;
use acom_changeset::LogEntry;
use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, ErrorCode, Group, Kind, Method, OpMeta,
    Operation, PersonOp, Planned, ReadOp, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::{ActionStatus, OrderStatus, PaymentMethod};
use acom_core::ids::{OrderId, OrderItemId, PaymentId};
use acom_promotion::service::for_sales as promotion;
use acom_store::{Cx, Page, TxCx};
use chrono::Duration;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::checkout;
use crate::delivery::{self, Address};
use crate::owner::{self, Viewer};
use crate::pricing::{self, CartView, Carts, DeliveryChoice};
use crate::repo::{self, OrderRow};
use crate::rules;
use crate::{
    ADDRESS_NOT_DELIVERABLE, ADDRESS_NOT_FOUND, CART_EMPTY, INVALID_FILTER, MIXED_ORDER_TYPE,
    NO_COMMON_PAYMENT_METHOD, NOT_SIGNED_IN, ORDER_NOT_FOUND, OUT_OF_STOCK, PAYMENT_DECLINED,
    address_not_found, approval, cart_empty, not_signed_in, parse_time,
};

/// How the shopper names where the order goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// A saved address (`delivery_address_id`).
    Saved,
    /// An address for this order only (`delivery_address`), not saved.
    OneTime,
}

/// A postal address given for one order.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PostalAddressInput {
    /// Who receives the parcel.
    #[schemars(length(max = 100), example = &"Jamie")]
    pub name: Option<String>,
    /// Street, number, apartment.
    #[schemars(length(min = 1, max = 200), example = &"12 Harbor St, Apt 3")]
    pub street_address: String,
    /// City.
    #[schemars(length(min = 1, max = 100), example = &"Brooklyn")]
    pub address_locality: String,
    /// State or region.
    #[schemars(length(max = 100), example = &"NY")]
    pub address_region: Option<String>,
    /// Postal code.
    #[schemars(length(min = 1, max = 20), example = &"11201")]
    pub postal_code: String,
    /// ISO 3166-1 country code.
    #[schemars(regex(pattern = r"^[A-Z]{2}$"), example = &"US")]
    pub address_country: String,
}

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

/// One line of a prepared order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PlannedLine {
    pub cart_line_id: String,
    pub variant_id: String,
    pub quantity: u32,
}

/// What `prepare_order` hands to the shopper's "Place order" button.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PreparedOrder {
    order_id: String,
    customer_id: String,
    lines: Vec<PlannedLine>,
    delivery: DeliveryChoice,
    payment_method: String,
    /// The total the checkout card showed.
    total: Money,
    /// Checks that failed on the card; such an order cannot be placed.
    failed_checks: Vec<String>,
}

fn planned_lines(v: &CartView) -> Vec<PlannedLine> {
    v.lines
        .iter()
        .map(|l| PlannedLine {
            cart_line_id: l.line_id.clone(),
            variant_id: l.variant.variant_id.clone(),
            quantity: l.quantity,
        })
        .collect()
}

fn create_order_change(v: &CartView) -> Change {
    Change {
        op: ChangeOp::Create,
        entity: "order".into(),
        id: String::new(),
        from: Value::Null,
        to: pricing::order_json(v),
        note: None,
    }
}

fn floor_multiplier(tx_policy: &acom_policy::PolicySet) -> Option<Decimal> {
    promotion::rules(tx_policy).ok()?.price_floor_multiplier
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
    type Output = Value;
}

fn choice(input: &PrepareOrderInput) -> Result<DeliveryChoice, ToolError> {
    match input.delivery_mode {
        DeliveryMode::Saved => input
            .delivery_address_id
            .clone()
            .map(DeliveryChoice::Saved)
            .ok_or_else(|| {
                ToolError::invalid_input("`saved` needs `delivery_address_id`.")
                    .detail(json!({"field": "delivery_address_id", "reason": "missing"}))
            }),
        DeliveryMode::OneTime => {
            let a = input.delivery_address.as_ref().ok_or_else(|| {
                ToolError::invalid_input("`one_time` needs `delivery_address`.")
                    .detail(json!({"field": "delivery_address", "reason": "missing"}))
            })?;
            Ok(DeliveryChoice::OneTime(Address {
                name: a.name.clone(),
                street_address: a.street_address.trim().to_owned(),
                address_locality: a.address_locality.trim().to_owned(),
                address_region: a.address_region.clone(),
                postal_code: a.postal_code.trim().to_owned(),
                address_country: a.address_country.clone(),
            }))
        }
    }
}

/// Prices the caller's cart for checkout and refuses what can never be ordered.
async fn checkout_view(
    tx: &mut TxCx,
    delivery: &DeliveryChoice,
    payment_method: Option<&str>,
) -> Result<(Carts, CartView), ToolError> {
    let owners = owner::owners(&tx.principal)?;
    if owners.owner.customer_id.is_none() {
        return Err(not_signed_in());
    }
    let limits = rules::cart_limits(&tx.policy)?;
    let carts = pricing::load_carts(tx, &owners, limits).await?;
    let view = pricing::view(tx, &carts, delivery, payment_method)
        .await?
        .map_err(|id| address_not_found(&id))?;
    if view.lines.is_empty() {
        return Err(cart_empty());
    }
    let kinds: Vec<bool> = view
        .lines
        .iter()
        .map(|l| l.variant.sale_type == "subscription")
        .collect();
    if kinds.iter().any(|k| *k != kinds[0]) {
        return Err(ToolError::new(
            MIXED_ORDER_TYPE,
            "Subscriptions and one-time purchases are ordered separately.",
        )
        .detail(json!({"policy_key": "cart.separate_by"}))
        .next("remove_cart_item"));
    }
    let Some(d) = &view.destination else {
        return Err(ToolError::new(
            ADDRESS_NOT_FOUND,
            "There is no delivery address yet; save one or give one for this order.",
        )
        .detail(json!({"entity": "postal_address"}))
        .next("get_my_profile"));
    };
    let ships_to = rules::ships_to(&tx.policy);
    if !delivery::deliverable(&d.address.address_country, &ships_to) {
        return Err(ToolError::new(
            ADDRESS_NOT_DELIVERABLE,
            format!("The store does not ship to {}.", d.address.address_country),
        )
        .detail(
            json!({"country": d.address.address_country, "ships_to": ships_to,
                       "policy_key": "shipping.ships_to"}),
        )
        .next("get_my_profile"));
    }
    Ok((carts, view))
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaceOrderInput {
    /// The `prepare_order` preview the shopper confirmed on the checkout card.
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
        summary: "Place the order a prepare_order preview describes: re-price, take stock, record coupons, start payment, empty the cart.",
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
    json!({"ok": true, "id": done.id, "order_number": done.produced.get("order_number"),
           "order_status": done.produced.get("order_status"), "undo": null})
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

impl PersonOp<Cx> for PlaceOrder {
    async fn run(&self, tx: &mut TxCx, input: PlaceOrderInput) -> Result<Value, ToolError> {
        let claimed =
            acom_changeset::claim_preview(tx, &input.preview_id, PrepareOrder::META.tool).await?;
        if let Some(done) = &claimed.already {
            return Ok(placed(done));
        }
        let plan: PreparedOrder = serde_json::from_value(claimed.plan.clone())
            .map_err(|e| ToolError::new(ErrorCode::INTERNAL, format!("prepared order: {e}")))?;
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
        let checks =
            checkout::checkout_checks(&view, &required, plan.total, floor_multiplier(&tx.policy));
        if planned_lines(&view) != plan.lines || view.breakdown.total != plan.total {
            return Err(ToolError::new(
                ErrorCode::ESTIMATE_CHANGED,
                format!(
                    "The order now comes to {} instead of {}; prepare it again.",
                    view.breakdown.total, plan.total
                ),
            )
            .detail(
                json!({"new_preview": {"changes": [create_order_change(&view)],
                                           "checks": checks}}),
            ));
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
        // A saved card is charged at once; transfers wait for the money (payment_due).
        let (order_status, payment_status, paid_at) = if payment.payment_method == "credit_card" {
            ("processing", "payment_complete", Some(now))
        } else {
            ("payment_due", "payment_due", None)
        };
        let order_id = plan.order_id.clone();
        let customer_id = plan.customer_id.clone();
        let order_number = repo::next_order_number(tx).await?;
        let coupon = b.coupons_recorded.first();
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
                total: b.total,
                preview_id: &claimed.id,
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
                amount: b.total,
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
        let record = json!({
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

        let mut produced = Map::new();
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
                result: Some(json!({"order_number": order_number, "total": b.total,
                                    "order_status": order_status})),
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

// ---------------------------------------------------------------- reads

/// Facts the order reads add to each item.
struct ItemFacts {
    product_id: Option<String>,
    size: Option<String>,
    color: Option<String>,
}

async fn item_facts(
    tx: &mut TxCx,
    items: &[repo::ItemRow],
) -> Result<HashMap<String, ItemFacts>, ToolError> {
    let mut ids: Vec<String> = items.iter().map(|i| i.product_id.clone()).collect();
    ids.sort_unstable();
    ids.dedup();
    Ok(catalog::variants(tx, &ids)
        .await?
        .into_iter()
        .map(|v| {
            (
                v.variant_id.clone(),
                ItemFacts {
                    product_id: Some(v.product_id),
                    size: v.size,
                    color: v.color,
                },
            )
        })
        .collect())
}

fn item_json(
    i: &repo::ItemRow,
    facts: &HashMap<String, ItemFacts>,
    full: bool,
) -> Result<Value, ToolError> {
    let f = facts.get(&i.product_id);
    let mut v = json!({
        "id": i.id,
        "product_id": f.and_then(|f| f.product_id.clone()),
        "variant_id": i.product_id,
        "name": i.name,
        "size": f.and_then(|f| f.size.clone()),
        "color": f.and_then(|f| f.color.clone()),
        "order_quantity": i.order_quantity,
    });
    if full {
        v["price"] = json!(repo::money(i.price, &i.price_currency)?);
        v["discount"] = json!(repo::money(i.discount_value, &i.discount_currency)?);
    }
    v["order_item_status"] = json!(i.order_item_status);
    Ok(v)
}

fn order_not_found(id: &str) -> ToolError {
    ToolError::new(ORDER_NOT_FOUND, format!("There is no order {id}."))
        .detail(json!({"entity": "order", "id": id}))
        .next("search_orders")
}

/// Loads an order the caller may see.
pub(crate) async fn visible_order(cx: &Cx, tx: &mut TxCx, id: &str) -> Result<OrderRow, ToolError> {
    let o = repo::order(tx, id)
        .await?
        .ok_or_else(|| order_not_found(id))?;
    match owner::viewer(&cx.principal) {
        Viewer::Staff => Ok(o),
        Viewer::Shopper(c) if o.customer_id.as_deref() == Some(c.as_str()) => Ok(o),
        _ => Err(
            ToolError::new(ErrorCode::FORBIDDEN, "This is not your order.")
                .detail(json!({"reason": "owner"}))
                .next("get_my_orders"),
        ),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetMyOrdersInput {
    /// Only orders in these states. All when left out.
    #[schemars(example = ["delivered"])]
    pub order_status: Option<Vec<OrderStatus>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub(crate) struct GetMyOrders;

impl Operation for GetMyOrders {
    const META: OpMeta = OpMeta {
        address: "sales.order.list_mine",
        tool: "get_my_orders",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/orders/mine",
        summary: "The caller's own orders, newest first, with their lines and when they were delivered. Where returns and exchanges start.",
        consumes: &[],
        produces: &[Slot::many("order_id"), Slot::many("order_item_id")],
        checks: &[],
        errors: &[NOT_SIGNED_IN],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetMyOrdersInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetMyOrders {
    async fn read(&self, cx: &Cx, input: GetMyOrdersInput) -> Result<Value, ToolError> {
        let Viewer::Shopper(customer) = owner::viewer(&cx.principal) else {
            return Err(not_signed_in());
        };
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let filter = repo::OrderFilter {
            customer_id: Some(customer),
            statuses: input
                .order_status
                .unwrap_or_default()
                .iter()
                .map(|s| s.as_str().to_owned())
                .collect(),
            order_by: "order_date DESC, id DESC",
            limit: page.limit,
            offset: page.offset,
            ..repo::OrderFilter::default()
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (orders, total) = repo::orders(&mut tx, &filter).await?;
        let ids: Vec<String> = orders.iter().map(|o| o.id.clone()).collect();
        let items = repo::items(&mut tx, &ids).await?;
        let delivered = repo::delivered_at(&mut tx, &ids).await?;
        let facts = item_facts(&mut tx, &items).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut out = Vec::with_capacity(orders.len());
        for o in &orders {
            let lines: Vec<Value> = items
                .iter()
                .filter(|i| i.order_id == o.id)
                .map(|i| item_json(i, &facts, false))
                .collect::<Result<_, _>>()?;
            let delivered_at = (o.order_status == "delivered")
                .then(|| delivered.iter().find(|(id, _)| id == &o.id))
                .flatten()
                .map(|(_, t)| cx.time.format(*t));
            out.push(json!({
                "id": o.id, "order_number": o.order_number, "order_status": o.order_status,
                "order_date": cx.time.format(o.order_date), "ordered_item": lines,
                "delivered_at": delivered_at,
                "total": repo::money(o.total_payment_due_value, &o.total_payment_due_currency)?,
            }));
        }
        let mut v = json!({"items": out, "total": total});
        if let Some(c) = page.next_cursor(orders.len(), total) {
            v["next_cursor"] = json!(c);
        }
        Ok(v)
    }
}

/// Top-level fields of an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrderField {
    /// The order id.
    Id,
    /// The number people see.
    OrderNumber,
    /// Where the order is.
    OrderStatus,
    /// When it was placed.
    OrderDate,
    /// Who placed it.
    CustomerId,
    /// The lines.
    OrderedItem,
    /// The parcel.
    Delivery,
    /// The payment.
    Payment,
    /// Subtotal, discounts, shipping, total.
    Totals,
    /// Coupons the order recorded.
    CouponsRecorded,
    /// Cancel, return and exchange requests on the order.
    Claims,
}

impl OrderField {
    fn word(self) -> &'static str {
        match self {
            OrderField::Id => "id",
            OrderField::OrderNumber => "order_number",
            OrderField::OrderStatus => "order_status",
            OrderField::OrderDate => "order_date",
            OrderField::CustomerId => "customer_id",
            OrderField::Claims => "claims",
            OrderField::OrderedItem => "ordered_item",
            OrderField::Delivery => "delivery",
            OrderField::Payment => "payment",
            OrderField::Totals => "totals",
            OrderField::CouponsRecorded => "coupons_recorded",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetOrderInput {
    /// The order (`get_my_orders`, `search_orders`).
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20402")]
    pub order_id: String,
    /// Only these top-level fields. All when left out.
    #[schemars(example = ["ordered_item", "payment"])]
    pub fields: Option<Vec<OrderField>>,
}

pub(crate) struct GetOrder;

impl Operation for GetOrder {
    const META: OpMeta = OpMeta {
        address: "sales.order.get",
        tool: "get_order",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/orders/{order_id}",
        summary: "One order: lines with their own status, delivery, payment (card last four only), totals and recorded coupons. Operators see every order, shoppers their own.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::many("order_item_id"), Slot::one("delivery_id")],
        checks: &[],
        errors: &[ORDER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetOrderInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetOrder {
    async fn read(&self, cx: &Cx, input: GetOrderInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let o = visible_order(cx, &mut tx, &input.order_id).await?;
        let ids = vec![o.id.clone()];
        let items = repo::items(&mut tx, &ids).await?;
        let facts = item_facts(&mut tx, &items).await?;
        let delivered = repo::delivered_at(&mut tx, &ids).await?;
        let record = repo::placed_record(&mut tx, &o.id)
            .await?
            .unwrap_or(Value::Null);
        let parcel = match crate::service::providers::deliveries() {
            Some(read) => read(&mut tx, &o.id).await?,
            None => None,
        };
        let claims = match crate::service::providers::claims() {
            Some(read) => read(&mut tx, &o.id).await?,
            None => Vec::new(),
        };
        let payment = repo::payment(&mut tx, &o.id).await?;
        // Orders placed before discounts were recorded in full: one line per promotion.
        let discounts = match record.get("discounts") {
            Some(d) if d.is_array() => d.clone(),
            _ => {
                let mut by: Vec<(Option<String>, Decimal, String)> = Vec::new();
                for i in items.iter().filter(|i| i.discount_value > Decimal::ZERO) {
                    match by.iter_mut().find(|(p, _, _)| p == &i.promotion_id) {
                        Some((_, a, _)) => *a += i.discount_value,
                        None => by.push((
                            i.promotion_id.clone(),
                            i.discount_value,
                            i.discount_currency.clone(),
                        )),
                    }
                }
                let promotion_ids: Vec<String> =
                    by.iter().filter_map(|(p, _, _)| p.clone()).collect();
                let names = promotion::promotion_names(&mut tx, &promotion_ids).await?;
                let mut out = Vec::new();
                for (p, amount, currency) in by {
                    let label = p
                        .as_ref()
                        .and_then(|p| names.iter().find(|(id, _)| id == p).map(|(_, n)| n.clone()))
                        .or_else(|| o.discount_code.clone())
                        .unwrap_or_else(|| "Discount".to_owned());
                    out.push(
                        json!({"label": label, "amount": repo::money(amount, &currency)?,
                                    "promotion_id": p}),
                    );
                }
                json!(out)
            }
        };
        let coupons = match record.get("coupons_recorded") {
            Some(c) if c.is_array() => c.clone(),
            _ => {
                let ids: Vec<String> = o.coupon_id.iter().cloned().collect();
                let facts = promotion::coupon_facts(&mut tx, &ids).await?;
                json!(
                    facts
                        .iter()
                        .map(|c| json!({"coupon_id": c.coupon_id, "code": c.code,
                                    "promotion_id": c.promotion_id}))
                        .collect::<Vec<_>>()
                )
            }
        };
        tx.commit().await.map_err(ToolError::from)?;

        let delivered_at = (o.order_status == "delivered")
            .then(|| delivered.first().map(|(_, t)| cx.time.format(*t)))
            .flatten();
        let delivery = match parcel {
            Some(p) => json!({
                "id": p.id, "delivery_address": p.delivery_address,
                "expected_arrival_until": p.expected_arrival_until.map(|t| cx.time.format(t)),
                "delivery_state": p.delivery_state,
                "delivered_at": p.delivered_at.map(|t| cx.time.format(t)),
            }),
            // Without fulfillment's reader: what the order recorded at checkout.
            None => {
                let d = &record["delivery"];
                json!({
                    "id": null,
                    "delivery_address": d.get("delivery_address").cloned().unwrap_or(Value::Null),
                    "expected_arrival_until": null,
                    "delivery_state": null,
                    "delivered_at": delivered_at,
                })
            }
        };
        let payment = match payment {
            Some(p) => json!({"payment_method": p.payment_method, "card_last4": p.card_last4,
                              "payment_status": p.payment_status}),
            None => Value::Null,
        };
        let lines: Vec<Value> = items
            .iter()
            .map(|i| item_json(i, &facts, true))
            .collect::<Result<_, _>>()?;
        let out = json!({
            "id": o.id, "order_number": o.order_number, "order_status": o.order_status,
            "order_date": cx.time.format(o.order_date), "customer_id": o.customer_id,
            "ordered_item": lines,
            "delivery": delivery,
            "payment": payment,
            "totals": {
                "subtotal": repo::money(o.subtotal_value, &o.subtotal_currency)?,
                "discounts": discounts,
                "shipping": repo::money(o.shipping_value, &o.shipping_currency)?,
                "total": repo::money(o.total_payment_due_value, &o.total_payment_due_currency)?,
            },
            "coupons_recorded": coupons,
            "claims": claims.iter().map(|c| json!({
                "id": c.id, "claim_type": c.claim_type, "claim_status": c.claim_status,
                "order_item_ids": c.order_item_ids,
                "requested_at": cx.time.format(c.requested_at),
            })).collect::<Vec<_>>(),
        });
        let fields: Vec<String> = input
            .fields
            .unwrap_or_default()
            .iter()
            .map(|f| f.word().to_owned())
            .collect();
        Ok(pricing::pick(out, &fields))
    }
}

/// How to order the search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum OrderSort {
    /// Oldest first.
    #[serde(rename = "order_date")]
    OrderDate,
    /// Newest first (the default).
    #[serde(rename = "-order_date")]
    OrderDateDescending,
    /// Smallest total first.
    #[serde(rename = "total")]
    Total,
    /// Largest total first.
    #[serde(rename = "-total")]
    TotalDescending,
}

/// Fields of a search result row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrderRowField {
    /// The order id.
    Id,
    /// The number people see.
    OrderNumber,
    /// Where the order is.
    OrderStatus,
    /// Who placed it.
    CustomerId,
    /// When it was placed.
    OrderDate,
    /// What the shopper pays.
    Total,
}

impl OrderRowField {
    fn word(self) -> &'static str {
        match self {
            OrderRowField::Id => "id",
            OrderRowField::OrderNumber => "order_number",
            OrderRowField::OrderStatus => "order_status",
            OrderRowField::CustomerId => "customer_id",
            OrderRowField::OrderDate => "order_date",
            OrderRowField::Total => "total",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchOrdersInput {
    /// Only orders in these states.
    #[schemars(example = ["processing", "payment_due"])]
    pub order_status: Option<Vec<OrderStatus>>,
    /// Only this customer's orders.
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_alex")]
    pub customer_id: Option<String>,
    /// Placed on or after this day (store time).
    #[schemars(example = &"2026-10-01")]
    pub order_date_from: Option<String>,
    /// Placed on or before this day (store time).
    #[schemars(example = &"2026-11-06")]
    pub order_date_to: Option<String>,
    /// Only orders of this seller. Sellers are not recorded on orders yet.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_northwind")]
    pub seller_id: Option<String>,
    /// Order of the results. Default newest first.
    #[schemars(example = &"-order_date")]
    pub sort: Option<OrderSort>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
    /// Only these fields of each row. All when left out.
    #[schemars(example = ["id", "order_number", "total"])]
    pub fields: Option<Vec<OrderRowField>>,
}

pub(crate) struct SearchOrders;

impl Operation for SearchOrders {
    const META: OpMeta = OpMeta {
        address: "sales.order.search",
        tool: "search_orders",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/orders",
        summary: "Operators' order search by state, customer and date, sorted by date or total.",
        consumes: &[],
        produces: &[Slot::many("order_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchOrdersInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchOrders {
    async fn read(&self, cx: &Cx, input: SearchOrdersInput) -> Result<Value, ToolError> {
        if owner::viewer(&cx.principal) != Viewer::Staff {
            return Err(
                ToolError::new(ErrorCode::FORBIDDEN, "Order search is for operators.")
                    .detail(json!({"reason": "role"}))
                    .next("get_my_orders"),
            );
        }
        if input.seller_id.is_some() {
            return Err(ToolError::new(
                INVALID_FILTER,
                "Orders do not record a seller yet, so they cannot be filtered by one.",
            )
            .detail(json!({"field": "seller_id", "reason": "unsupported"}))
            .next("list_enums"));
        }
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let from = parse_time(cx, "order_date_from", input.order_date_from.as_deref())?;
        let until = parse_time(cx, "order_date_to", input.order_date_to.as_deref())?
            .map(|t| t + Duration::days(1));
        let filter = repo::OrderFilter {
            customer_id: input.customer_id,
            statuses: input
                .order_status
                .unwrap_or_default()
                .iter()
                .map(|s| s.as_str().to_owned())
                .collect(),
            from,
            until,
            order_by: match input.sort {
                Some(OrderSort::OrderDate) => "order_date, id",
                Some(OrderSort::Total) => "total_payment_due_value, id",
                Some(OrderSort::TotalDescending) => "total_payment_due_value DESC, id",
                _ => "order_date DESC, id DESC",
            },
            limit: page.limit,
            offset: page.offset,
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (orders, total) = repo::orders(&mut tx, &filter).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let fields: Vec<String> = input
            .fields
            .unwrap_or_default()
            .iter()
            .map(|f| f.word().to_owned())
            .collect();
        let items: Vec<Value> = orders
            .iter()
            .map(|o| {
                Ok(pricing::pick(
                    json!({"id": o.id, "order_number": o.order_number,
                           "order_status": o.order_status, "customer_id": o.customer_id,
                           "order_date": cx.time.format(o.order_date),
                           "total": repo::money(o.total_payment_due_value, &o.total_payment_due_currency)?}),
                    &fields,
                ))
            })
            .collect::<Result<_, ToolError>>()?;
        let mut v = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(orders.len(), total) {
            v["next_cursor"] = json!(c);
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cart::{GetCart, GetCartInput};
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schemas_are_closed_and_flat() {
        for s in [
            closed_schema_for::<PrepareOrderInput>(),
            closed_schema_for::<PlaceOrderInput>(),
            closed_schema_for::<GetMyOrdersInput>(),
            closed_schema_for::<GetOrderInput>(),
            closed_schema_for::<SearchOrdersInput>(),
        ] {
            assert!(violations(&s).is_empty(), "{s:#}");
        }
        let p = closed_schema_for::<PrepareOrderInput>();
        assert_eq!(p["required"], json!(["delivery_mode", "client_total"]));
        assert_eq!(
            p["properties"]["delivery_address"]["additionalProperties"],
            json!(false)
        );
        let s = closed_schema_for::<SearchOrdersInput>();
        assert_eq!(
            s["properties"]["sort"]["enum"],
            json!(["order_date", "-order_date", "total", "-total"])
        );
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

    /// Integration: order reads for Alex's #20402 (`DATABASE_URL`, demo seed).
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reads_alexs_order() {
        let alex = crate::testdb::cx("cus_alex").await;
        let mine = GetMyOrders
            .read(
                &alex,
                GetMyOrdersInput {
                    order_status: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap();
        assert!(
            mine["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["id"] == "ord_20402")
        );
        let o = GetOrder
            .read(
                &alex,
                GetOrderInput {
                    order_id: "ord_20402".into(),
                    fields: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(o["totals"]["total"]["value"].to_string(), "312.00");
        assert_eq!(o["payment"]["card_last4"], "0937");
        let jamie = crate::testdb::cx("cus_jamie").await;
        let e = GetOrder
            .read(
                &jamie,
                GetOrderInput {
                    order_id: "ord_20402".into(),
                    fields: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let staff = crate::testdb::cx("adm_sj").await;
        let found = SearchOrders
            .read(
                &staff,
                SearchOrdersInput {
                    order_status: Some(vec![OrderStatus::Delivered]),
                    customer_id: Some("cus_alex".into()),
                    order_date_from: None,
                    order_date_to: None,
                    seller_id: None,
                    sort: None,
                    limit: None,
                    cursor: None,
                    fields: None,
                },
            )
            .await
            .unwrap();
        assert!(found["total"].as_i64().unwrap() >= 1);
    }
}
