//! The shopper's cart, priced: their lines (with a guest cart they are taking over), what the
//! catalog says about each variant, every benefit from `calculate_benefits`, where it goes and
//! when it arrives, and how it is paid. The cart, the checkout card and order placement all
//! read the cart through here, so they show the same numbers.

use std::collections::HashMap;

use acom_catalog::service::for_sales::{self as catalog, VariantFacts};
use acom_contract::{Basis, ToolError};
use acom_core::ids::CartId;
use acom_customer::service::for_sales::{self as customer, SavedAddress};
use acom_promotion::service::for_sales::{self as promotion, Breakdown, Line};
use acom_store::TxCx;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::checkout::{self, PaymentOption};
use crate::delivery::{self, Address};
use crate::merge::{self, Absorb, LineRef, Merged};
use crate::owner::{Owner, Owners};
use crate::repo::{self, CartRow, LineRow};
use crate::rules::{self, CartLimits};

/// The stored carts behind what the shopper sees.
#[derive(Debug, Clone)]
pub(crate) struct Carts {
    pub owners: Owners,
    /// The shopper's (or the guest's) own cart.
    pub main: Option<CartRow>,
    /// A guest cart from before the shopper signed in, being taken over.
    pub guest: Option<CartRow>,
    pub rows: Vec<LineRow>,
    /// The lines as the shopper sees them.
    pub merged: Vec<Merged>,
}

fn refs(rows: &[LineRow]) -> Vec<LineRef> {
    rows.iter()
        .map(|l| LineRef {
            id: l.id.clone(),
            variant_id: l.product_id.clone(),
            quantity: u32::try_from(l.order_quantity).unwrap_or(0),
        })
        .collect()
}

pub(crate) async fn load_carts(
    tx: &mut TxCx,
    owners: &Owners,
    limits: CartLimits,
) -> Result<Carts, ToolError> {
    let main = match (&owners.owner.customer_id, &owners.owner.session_id) {
        (Some(c), _) => repo::customer_cart(tx, c).await?,
        (None, Some(s)) => repo::session_cart(tx, s).await?,
        (None, None) => None,
    };
    let guest = match &owners.guest_session {
        Some(s) => repo::session_cart(tx, s)
            .await?
            .filter(|g| main.as_ref().is_none_or(|m| m.id != g.id)),
        None => None,
    };
    let main_rows = match &main {
        Some(c) => repo::lines(tx, &c.id).await?,
        None => Vec::new(),
    };
    let guest_rows = match &guest {
        Some(c) => repo::lines(tx, &c.id).await?,
        None => Vec::new(),
    };
    let merged = merge::merge(
        &refs(&main_rows),
        &refs(&guest_rows),
        limits.max_quantity_per_line,
        limits.max_lines,
    );
    let mut rows = main_rows;
    rows.extend(guest_rows);
    Ok(Carts {
        owners: owners.clone(),
        main,
        guest,
        rows,
        merged,
    })
}

impl Carts {
    /// The cart the shopper sees: theirs, else the guest cart they are taking over.
    pub fn cart(&self) -> Option<&CartRow> {
        self.main.as_ref().or(self.guest.as_ref())
    }

    pub fn row(&self, line_id: &str) -> Option<&LineRow> {
        self.rows.iter().find(|r| r.id == line_id)
    }

    /// The merged line with this id (a guest line folded into another is found as that line).
    pub fn line(&self, line_id: &str) -> Option<&Merged> {
        self.merged
            .iter()
            .find(|m| m.id == line_id || m.absorbs.as_deref() == Some(line_id))
    }

    /// The cart rows and line rows a write reads, with their versions.
    pub fn basis(&self) -> Vec<Basis> {
        let mut out: Vec<Basis> = [&self.main, &self.guest]
            .into_iter()
            .flatten()
            .map(|c| Basis {
                entity: "sales.cart".into(),
                id: c.id.clone(),
                version: i64::from(c.version),
            })
            .collect();
        out.extend(self.rows.iter().map(|l| Basis {
            entity: "sales.cart_line".into(),
            id: l.id.clone(),
            version: i64::from(l.version),
        }));
        out
    }

    /// Where a write puts its lines, creating or taking over a cart as needed.
    pub fn target(&self) -> CartTarget {
        let owner = self.owners.owner.clone();
        match (&self.main, &self.guest) {
            (Some(m), g) => CartTarget {
                cart_id: m.id.clone(),
                owner,
                create: false,
                adopt: false,
                absorb: merge::absorb(g.as_ref().map(|g| g.id.as_str()), &self.merged),
            },
            (None, Some(g)) => CartTarget {
                cart_id: g.id.clone(),
                owner,
                create: false,
                adopt: true,
                absorb: None,
            },
            (None, None) => CartTarget {
                cart_id: CartId::generate().to_string(),
                owner,
                create: true,
                adopt: false,
                absorb: None,
            },
        }
    }
}

/// Where a cart write lands, decided at preview time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CartTarget {
    pub cart_id: String,
    pub owner: Owner,
    /// Nobody had a cart: create this one.
    pub create: bool,
    /// Hand the guest cart `cart_id` to the signed-in shopper.
    pub adopt: bool,
    /// Move the guest cart's lines into `cart_id`.
    pub absorb: Option<Absorb>,
}

/// Makes the stored carts match the target: create, take over, or absorb. Safe to repeat.
/// Returns the cart to write to: the planned one, or the cart the owner got in the meantime
/// (two writes previewed before either created the cart).
pub(crate) async fn apply_target(tx: &mut TxCx, t: &CartTarget) -> Result<String, ToolError> {
    if t.create {
        let existing = match (&t.owner.customer_id, &t.owner.session_id) {
            (Some(c), _) => repo::customer_cart(tx, c).await?,
            (None, Some(s)) => repo::session_cart(tx, s).await?,
            (None, None) => None,
        };
        if let Some(c) = existing {
            return Ok(c.id);
        }
        repo::insert_cart(
            tx,
            &t.cart_id,
            t.owner.customer_id.as_deref(),
            t.owner.session_id.as_deref(),
            "web",
        )
        .await?;
    }
    if t.adopt
        && let Some(c) = &t.owner.customer_id
    {
        repo::adopt_cart(tx, &t.cart_id, c).await?;
    }
    if let Some(a) = &t.absorb {
        let folded: Vec<String> = a.folds.iter().map(|(g, _, _)| g.clone()).collect();
        repo::delete_lines(tx, &folded).await?;
        for (_, into, quantity) in &a.folds {
            repo::set_quantity(tx, into, *quantity).await?;
        }
        for (line, position) in &a.moves {
            repo::move_line(tx, line, &t.cart_id, *position).await?;
        }
        repo::touch_cart(tx, &a.guest_cart_id).await?;
    }
    Ok(t.cart_id.clone())
}

/// A cart line with what the catalog says about its variant.
#[derive(Debug, Clone)]
pub(crate) struct PricedLine {
    pub line_id: String,
    pub variant: VariantFacts,
    pub quantity: u32,
    pub note: Option<String>,
}

impl PricedLine {
    /// "Packable windbreaker M Slate", for check details.
    pub fn title(&self) -> String {
        let mut t = self.variant.product_name.clone();
        for part in [&self.variant.size, &self.variant.color]
            .into_iter()
            .flatten()
        {
            t.push(' ');
            t.push_str(part);
        }
        t
    }
}

/// Where the order goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Destination {
    /// The saved address (`adr_…`), `None` for a one-time address.
    pub address_id: Option<String>,
    /// The shopper's name for the address ("Home").
    pub label: Option<String>,
    pub address: Address,
    pub arrives: NaiveDate,
}

/// How the order is paid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PaymentChoice {
    pub payment_method: String,
    pub label: String,
}

/// Where the shopper wants it delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum DeliveryChoice {
    /// The cart's address, else the shopper's default shipping address.
    CartDefault,
    Saved(String),
    OneTime(Address),
    /// Nowhere yet: a gift whose recipient gives the address when accepting it.
    Later,
}

/// The priced cart.
#[derive(Debug, Clone)]
pub(crate) struct CartView {
    pub cart_id: Option<String>,
    pub customer_id: Option<String>,
    pub lines: Vec<PricedLine>,
    pub breakdown: Breakdown,
    pub destination: Option<Destination>,
    pub payment: Option<PaymentChoice>,
    pub options: Vec<PaymentOption>,
    pub card_last4: Option<String>,
}

pub(crate) fn address_of(a: &SavedAddress) -> Address {
    Address {
        name: a.name.clone(),
        street_address: a.street_address.clone(),
        address_locality: a.address_locality.clone(),
        address_region: a.address_region.clone(),
        postal_code: a.postal_code.clone(),
        address_country: a.address_country.clone(),
    }
}

/// Prices the shopper's cart. `Ok(Err(address_id))` when a chosen saved address is not theirs.
pub(crate) async fn view(
    tx: &mut TxCx,
    carts: &Carts,
    delivery: &DeliveryChoice,
    payment_method: Option<&str>,
) -> Result<Result<CartView, String>, ToolError> {
    let variant_ids: Vec<String> = carts.merged.iter().map(|m| m.variant_id.clone()).collect();
    let facts: HashMap<String, VariantFacts> = catalog::variants(tx, &variant_ids)
        .await?
        .into_iter()
        .map(|v| (v.variant_id.clone(), v))
        .collect();
    let lines: Vec<PricedLine> = carts
        .merged
        .iter()
        .filter_map(|m| {
            let variant = facts.get(&m.variant_id)?.clone();
            Some(PricedLine {
                line_id: m.id.clone(),
                variant,
                quantity: m.quantity,
                note: carts.row(&m.id).and_then(|r| r.description.clone()),
            })
        })
        .collect();

    let customer_id = carts.owners.owner.customer_id.clone();
    let shopper = match &customer_id {
        Some(c) => customer::shopper(tx, c).await?,
        None => None,
    };
    let tier_id = shopper.as_ref().and_then(|s| s.tier_id.clone());
    let card_last4 = shopper.as_ref().and_then(|s| s.card_last4.clone());

    let breakdown = promotion::calculate(
        tx,
        &promotion::Request {
            customer_id: customer_id.clone(),
            tier_id,
            lines: benefit_lines(&lines),
            coupon_ids: Vec::new(),
            spend_loyalty: None,
        },
    )
    .await?;

    let saved = match (delivery, &customer_id) {
        (DeliveryChoice::Saved(id), Some(c)) => match customer::address(tx, c, id).await? {
            Some(a) => Some(a),
            None => return Ok(Err(id.clone())),
        },
        (DeliveryChoice::Saved(id), None) => return Ok(Err(id.clone())),
        (DeliveryChoice::CartDefault, Some(c)) => {
            let chosen = match carts.cart().and_then(|k| k.delivery_address_id.clone()) {
                Some(id) => customer::address(tx, c, &id).await?,
                None => None,
            };
            match chosen {
                Some(a) => Some(a),
                None => customer::default_shipping_address(tx, c).await?,
            }
        }
        _ => None,
    };
    let calendar = rules::ship_calendar(&tx.policy)?;
    let local_now = tx.clock.now().with_timezone(&tx.time.zone).naive_local();
    let arrives = delivery::arrival(local_now, &calendar);
    let destination = match (delivery, saved) {
        (DeliveryChoice::OneTime(a), _) => Some(Destination {
            address_id: None,
            label: a.name.clone(),
            address: a.clone(),
            arrives,
        }),
        (_, Some(s)) => Some(Destination {
            address_id: Some(s.id.clone()),
            label: s.label.clone(),
            address: address_of(&s),
            arrives,
        }),
        (_, None) => None,
    };

    let pay_rules = rules::payment_rules(&tx.policy)?;
    let chosen = payment_method
        .map(str::to_owned)
        .or_else(|| carts.cart().and_then(|c| c.payment_method.clone()));
    let options = checkout::payment_options(
        &pay_rules,
        card_last4.as_deref(),
        customer_id.is_some(),
        breakdown.total,
        chosen.as_deref(),
    );
    let payment = match chosen {
        Some(m) => Some(PaymentChoice {
            label: checkout::payment_label(&m, card_last4.as_deref()),
            payment_method: m,
        }),
        None if customer_id.is_some() => {
            options.iter().find(|o| o.default).map(|o| PaymentChoice {
                payment_method: o.payment_method.clone(),
                label: o.label.clone(),
            })
        }
        None => None,
    };

    Ok(Ok(CartView {
        cart_id: carts.cart().map(|c| c.id.clone()),
        customer_id,
        lines,
        breakdown,
        destination,
        payment,
        options,
        card_last4,
    }))
}

/// The lines as the benefit calculator takes them.
pub(crate) fn benefit_lines(lines: &[PricedLine]) -> Vec<Line> {
    lines
        .iter()
        .map(|l| Line {
            cart_line_id: l.line_id.clone(),
            variant_id: l.variant.variant_id.clone(),
            product_id: l.variant.product_id.clone(),
            offer_id: l.variant.offer_id.clone(),
            quantity: l.quantity,
            list_price: l.variant.list_price,
            cost: l.variant.cost,
        })
        .collect()
}

/// The `get_cart` output (and `cart` block): field names as in docs/api/sales.md.
pub(crate) fn cart_json(v: &CartView) -> Value {
    let b = &v.breakdown;
    json!({
        "id": v.cart_id,
        "customer_id": v.customer_id,
        "lines": v.lines.iter().map(|l| json!({
            "cart_line_id": l.line_id,
            "product_id": l.variant.product_id,
            "variant_id": l.variant.variant_id,
            "name": l.variant.product_name,
            "color": l.variant.color,
            "size": l.variant.size,
            "quantity": l.quantity,
            "price": promotion::money(l.variant.list_price.value, l.variant.list_price.currency),
            "note": l.note,
        })).collect::<Vec<_>>(),
        "subtotal": b.subtotal,
        "discounts": b.discounts(),
        "shipping": b.shipping,
        "total": b.total,
        "delivery": v.destination.as_ref().map(|d| json!({
            "address_id": d.address_id,
            "address_label": delivery::short_label(d.label.as_deref(), &d.address.street_address),
            "arrives": d.arrives.format("%Y-%m-%d").to_string(),
        })),
        "payment": v.payment.as_ref().map(|p| json!({
            "payment_method": p.payment_method,
            "label": p.label,
        })),
        "coupons_recorded": b.coupons_recorded.iter().map(|c| json!({
            "coupon_id": c.coupon_id,
            "code": c.code,
            "promotion_id": c.promotion_id,
        })).collect::<Vec<_>>(),
    })
}

/// The order a prepared checkout would place (the `create order` change and the checkout card).
pub(crate) fn order_json(v: &CartView) -> Value {
    let b = &v.breakdown;
    json!({
        "lines": v.lines.iter().map(|l| json!({
            "cart_line_id": l.line_id,
            "variant_id": l.variant.variant_id,
            "name": l.variant.product_name,
            "color": l.variant.color,
            "size": l.variant.size,
            "quantity": l.quantity,
            "price": promotion::money(l.variant.list_price.value, l.variant.list_price.currency),
        })).collect::<Vec<_>>(),
        "subtotal": b.subtotal,
        "discounts": b.discounts().iter().map(|d| json!({
            "label": d.label,
            "amount": d.amount,
            "promotion_id": d.promotion_id,
        })).collect::<Vec<_>>(),
        "shipping": b.shipping,
        "total": b.total,
        "delivery": v.destination.as_ref().map(|d| json!({
            "address_id": d.address_id,
            "address_label": delivery::full_label(d.label.as_deref(), &d.address),
            "arrives": d.arrives.format("%Y-%m-%d").to_string(),
        })),
        "payment": v.payment.as_ref().map(|p| json!({
            "payment_method": p.payment_method,
            "label": p.label,
        })),
        "coupons_recorded": b.coupons_recorded.iter().map(|c| json!({
            "coupon_id": c.coupon_id,
            "code": c.code,
        })).collect::<Vec<_>>(),
    })
}
