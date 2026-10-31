//! Gift orders (`docs/api/sales.md` §gift_order): `prepare_gift_order`, which only prepares a
//! gift from the caller's own cart for the sender's Place order button (the same person-only
//! `POST /v1/sales/orders` that places any order); `get_gift_order`, what the recipient sees
//! through the link; and the recipient's person-only
//! `POST /v1/sales/gift-orders/{gift_order_id}/accept`.
//!
//! The recipient's name, address, contact and size are used for this one order and never
//! saved to the sender's profile or address book. The wrap fee, the gift receipt, the exchange
//! deadline and how long a recipient has to accept come from `policies/order.yml` (`gift`) and
//! `policies/fulfillment.yml` (`gift_exchange_until`).

use acom_catalog::service::for_fulfillment as stock;
use acom_catalog::service::for_sales::{self as catalog, VariantMatch};
use acom_changeset::LogEntry;
use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind, Method,
    OpMeta, Operation, PersonOp, Planned, ReadOp, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::ActionStatus;
use acom_core::ids::OrderId;
use acom_core::{Money, StoreTime};
use acom_customer::service::for_sales as customer;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::checkout;
use crate::delivery::{self, Address};
use crate::order::{self, PreparedOrder};
use crate::pricing::{self, CartView, DeliveryChoice};
use crate::repo;
use crate::rules::{self, GiftRules};
use crate::{
    ADDRESS_NOT_DELIVERABLE, CART_EMPTY, GIFT_EXPIRED, MIXED_ORDER_TYPE, NO_COMMON_PAYMENT_METHOD,
    NOT_SIGNED_IN, OUT_OF_STOCK, approval,
};

/// The tool whose previews `POST /v1/sales/orders` also places.
pub(crate) const PREPARE_TOOL: &str = "prepare_gift_order";

/// Longest card message.
const CARD_MESSAGE_CHARS: usize = 300;

/// How the gift reaches its recipient (`gift.recipient_modes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecipientMode {
    /// The sender knows the address (`delivery_address`); the gift ships there.
    AddressKnown,
    /// The recipient gets a link (`recipient_contact`), gives the address and options, and
    /// payment waits until they accept.
    RecipientAccepts,
}

impl RecipientMode {
    pub fn word(self) -> &'static str {
        match self {
            Self::AddressKnown => "address_known",
            Self::RecipientAccepts => "recipient_accepts",
        }
    }
}

/// An address for this gift only. Region and postal code may be left out when the sender does
/// not know them; the carrier label then reads street and town.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GiftAddressInput {
    /// Name on the parcel. The recipient's name when left out.
    #[schemars(length(max = 100), example = &"Dad")]
    pub name: Option<String>,
    /// Street and number.
    #[schemars(length(min = 1, max = 200), example = &"48 Pine Rd")]
    pub street_address: String,
    /// Town or city.
    #[schemars(length(min = 1, max = 100), example = &"Lakeview")]
    pub address_locality: String,
    /// State or region.
    #[schemars(length(max = 100), example = &"NY")]
    pub address_region: Option<String>,
    /// Postal code.
    #[schemars(length(max = 20), example = &"12801")]
    pub postal_code: Option<String>,
    /// ISO 3166-1 country code.
    #[schemars(regex(pattern = r"^[A-Z]{2}$"), example = &"US")]
    pub address_country: String,
}

impl GiftAddressInput {
    fn address(&self, recipient: &str) -> Address {
        let name = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(recipient);
        Address {
            name: Some(name.to_owned()),
            street_address: self.street_address.trim().to_owned(),
            address_locality: self.address_locality.trim().to_owned(),
            address_region: self
                .address_region
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_owned),
            postal_code: self
                .postal_code
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .to_owned(),
            address_country: self.address_country.trim().to_owned(),
        }
    }
}

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

/// What `prepare_gift_order` hands to the sender's Place order button.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PreparedGift {
    /// The order part, as `prepare_order` plans it; `total` includes the wrap.
    pub order: PreparedOrder,
    pub mode: RecipientMode,
    pub recipient_name: String,
    pub recipient_contact: Option<String>,
    pub card_message: Option<String>,
    /// The wrap fee, when wrapped.
    pub gift_wrap: Option<Money>,
    pub gift_receipt: bool,
    pub receipt_hides_prices: bool,
    /// The last day the recipient can swap the size.
    pub exchange_until: NaiveDate,
    /// Days a `recipient_accepts` gift waits for its recipient.
    pub accept_days: u32,
}

/// A placed gift that waits for its recipient.
pub(crate) struct Acceptance {
    pub gift_order_id: String,
    pub expires: DateTime<Utc>,
    pub accept_url: String,
}

/// The goods' total plus the wrap fee.
pub(crate) fn total_of(view: &CartView, wrap: Option<Money>) -> Result<Money, ToolError> {
    match wrap {
        None => Ok(view.breakdown.total),
        Some(w) => view.breakdown.total.checked_add(w).map_err(|e| {
            ToolError::new(
                ErrorCode::INTERNAL,
                format!("the gift wrap fee cannot be added to the order: {e}"),
            )
            .detail(json!({"policy_key": "order.gift.wrap_fee"}))
        }),
    }
}

fn same(a: Money, b: Money) -> bool {
    a.currency == b.currency && a.value == b.value
}

/// The gift's checks: one recipient, then `order.checkout_validation`. The total the sender
/// saw may be the cart's (before the wrap) or the gift's (with it); either must match.
pub(crate) fn checks(
    view: &CartView,
    required: &[String],
    gift: &PreparedGift,
    client_total: Money,
    floor: Option<Decimal>,
    total: Money,
) -> Vec<Check> {
    let mut out = vec![Check {
        rule: "gift.recipient_per_order".into(),
        passed: true,
        detail: Some(format!("One recipient: {}", gift.recipient_name)),
        fix_tool: None,
    }];
    let seen = if same(client_total, total) {
        view.breakdown.total
    } else {
        client_total
    };
    let mut cc = checkout::checkout_checks(view, required, seen, floor);
    if let Some(w) = gift.gift_wrap {
        for c in cc
            .iter_mut()
            .filter(|c| c.rule == "checkout_validation.server_total_matches_client")
        {
            if let Some(d) = &mut c.detail {
                d.push_str(&format!(
                    "; with the gift wrap ({}) the gift comes to {}",
                    w.value, total.value
                ));
            }
        }
    }
    out.extend(cc);
    out
}

impl PreparedGift {
    pub fn waits_for_recipient(&self) -> bool {
        self.mode == RecipientMode::RecipientAccepts
    }

    fn options(&self) -> Value {
        json!({
            "wrap": self.gift_wrap.is_some(),
            "wrap_fee": self.gift_wrap,
            "card_message": self.card_message,
            "gift_receipt": self.gift_receipt,
            "receipt_hides_prices": self.receipt_hides_prices,
            "exchange_until": self.exchange_until.format("%Y-%m-%d").to_string(),
        })
    }

    /// `sales_order.gift_options`.
    pub fn options_json(&self) -> Value {
        let mut v = self.options();
        v["recipient"] = json!(self.recipient_name);
        v["recipient_mode"] = json!(self.mode.word());
        v
    }

    /// What the order event records about the gift.
    pub fn record_json(&self) -> Value {
        let mut v = self.options();
        v["recipient"] = json!({"name": self.recipient_name, "mode": self.mode.word()});
        v
    }

    /// The `create order` change the gift's preview shows (and the gift picker and card read).
    pub fn change(&self, view: &CartView, total: Money) -> Change {
        let mut to = pricing::order_json(view);
        to["order_type"] = json!("gift");
        to["gift_wrap"] = json!(self.gift_wrap);
        to["goods_total"] = json!(view.breakdown.total);
        to["total"] = json!(total);
        to["delivery"] = match self.mode {
            RecipientMode::AddressKnown => view.destination.as_ref().map_or(Value::Null, |d| {
                json!({"to": self.recipient_name,
                       "address_label": delivery::full_label(d.label.as_deref(), &d.address),
                       "arrives": d.arrives.format("%Y-%m-%d").to_string(),
                       "address_saved": false})
            }),
            RecipientMode::RecipientAccepts => json!({
                "to": self.recipient_name, "address_label": null, "arrives": null,
                "address_saved": false, "recipient_contact": self.recipient_contact,
                "accept_within_days": self.accept_days}),
        };
        to["gift_options"] = self.options();
        to["recipient"] = json!({"name": self.recipient_name, "mode": self.mode.word()});
        Change {
            op: ChangeOp::Create,
            entity: "order".into(),
            id: String::new(),
            from: Value::Null,
            to,
            note: None,
        }
    }

    /// For a gift the recipient accepts: the row that waits for them.
    pub async fn place_acceptance(
        &self,
        tx: &mut TxCx,
        order_id: &str,
    ) -> Result<Option<Acceptance>, ToolError> {
        if !self.waits_for_recipient() {
            return Ok(None);
        }
        let id = format!("gift_{}", acom_core::id::new_suffix());
        let expires = tx.clock.now() + Duration::days(i64::from(self.accept_days));
        let contact = self.recipient_contact.clone().unwrap_or_default();
        repo::insert_gift_order(tx, &id, order_id, &self.recipient_name, &contact, expires).await?;
        Ok(Some(Acceptance {
            accept_url: format!("/gifts/{id}"),
            gift_order_id: id,
            expires,
        }))
    }
}

/// `gift_exchange_until` in the year of `today`, or the next year once it has passed.
fn exchange_date(today: NaiveDate, (month, day): (u32, u32)) -> NaiveDate {
    let this_year = NaiveDate::from_ymd_opt(today.year(), month, day);
    match this_year {
        Some(d) if d >= today => d,
        _ => NaiveDate::from_ymd_opt(today.year() + 1, month, day).unwrap_or(today),
    }
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetGiftOrderInput {
    /// The gift, from the accept link.
    #[schemars(regex(pattern = r"^gift_[a-z0-9_]+$"), example = &"gift_01jc9x2m")]
    pub gift_order_id: String,
}

/// What the recipient sees through the link. The link is the permission, so nothing about the
/// sender's contact, address or payment, or the recipient's contact, is shown, and no price
/// (`gift.gift_receipt_hides_prices`).
pub(crate) struct GetGiftOrder;

impl Operation for GetGiftOrder {
    const META: OpMeta = OpMeta {
        address: "sales.gift_order.get",
        tool: "get_gift_order",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/gift-orders/{gift_order_id}",
        summary: "A gift as its recipient opens it from the link: what it is, who sent it and their card, the sizes and colors they may switch to at the same price, and until when they can accept. No prices.",
        consumes: &[Slot::one("gift_order_id")],
        produces: &[Slot::one("gift_order_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetGiftOrderInput;
    type Output = Value;
}

fn no_such_gift(id: &str) -> ToolError {
    ToolError::new(
        ErrorCode::NOT_FOUND,
        "There is no such gift; check the link.",
    )
    .detail(json!({"entity": "gift_order", "id": id}))
}

impl ReadOp<Cx> for GetGiftOrder {
    async fn read(&self, cx: &Cx, input: GetGiftOrderInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let view = gift_view(&mut tx, &input.gift_order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(view)
    }
}

/// Where an acceptance stands at `now`: one still waiting past its deadline has lapsed.
pub(crate) fn acceptance_status(g: &repo::GiftOrderRow, now: DateTime<Utc>) -> &str {
    if g.action_status == ActionStatus::Potential.as_str() && now > g.expires {
        ActionStatus::Failed.as_str()
    } else {
        &g.action_status
    }
}

/// `get_order`'s `gift`: who it is for, the card, and the link's acceptance when it has one.
pub(crate) fn order_gift(
    options: &Value,
    acceptance: Option<&repo::GiftOrderRow>,
    now: DateTime<Utc>,
    time: &StoreTime,
) -> Value {
    let field = |k: &str| options.get(k).cloned().unwrap_or(Value::Null);
    json!({
        "recipient_name": field("recipient"),
        "recipient_mode": field("recipient_mode"),
        "card_message": field("card_message"),
        "gift_receipt": field("gift_receipt"),
        "exchange_until": field("exchange_until"),
        "acceptance": acceptance.map(|g| json!({
            "gift_order_id": g.id,
            "accept_url": format!("/gifts/{}", g.id),
            "action_status": acceptance_status(g, now),
            "expires": time.format(g.expires),
        })),
    })
}

/// `get_gift_order`'s answer, read in `tx`.
pub(crate) async fn gift_view(tx: &mut TxCx, id: &str) -> Result<Value, ToolError> {
    let gift_rules = rules::gift_rules(&tx.policy)?;
    let g = repo::gift_order(tx, id)
        .await?
        .ok_or_else(|| no_such_gift(id))?;
    let order = repo::order(tx, &g.order_id)
        .await?
        .ok_or_else(|| ToolError::new(ErrorCode::INTERNAL, "gift without its order"))?;
    let options = order.gift_options.clone().unwrap_or_default();
    let from = match order.customer_id.as_deref() {
        Some(c) => customer::shopper(tx, c).await?.map(|s| s.name),
        None => None,
    };
    let items = repo::items(tx, std::slice::from_ref(&g.order_id)).await?;
    let ids: Vec<String> = items.iter().map(|i| i.product_id.clone()).collect();
    let variants = catalog::variants(tx, &ids).await?;
    let may_switch = items.len() == 1
        && gift_rules
            .recipient_can_change
            .iter()
            .any(|c| c == "options");
    let mut lines = Vec::new();
    for item in &items {
        let v = variants.iter().find(|v| v.variant_id == item.product_id);
        let mut choices = Vec::new();
        if let Some(v) = v.filter(|_| may_switch) {
            // Same product, same list price (`gift.recipient_can_change`); the chosen one is
            // held already.
            let quantity = i64::from(item.order_quantity);
            for o in catalog::product_variants(tx, &v.product_id).await? {
                if o.approval_status != "active" || !same(o.list_price, v.list_price) {
                    continue;
                }
                let available =
                    o.variant_id == v.variant_id || (o.sellable() && o.has_stock(quantity));
                choices.push(json!({"size": o.size, "color": o.color, "available": available}));
            }
        }
        lines.push(json!({
            "name": item.name,
            "image": v.and_then(|v| v.image.clone()),
            "size": v.and_then(|v| v.size.clone()),
            "color": v.and_then(|v| v.color.clone()),
            "quantity": item.order_quantity,
            "options": choices,
        }));
    }

    Ok(json!({
        "id": g.id,
        "action_status": acceptance_status(&g, tx.clock.now()),
        "expires": tx.time.format(g.expires),
        "recipient_name": g.recipient_name,
        "from": from.as_deref().and_then(|n| n.split_whitespace().next()),
        "card_message": options.get("card_message").cloned().unwrap_or(Value::Null),
        "gift_wrap": options.get("wrap").and_then(Value::as_bool).unwrap_or(false),
        "exchange_until": options.get("exchange_until").cloned().unwrap_or(Value::Null),
        "items": lines,
        "can_change": gift_rules.recipient_can_change,
        "ships_to": rules::ships_to(&tx.policy),
    }))
}

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
    use acom_contract::schema::{closed_schema_for, violations};
    use acom_core::Currency;
    use std::str::FromStr;
    use std::sync::Arc;

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    #[test]
    fn input_schemas_are_closed_and_flat() {
        for s in [
            closed_schema_for::<PrepareGiftOrderInput>(),
            closed_schema_for::<AcceptGiftOrderInput>(),
            closed_schema_for::<GetGiftOrderInput>(),
        ] {
            assert!(violations(&s).is_empty(), "{s:#}");
        }
        let p = closed_schema_for::<PrepareGiftOrderInput>();
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
        let a = closed_schema_for::<AcceptGiftOrderInput>();
        assert_eq!(a["required"], json!(["gift_order_id", "delivery_address"]));
    }

    #[test]
    fn the_exchange_deadline_is_this_years_until_it_passes() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        assert_eq!(exchange_date(d("2026-11-07"), (12, 31)), d("2026-12-31"));
        assert_eq!(exchange_date(d("2027-01-05"), (12, 31)), d("2027-12-31"));
        assert_eq!(exchange_date(d("2026-12-31"), (12, 31)), d("2026-12-31"));
    }

    #[test]
    fn a_gift_address_names_the_recipient_and_may_lack_a_postal_code() {
        let a: GiftAddressInput = serde_json::from_value(json!({
            "street_address": "48 Pine Rd", "address_locality": "Lakeview", "address_country": "US"}))
        .unwrap();
        let addr = a.address("Dad");
        assert_eq!(addr.name.as_deref(), Some("Dad"));
        assert_eq!(addr.postal_code, "");
        assert_eq!(
            delivery::full_label(addr.name.as_deref(), &addr),
            "Dad · 48 Pine Rd, Lakeview"
        );
        assert!(
            serde_json::from_value::<GiftAddressInput>(json!({"street_address": "x",
                "address_locality": "y", "address_country": "US", "floor": 3}))
            .is_err(),
            "unknown fields are refused"
        );
    }

    /// A context whose pool never connects: for inputs refused before any query.
    fn offline() -> Cx {
        let time = acom_core::StoreTime::new(chrono_tz::America::New_York);
        Cx {
            tenant: acom_core::TenantId::parse("store_northwind").unwrap(),
            principal: Arc::new(acom_auth::Principal {
                subject: "cus_jamie".into(),
                kind: acom_core::enums::ActorKind::Shopper,
                tenant: acom_core::TenantId::parse("store_northwind").unwrap(),
                surface: acom_auth::Surface::Fo,
                scopes: acom_auth::scopes::shopper(),
                name: None,
                session_id: None,
            }),
            request_id: "req_test".into(),
            clock: Arc::new(acom_core::FixedClock::at(
                time.parse("2026-11-07T10:00").unwrap(),
            )),
            time,
            currency: Currency::USD,
            policy: Arc::new(crate::rules::tests::defaults()),
            db: acom_store::Db::connect_lazy("postgres://offline.invalid/acom", 1).unwrap(),
        }
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

    #[test]
    fn prepared_gifts_survive_the_preview_store() {
        let g = PreparedGift {
            order: PreparedOrder {
                order_id: "ord_01".into(),
                customer_id: "cus_jamie".into(),
                lines: vec![],
                delivery: DeliveryChoice::Later,
                payment_method: "credit_card".into(),
                total: usd("76.20"),
                failed_checks: vec![],
            },
            mode: RecipientMode::RecipientAccepts,
            recipient_name: "Dad".into(),
            recipient_contact: Some("dad@example.com".into()),
            card_message: Some("For the next trail. Love, J.".into()),
            gift_wrap: Some(usd("5.00")),
            gift_receipt: true,
            receipt_hides_prices: true,
            exchange_until: NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
            accept_days: 3,
        };
        let v = serde_json::to_value(&g).unwrap();
        assert_eq!(v["mode"], "recipient_accepts");
        let back: PreparedGift = serde_json::from_value(v).unwrap();
        assert_eq!(back, g);
        assert!(back.waits_for_recipient());
        let o = g.options_json();
        assert_eq!(o["exchange_until"], "2026-12-31");
        assert_eq!(o["wrap"], true);
        assert_eq!(o["recipient"], "Dad");
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

        use crate::order::{PlaceOrder, PlaceOrderInput};

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
