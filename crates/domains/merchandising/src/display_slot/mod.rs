//! The `display_slot` resource (docs/api/merchandising.md#display_slot): what its operations
//! share.
//!
//! Whether something shows is computed when asked, from the period, the stock and whether the
//! product can be sold (`display.visible_when`); nothing is switched on or off by a batch.

pub mod arrange_display_slot;
pub mod get_display_slot;
pub mod list_display_slots;
pub mod schedule_display;

use std::collections::BTreeMap;

use acom_catalog::service::for_merchandising::ProductCard;
use acom_contract::{Check, ErrorClass, ErrorCode, ToolError};
use acom_customer::service::for_merchandising as customers;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::banner::BannerChannel;
use crate::model::{Busy, DisplayRules, display_status, free_spans};
use crate::repo::BookingRow;
use crate::{audience, known_tier};

pub const PAGE_NOT_FOUND: ErrorCode = ErrorCode::new("page_not_found", ErrorClass::NotFound);
pub const SLOT_NOT_FOUND: ErrorCode = ErrorCode::new("slot_not_found", ErrorClass::NotFound);
/// Two items of an arrangement in the same place.
pub const DUPLICATE_POSITION: ErrorCode =
    ErrorCode::new("duplicate_position", ErrorClass::AlreadyExists);
/// The slot is another kind than the caller said, or cannot hold that kind of item.
pub const SLOT_TYPE_MISMATCH: ErrorCode = ErrorCode::new("slot_type_mismatch", ErrorClass::Domain);
/// A product in the slot was discontinued (`display.exclude_discontinued`).
pub const ITEM_DISCONTINUED: ErrorCode =
    ErrorCode::new("item_discontinued", ErrorClass::PolicyViolation);
/// The slot has no items to book.
pub const EMPTY_SLOT: ErrorCode = ErrorCode::new("empty_slot", ErrorClass::Domain);

/// One item of a slot, in its place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotItem {
    /// Place in the slot, 1 first.
    #[schemars(range(min = 1, max = 1000), example = 1)]
    pub position: i32,
    /// What goes there: a product (`prd_`), a banner (`bnr_`), a curated page (`col_`), or a
    /// brand (`brn_`) in a brand strip. Which kinds a slot takes depends on its slot_type.
    #[schemars(regex(pattern = r"^(prd|bnr|col|brn)_[a-z0-9_]+$"), example = &"prd_wool_overcoat")]
    pub item_id: String,
}

/// A slot's stored items (`[{position, item_id}]`), in order; entries that do not read as items
/// are left out.
pub(crate) fn slot_items(v: &Value) -> Vec<SlotItem> {
    let mut items: Vec<SlotItem> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| serde_json::from_value(i.clone()).ok())
        .collect();
    items.sort_by_key(|i| i.position);
    items
}

/// The products among the items.
pub(crate) fn product_ids(items: &[SlotItem]) -> Vec<String> {
    items
        .iter()
        .filter(|i| i.item_id.starts_with("prd_"))
        .map(|i| i.item_id.clone())
        .collect()
}

/// Products the catalog has discontinued, as `Name (prd_…)`.
pub(crate) fn discontinued(cards: &[ProductCard]) -> Vec<String> {
    cards
        .iter()
        .filter(|c| c.approval_status == "discontinued")
        .map(|c| format!("{} ({})", c.name, c.product_id))
        .collect()
}

/// `display.exclude_discontinued` over the products a slot or a curated page would show.
pub(crate) fn discontinued_check(rules: &DisplayRules, cards: &[ProductCard]) -> Check {
    let gone = discontinued(cards);
    let passed = !rules.exclude_discontinued || gone.is_empty();
    Check {
        rule: "display.exclude_discontinued".into(),
        passed,
        detail: Some(if gone.is_empty() {
            "No discontinued product".into()
        } else {
            format!("Discontinued: {}", gone.join(", "))
        }),
        fix_tool: (!passed).then(|| "search_products".into()),
    }
}

/// The channel words of a booking or page; every channel shoppers use when none is stored.
pub(crate) fn channel_words(stored: Option<&[String]>) -> Vec<String> {
    match stored {
        Some(c) => c.to_vec(),
        None => [
            BannerChannel::Web,
            BannerChannel::MobileWeb,
            BannerChannel::App,
        ]
        .iter()
        .map(|c| c.word().to_owned())
        .collect(),
    }
}

/// A store page that has display slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StorePage {
    /// The main page.
    Main,
    /// A category's product list.
    Category,
    /// A product's detail page.
    Product,
    /// Search results.
    Search,
}

impl StorePage {
    pub fn word(self) -> &'static str {
        match self {
            StorePage::Main => "main",
            StorePage::Category => "category",
            StorePage::Product => "product",
            StorePage::Search => "search",
        }
    }
}

/// Whose view a slot read takes: operators see every audience's bookings; a shopper (or guest,
/// or an agent acting for one) sees only what shows to them now, by their own tier.
pub(crate) enum View {
    Operator,
    Shopper(String),
}

pub(crate) async fn view(cx: &Cx, tx: &mut TxCx) -> Result<View, ToolError> {
    if acom_catalog::is_operator(&cx.principal) {
        return Ok(View::Operator);
    }
    let tier = match cx.principal.customer_id() {
        Some(c) => customers::tier_of(tx, c).await?,
        None => None,
    };
    Ok(View::Shopper(tier.unwrap_or_else(|| "everyone".to_owned())))
}

pub(crate) fn operators_only(field: &str) -> ToolError {
    ToolError::new(
        ErrorCode::FORBIDDEN,
        format!("Only store operators can set `{field}`."),
    )
    .detail(json!({"reason": "scope", "field": field}))
}

/// The audiences of a slot's bookings: `everyone` and the store's tiers (highest first), or
/// just the one asked for.
async fn audiences(tx: &mut TxCx, only: Option<&str>) -> Result<Vec<String>, ToolError> {
    match only {
        Some(a) => {
            known_tier(tx, a).await?;
            Ok(vec![a.to_owned()])
        }
        None => {
            let mut out = vec!["everyone".to_owned()];
            out.extend(customers::tiers(tx).await?.into_iter().map(|t| t.id));
            Ok(out)
        }
    }
}

/// Free time per audience in `from`–`through`, given the slot's bookings.
pub(crate) fn availability(
    rules: &DisplayRules,
    audiences: &[String],
    bookings: &[BookingRow],
    from: DateTime<Utc>,
    through: DateTime<Utc>,
    time: &acom_core::StoreTime,
) -> Vec<Value> {
    let mut out = Vec::new();
    for a in audiences {
        let busy: Vec<Busy> = bookings
            .iter()
            .filter(|b| rules.blocks(a, audience(b.valid_for_member_tier.as_deref())))
            .map(|b| Busy {
                from: b.valid_from,
                through: b.valid_through,
            })
            .collect();
        for (f, t) in free_spans(from, through, &busy) {
            out.push(
                json!({"audience": a, "free_from": time.format(f), "free_until": time.format(t)}),
            );
        }
    }
    out
}

pub(crate) fn booking_summary(cx: &Cx, b: &BookingRow, at: DateTime<Utc>) -> Value {
    let mut out = json!({
        "display_booking_id": b.id,
        "audience": audience(b.valid_for_member_tier.as_deref()),
        "headline": b.headline,
        "valid_from": cx.time.format(b.valid_from),
        "valid_through": b.valid_through.map(|t| cx.time.format(t)),
        "status": display_status(&b.display_status, b.valid_from, b.valid_through, at),
        "promotion_id": b.promotion_id,
        "change_set_id": b.change_set_id,
        "channels": channel_words(b.sales_channel.as_deref()),
    });
    // A booking of the slot's own items (`schedule_display`) shows those items, not a banner.
    if let Some(items) = &b.item_list_element {
        out["item_list_element"] = items.clone();
    }
    out
}

/// The banner of a booking as a page shows it; a product image comes from the catalog.
pub(crate) fn banner_json(b: &BookingRow, cards: &BTreeMap<String, ProductCard>) -> Value {
    let Some(id) = &b.banner_id else {
        return Value::Null;
    };
    let product_image = b
        .image_product_group_id
        .as_ref()
        .and_then(|p| cards.get(p))
        .and_then(|c| c.image.clone());
    json!({
        "id": id,
        "eyebrow": b.eyebrow,
        "headline": b.headline,
        "alternative_headline": b.alternative_headline,
        "cta_label": b.cta_label,
        "url": b.url,
        "content_url": b.image.clone().or(product_image),
        "image_product_id": b.image_product_group_id,
        "image_color": b.image_color,
    })
}

/// Why a product in a slot does not show now, if it does not.
pub(crate) fn hidden_reason(
    rules: &DisplayRules,
    card: Option<&ProductCard>,
) -> Option<&'static str> {
    let Some(c) = card else {
        return Some("not_found");
    };
    let wants = |w: &str| rules.visible_when.iter().any(|v| v == w);
    if wants("product_sellable") && c.approval_status != "active" {
        Some("not_sellable")
    } else if wants("in_stock") && c.inventory_level <= 0 {
        Some("out_of_stock")
    } else {
        None
    }
}

/// Which live booking each audience sees: its own, else the one for everyone.
pub(crate) fn showing(audiences: &[String], live: &[&BookingRow]) -> Vec<Value> {
    let for_audience = |a: &str| {
        live.iter()
            .find(|b| audience(b.valid_for_member_tier.as_deref()) == a)
            .map(|b| b.id.clone())
    };
    let everyone = for_audience("everyone");
    audiences
        .iter()
        .filter_map(|a| {
            let id = for_audience(a).or_else(|| everyone.clone())?;
            Some(json!({"audience": a, "display_booking_id": id}))
        })
        .collect()
}
