//! `get_display_slot` (docs/api/merchandising.md#display_slot).

use std::collections::BTreeMap;

use acom_catalog::service::for_merchandising::{ProductCard, product_cards};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::DisplayStatus;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;
use crate::model::{DisplayRules, display_status};
use crate::repo::{self, BookingRow, SlotRow};
use crate::{check_id, parse_time, pick};

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

impl ReadOp<Cx> for GetDisplaySlot {
    async fn read(&self, cx: &Cx, input: GetDisplaySlotInput) -> Result<Value, ToolError> {
        check_id("slot_id", "slot", &input.slot_id)?;
        let rules = DisplayRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let view = view(cx, &mut tx).await?;
        let at = match (&input.as_of, &view) {
            (Some(_), View::Shopper(_)) => return Err(operators_only("as_of")),
            (Some(s), View::Operator) => parse_time(&cx.time, "as_of", s)?,
            (None, _) => cx.clock.now(),
        };
        let slot: SlotRow = repo::slot(&mut tx, &input.slot_id).await?.ok_or_else(|| {
            ToolError::new(
                SLOT_NOT_FOUND,
                format!("There is no slot {}.", input.slot_id),
            )
            .detail(json!({"entity": "display_slot", "id": input.slot_id}))
            .next("list_display_slots")
        })?;
        let bookings = repo::bookings_from(&mut tx, &slot.id, at).await?;
        let audiences = match &view {
            View::Operator => audiences(&mut tx, None).await?,
            View::Shopper(a) => vec![a.clone()],
        };
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
        // Bookings of the slot's items (`schedule_display`) show their own copy of the items.
        let booked_products = |b: &BookingRow| -> Vec<String> {
            b.item_list_element
                .as_ref()
                .map(|v| product_ids(&slot_items(v)))
                .unwrap_or_default()
        };
        wanted.extend(bookings.iter().flat_map(booked_products));
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
        let showing = showing(&audiences, &live);
        // A shopper sees the one booking that shows to them now, never what is coming or what
        // other tiers see.
        let shown: Vec<&BookingRow> = match &view {
            View::Operator => bookings.iter().collect(),
            View::Shopper(_) => bookings
                .iter()
                .filter(|b| {
                    showing
                        .iter()
                        .any(|s| s["display_booking_id"].as_str() == Some(b.id.as_str()))
                })
                .collect(),
        };
        let out_bookings: Vec<Value> = shown
            .iter()
            .map(|b| {
                let mut v = booking_summary(cx, b, at);
                v["banner"] = banner_json(b, &cards);
                if b.item_list_element.is_some() {
                    v["hidden_items"] = json!(
                        booked_products(b)
                            .iter()
                            .filter_map(|p| hidden_reason(&rules, cards.get(p))
                                .map(|r| json!({"product_id": p, "reason": r})))
                            .collect::<Vec<_>>()
                    );
                }
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
            "showing": showing,
            "live_now": live_now,
            "hidden_items": hidden,
            "as_of": cx.time.format(at),
        });
        Ok(pick(out, input.fields.as_deref()))
    }
}
