//! `list_display_slots` (docs/api/merchandising.md#display_slot).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::SlotType;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;
use crate::model::DisplayRules;
use crate::repo::{self, BookingRow};
use crate::{INVALID_PERIOD, TIER_NOT_FOUND, parse_time};

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
        summary: "The slots of one page with their bookings in a period and the free time per audience (everyone and each tier). Use before schedule_banner to see whether a slot is open for, say, VIP on the weekend. A shopper gets only the page's slots, in order, without bookings or free time.",
        consumes: &[],
        produces: &[Slot::one("slot_id")],
        checks: &[],
        errors: &[PAGE_NOT_FOUND, INVALID_PERIOD, TIER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListDisplaySlotsInput;
    type Output = Value;
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
        let shopper = matches!(view(cx, &mut tx).await?, View::Shopper(_));
        if shopper && input.audience.is_some() {
            return Err(operators_only("audience"));
        }
        let audiences = if shopper {
            Vec::new()
        } else {
            audiences(&mut tx, input.audience.as_deref()).await?
        };
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
        let bookings = if shopper {
            Vec::new()
        } else {
            repo::bookings_between(&mut tx, &ids, from, through).await?
        };
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = slots
            .iter()
            .map(|s| {
                if shopper {
                    // What shows to the shopper is `get_display_slot`'s answer, slot by slot.
                    return json!({"id": s.id, "page": s.page, "name": s.name,
                                  "slot_type": s.slot_type, "position": s.position});
                }
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
