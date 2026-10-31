//! The `gift_order` resource (`docs/api/sales.md` §gift_order): `prepare_gift_order`, which only prepares a
//! gift from the caller's own cart for the sender's Place order button (the same person-only
//! `POST /v1/sales/orders` that places any order); `get_gift_order`, what the recipient sees
//! through the link; and the recipient's person-only
//! `POST /v1/sales/gift-orders/{gift_order_id}/accept`.
//!
//! The recipient's name, address, contact and size are used for this one order and never
//! saved to the sender's profile or address book. The wrap fee, the gift receipt, the exchange
//! deadline and how long a recipient has to accept come from `policies/order.yml` (`gift`) and
//! `policies/fulfillment.yml` (`gift_exchange_until`).

pub mod accept_gift_order;
pub mod get_gift_order;
pub mod prepare_gift_order;

use acom_catalog::service::for_sales as catalog;
use acom_contract::{Change, ChangeOp, Check, ErrorCode, ToolError};
use acom_core::enums::ActionStatus;
use acom_core::{Money, StoreTime};
use acom_customer::service::for_sales as customer;
use acom_store::TxCx;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::checkout;
use crate::delivery::{self, Address};
use crate::order::PreparedOrder;
use crate::pricing::{self, CartView};
use crate::repo;
use crate::rules;

/// The tool whose previews `POST /v1/sales/orders` also places.
pub(crate) const PREPARE_TOOL: &str = "prepare_gift_order";

/// Longest card message.
pub(crate) const CARD_MESSAGE_CHARS: usize = 300;

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
    pub(crate) fn address(&self, recipient: &str) -> Address {
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

pub(crate) fn same(a: Money, b: Money) -> bool {
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
pub(crate) fn exchange_date(today: NaiveDate, (month, day): (u32, u32)) -> NaiveDate {
    let this_year = NaiveDate::from_ymd_opt(today.year(), month, day);
    match this_year {
        Some(d) if d >= today => d,
        _ => NaiveDate::from_ymd_opt(today.year() + 1, month, day).unwrap_or(today),
    }
}

pub(crate) fn no_such_gift(id: &str) -> ToolError {
    ToolError::new(
        ErrorCode::NOT_FOUND,
        "There is no such gift; check the link.",
    )
    .detail(json!({"entity": "gift_order", "id": id}))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pricing::DeliveryChoice;
    use acom_core::Currency;
    use std::str::FromStr;

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
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
}
