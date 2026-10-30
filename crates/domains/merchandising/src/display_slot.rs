//! `list_display_slots`, `get_display_slot` (docs/api/merchandising.md#display_slot).
//!
//! Whether something shows is computed when asked, from the period, the stock and whether the
//! product can be sold (`display.visible_when`); nothing is switched on or off by a batch.

use std::collections::BTreeMap;

use acom_catalog::service::for_merchandising::{ProductCard, product_cards};
use acom_contract::{
    ActionType, Audience, ErrorClass, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp,
    Slot, ToolError,
};
use acom_core::enums::{DisplayStatus, SlotType};
use acom_customer::service::for_merchandising as customers;
use acom_store::{Cx, Page, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{Busy, DisplayRules, display_status, free_spans};
use crate::repo::{self, BookingRow, SlotRow};
use crate::{INVALID_PERIOD, TIER_NOT_FOUND, audience, check_id, known_tier, parse_time, pick};

pub const PAGE_NOT_FOUND: ErrorCode = ErrorCode::new("page_not_found", ErrorClass::NotFound);
pub const SLOT_NOT_FOUND: ErrorCode = ErrorCode::new("slot_not_found", ErrorClass::NotFound);

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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDisplaySlotsInput {
    /// The page whose slots to list.
    #[schemars(example = &"main")]
    pub page: StorePage,
    /// Start of the period to look at (inclusive). Without an offset it is read in store time.
    #[schemars(example = &"2026-11-07T00:00")]
    pub valid_from: String,
    /// End of the period (inclusive, minute precision).
    #[schemars(example = &"2026-11-08T23:59")]
    pub valid_through: String,
    /// Free time for this audience only: a tier id (`list_tiers`) or `everyone`. Default:
    /// everyone and every tier.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub audience: Option<String>,
    /// Only slots of this kind. hero, banner_board, product_grid, brand_strip, collection_feed.
    #[schemars(example = &"hero")]
    pub slot_type: Option<SlotType>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListDisplaySlots;

impl Operation for ListDisplaySlots {
    const META: OpMeta = OpMeta {
        address: "merchandising.display_slot.list",
        tool: "list_display_slots",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/merchandising/display-slots",
        summary: "The slots of one page with their bookings in a period and the free time per audience (everyone and each tier). Use before schedule_banner to see whether a slot is open for, say, VIP on the weekend.",
        consumes: &[],
        produces: &[Slot::one("slot_id")],
        checks: &[],
        errors: &[PAGE_NOT_FOUND, INVALID_PERIOD, TIER_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListDisplaySlotsInput;
    type Output = Value;
}

pub(crate) fn booking_summary(cx: &Cx, b: &BookingRow, at: DateTime<Utc>) -> Value {
    json!({
        "display_booking_id": b.id,
        "audience": audience(b.valid_for_member_tier.as_deref()),
        "headline": b.headline,
        "valid_from": cx.time.format(b.valid_from),
        "valid_through": b.valid_through.map(|t| cx.time.format(t)),
        "status": display_status(&b.display_status, b.valid_from, b.valid_through, at),
        "promotion_id": b.promotion_id,
        "change_set_id": b.change_set_id,
    })
}

impl ReadOp<Cx> for ListDisplaySlots {
    async fn read(&self, cx: &Cx, input: ListDisplaySlotsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let rules = DisplayRules::from_policy(&cx.policy)?;
        let from = parse_time(&cx.time, "valid_from", &input.valid_from)?;
        let through = parse_time(&cx.time, "valid_through", &input.valid_through)?;
        if from > through {
            return Err(
                ToolError::new(INVALID_PERIOD, "`valid_from` is after `valid_through`.")
                    .detail(json!({"field": "valid_from", "reason": "range"}))
                    .next("list_display_slots"),
            );
        }
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let audiences = audiences(&mut tx, input.audience.as_deref()).await?;
        let (total, slots) = repo::slots(
            &mut tx,
            input.page.word(),
            input.slot_type.map(|s| s.as_str()),
            page,
        )
        .await?;
        if total == 0 && input.slot_type.is_none() {
            return Err(ToolError::new(
                PAGE_NOT_FOUND,
                format!("The {} page has no display slots.", input.page.word()),
            )
            .detail(json!({"entity": "page", "id": input.page.word()}))
            .next("list_display_slots"));
        }
        let ids: Vec<String> = slots.iter().map(|s| s.id.clone()).collect();
        let bookings = repo::bookings_between(&mut tx, &ids, from, through).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = slots
            .iter()
            .map(|s| {
                let mine: Vec<BookingRow> = bookings
                    .iter()
                    .filter(|b| b.display_slot_id == s.id)
                    .cloned()
                    .collect();
                json!({
                    "id": s.id, "page": s.page, "name": s.name, "slot_type": s.slot_type,
                    "bookings": mine.iter().map(|b| booking_summary(cx, b, now)).collect::<Vec<_>>(),
                    "availability": availability(&rules, &audiences, &mine, from, through, &cx.time),
                })
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(slots.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetDisplaySlotInput {
    /// The slot.
    #[schemars(regex(pattern = r"^slot_[a-z0-9_]+$"), example = &"slot_home_hero")]
    pub slot_id: String,
    /// The moment to judge what shows (store time without an offset). Default: now.
    #[schemars(example = &"2026-11-07T10:00")]
    pub as_of: Option<String>,
    /// Only these fields (`id` always comes back).
    #[schemars(example = ["bookings", "live_now"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetDisplaySlot;

impl Operation for GetDisplaySlot {
    const META: OpMeta = OpMeta {
        address: "merchandising.display_slot.get",
        tool: "get_display_slot",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/merchandising/display-slots/{slot_id}",
        summary: "One slot with its items, its current and coming bookings (banner content included), which booking each audience sees at a moment, and which products are hidden (sold out or not for sale).",
        consumes: &[Slot::one("slot_id")],
        produces: &[],
        checks: &[],
        errors: &[SLOT_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetDisplaySlotInput;
    type Output = Value;
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

impl ReadOp<Cx> for GetDisplaySlot {
    async fn read(&self, cx: &Cx, input: GetDisplaySlotInput) -> Result<Value, ToolError> {
        check_id("slot_id", "slot", &input.slot_id)?;
        let rules = DisplayRules::from_policy(&cx.policy)?;
        let at = match &input.as_of {
            Some(s) => parse_time(&cx.time, "as_of", s)?,
            None => cx.clock.now(),
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let slot: SlotRow = repo::slot(&mut tx, &input.slot_id).await?.ok_or_else(|| {
            ToolError::new(
                SLOT_NOT_FOUND,
                format!("There is no slot {}.", input.slot_id),
            )
            .detail(json!({"entity": "display_slot", "id": input.slot_id}))
            .next("list_display_slots")
        })?;
        let bookings = repo::bookings_from(&mut tx, &slot.id, at).await?;
        let audiences = audiences(&mut tx, None).await?;
        let item_products: Vec<String> = slot
            .item_list_element
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["item_id"].as_str())
            .filter(|id| id.starts_with("prd_"))
            .map(str::to_owned)
            .collect();
        let mut wanted = item_products.clone();
        wanted.extend(
            bookings
                .iter()
                .filter_map(|b| b.image_product_group_id.clone()),
        );
        let cards: BTreeMap<String, ProductCard> = product_cards(&mut tx, &wanted)
            .await?
            .into_iter()
            .map(|c| (c.product_id.clone(), c))
            .collect();
        tx.commit().await.map_err(ToolError::from)?;

        let hidden: Vec<Value> = item_products
            .iter()
            .filter_map(|p| {
                hidden_reason(&rules, cards.get(p)).map(|r| json!({"product_id": p, "reason": r}))
            })
            .collect();
        let live: Vec<&BookingRow> = bookings
            .iter()
            .filter(|b| {
                display_status(&b.display_status, b.valid_from, b.valid_through, at)
                    == DisplayStatus::Active
            })
            .collect();
        let live_now = !live.is_empty() || item_products.len() > hidden.len();
        let out_bookings: Vec<Value> = bookings
            .iter()
            .map(|b| {
                let mut v = booking_summary(cx, b, at);
                v["banner"] = banner_json(b, &cards);
                v
            })
            .collect();
        let out = json!({
            "id": slot.id,
            "page": slot.page,
            "name": slot.name,
            "slot_type": slot.slot_type,
            "position": slot.position,
            "item_list_element": slot.item_list_element,
            "bookings": out_bookings,
            "showing": showing(&audiences, &live),
            "live_now": live_now,
            "hidden_items": hidden,
            "as_of": cx.time.format(at),
        });
        Ok(pick(out, input.fields.as_deref()))
    }
}
