//! `schedule_display` (docs/api/merchandising.md#display_slot).
//!
//! Books the slot's current items (as `arrange_display_slot` left them) for one audience, a
//! period and the sales channels. The booking keeps a copy of the items, so arranging the slot
//! again later does not change what was approved. It reaches shoppers, so it always needs a
//! person's approval; whether it shows is computed from its period when a page asks.

use std::collections::BTreeMap;

use acom_catalog::service::for_merchandising::{ProductCard, product_cards};
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::DisplayStatus;
use acom_core::ids::BookingId;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    EMPTY_SLOT, ITEM_DISCONTINUED, SLOT_NOT_FOUND, SlotItem, discontinued, discontinued_check,
    hidden_reason, product_ids, slot_items,
};
use crate::banner::{
    BannerChannel, SLOT_TAKEN, VALID_FROM_IN_PAST, banner_reason, channel_input, check_slot_free,
};
use crate::model::{DisplayRules, period_words};
use crate::repo::{self, BookingRow};
use crate::{INVALID_PERIOD, TIER_NOT_FOUND, audience, check_id, known_tier, parse_time};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDisplayInput {
    /// The slot whose items to show (`list_display_slots`); set the items first with
    /// `arrange_display_slot`.
    #[schemars(regex(pattern = r"^slot_[a-z0-9_]+$"), example = &"slot_home_new_in")]
    pub slot_id: String,
    /// Who sees it: a tier id (`list_tiers`) or `everyone`. Other audiences keep seeing what
    /// they see now.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"everyone")]
    pub audience: String,
    /// Start (inclusive), not in the past. Without an offset it is read in store time.
    #[schemars(example = &"2026-11-13T00:00")]
    pub valid_from: String,
    /// End (inclusive, minute precision), after `valid_from`.
    #[schemars(example = &"2026-11-20T23:59")]
    pub valid_through: String,
    /// Sales channels it shows on: web, mobile_web, app. Default: all three.
    #[schemars(length(min = 1, max = 3), example = ["web", "app"])]
    pub channels: Option<Vec<BannerChannel>>,
}

/// What a run of `schedule_display` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayRun {
    pub booking_id: String,
    pub slot_id: String,
    /// `None` for everyone.
    pub valid_for_member_tier: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
    /// `None` for every channel.
    pub channels: Option<Vec<String>>,
    /// The slot's items as booked, in place order.
    pub items: Vec<SlotItem>,
}

/// Book a slot's items for an audience, a period and channels.
#[derive(Debug)]
pub struct ScheduleDisplay;

impl Operation for ScheduleDisplay {
    const META: OpMeta = OpMeta {
        address: "merchandising.display_slot.schedule",
        tool: "schedule_display",
        group: Group::Offer,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/merchandising/display-slots/{slot_id}/bookings",
        summary: "Book a slot's current items (arranged with arrange_display_slot) for one audience, period and sales channels. Other audiences keep what they see now. Reaches shoppers, so it always needs a person's approval. For a single banner use schedule_banner.",
        consumes: &[Slot::one("slot_id")],
        produces: &[Slot::one("display_booking_id")],
        checks: &[
            "slot_free",
            "display.start_not_in_past",
            "display.exclude_discontinued",
        ],
        errors: &[
            SLOT_TAKEN,
            SLOT_NOT_FOUND,
            VALID_FROM_IN_PAST,
            INVALID_PERIOD,
            EMPTY_SLOT,
            ITEM_DISCONTINUED,
            TIER_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::SCHEDULE,
        person_scope: "",
    };
    type Input = ScheduleDisplayInput;
    type Output = Value;
}

fn slot_not_found(id: &str) -> ToolError {
    ToolError::new(SLOT_NOT_FOUND, format!("There is no slot {id}."))
        .detail(json!({"entity": "display_slot", "id": id}))
        .next("list_display_slots")
}

impl WriteOp<Cx> for ScheduleDisplay {
    type Plan = DisplayRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ScheduleDisplayInput,
    ) -> Result<Planned<DisplayRun>, ToolError> {
        let rules = DisplayRules::from_policy(&cx.policy)?;
        check_id("slot_id", "slot", &input.slot_id)?;
        let from = parse_time(&cx.time, "valid_from", &input.valid_from)?;
        let through = parse_time(&cx.time, "valid_through", &input.valid_through)?;
        if through <= from {
            return Err(ToolError::new(
                INVALID_PERIOD,
                "`valid_through` must come after `valid_from`.",
            )
            .detail(json!({"field": "valid_through", "reason": "range"}))
            .next("schedule_display"));
        }
        let now = cx.clock.now();
        if rules.start_not_in_past && from < now {
            return Err(ToolError::new(
                VALID_FROM_IN_PAST,
                format!(
                    "The booking would start in the past ({}).",
                    cx.time.format(from)
                ),
            )
            .detail(json!({"field": "valid_from", "reason": "past"}))
            .next("schedule_display"));
        }
        let channels = channel_input(input.channels.as_deref());

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let slot = repo::slot(&mut tx, &input.slot_id)
            .await?
            .ok_or_else(|| slot_not_found(&input.slot_id))?;
        let tier = known_tier(&mut tx, &input.audience).await?;
        let items = slot_items(&slot.item_list_element);
        if items.is_empty() {
            return Err(ToolError::new(
                EMPTY_SLOT,
                format!("{} holds no items to show; arrange it first.", slot.name),
            )
            .detail(json!({"entity": "display_slot", "id": slot.id}))
            .next("arrange_display_slot"));
        }
        let cards: Vec<ProductCard> = product_cards(&mut tx, &product_ids(&items)).await?;
        let others =
            repo::bookings_between(&mut tx, std::slice::from_ref(&slot.id), from, through).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let audience_name = tier
            .as_ref()
            .map_or("everyone".to_owned(), |t| t.name.clone());
        let period = period_words(from, Some(through), cx.time.zone);
        let by_id: BTreeMap<&str, &ProductCard> =
            cards.iter().map(|c| (c.product_id.as_str(), c)).collect();
        let hidden: Vec<String> = product_ids(&items)
            .into_iter()
            .filter(|p| hidden_reason(&rules, by_id.get(p.as_str()).copied()).is_some())
            .collect();
        let mut stock_check = discontinued_check(&rules, &cards);
        if stock_check.passed && !hidden.is_empty() {
            stock_check.detail = Some(format!(
                "No discontinued product; {} sold out or not for sale stay hidden until they can be bought",
                hidden.len()
            ));
        }
        let checks = vec![
            check_slot_free(
                &rules,
                &slot.slot_type,
                &input.audience,
                &audience_name,
                &period,
                &others,
            ),
            Check {
                rule: "display.start_not_in_past".into(),
                passed: true,
                detail: Some(format!("Starts {}, after now", cx.time.format(from))),
                fix_tool: None,
            },
            stock_check,
        ];
        let change = Change {
            op: ChangeOp::Schedule,
            entity: "display_booking".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({
                "slot_id": slot.id,
                "audience": input.audience,
                "valid_from": cx.time.format(from),
                "valid_through": cx.time.format(through),
                "channels": super::channel_words(channels.as_deref()),
                "item_list_element": items,
                "hidden_items": hidden.len(),
            }),
            note: None,
        };
        let basis = others
            .iter()
            .map(|b| Basis {
                entity: "merchandising.display_booking".into(),
                id: b.id.clone(),
                version: i64::from(b.version),
            })
            .chain([Basis {
                entity: "merchandising.display_slot".into(),
                id: slot.id.clone(),
                version: i64::from(slot.version),
            }])
            .collect();
        Ok(Planned {
            plan: DisplayRun {
                booking_id: BookingId::generate().to_string(),
                slot_id: slot.id.clone(),
                valid_for_member_tier: tier.as_ref().map(|t| t.id.clone()),
                valid_from: from,
                valid_through: through,
                channels,
                items,
            },
            changes: vec![change],
            checks,
            basis,
            approval: ApprovalNeed::Needed {
                reason: banner_reason(&slot.page, tier.as_ref()),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &DisplayRun) -> Result<Committed, ToolError> {
        let rules = DisplayRules::from_policy(&tx.policy)?;
        let audience_word = audience(run.valid_for_member_tier.as_deref()).to_owned();
        // Someone may have booked the slot since the preview.
        let others: Vec<BookingRow> = repo::bookings_between(
            tx,
            std::slice::from_ref(&run.slot_id),
            run.valid_from,
            run.valid_through,
        )
        .await?
        .into_iter()
        .filter(|b| b.id != run.booking_id)
        .filter(|b| rules.blocks(&audience_word, audience(b.valid_for_member_tier.as_deref())))
        .collect();
        if let Some(first) = others.first() {
            let ids: Vec<&String> = others.iter().map(|b| &b.id).collect();
            return Err(ToolError::new(
                SLOT_TAKEN,
                format!("{} now holds the slot for {audience_word}.", first.id),
            )
            .detail(json!({"rule": "slot_free", "items": ids}))
            .next("list_display_slots"));
        }
        // A product may have been discontinued since the preview.
        let gone = discontinued(&product_cards(tx, &product_ids(&run.items)).await?);
        if rules.exclude_discontinued && !gone.is_empty() {
            return Err(ToolError::new(
                ITEM_DISCONTINUED,
                format!(
                    "{} can no longer be displayed: discontinued.",
                    gone.join(", ")
                ),
            )
            .detail(json!({"rule": "display.exclude_discontinued", "items": gone}))
            .next("arrange_display_slot"));
        }
        let status = if tx.clock.now() < run.valid_from {
            DisplayStatus::Scheduled
        } else {
            DisplayStatus::Active
        };
        let items = json!(run.items);
        repo::insert_booking(
            tx,
            &repo::NewBooking {
                id: &run.booking_id,
                slot_id: &run.slot_id,
                banner_id: None,
                tier: run.valid_for_member_tier.as_deref(),
                valid_from: run.valid_from,
                valid_through: run.valid_through,
                status: status.as_str(),
                promotion_id: None,
                item_list_element: Some(&items),
                sales_channel: run.channels.as_deref(),
            },
        )
        .await?;
        Ok(Committed {
            id: run.booking_id.clone(),
            produced: Map::from_iter([("display_booking_id".to_owned(), json!(run.booking_id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &DisplayRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let stored = repo::booking(&mut tx, &run.booking_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(compare(run, stored.as_ref()))
    }

    async fn undo(&self, tx: &mut TxCx, run: &DisplayRun, _: &Committed) -> Result<(), ToolError> {
        let Some(b) = repo::booking(tx, &run.booking_id).await? else {
            return Ok(());
        };
        let now = tx.clock.now();
        if now < b.valid_from {
            // Never shown: cancel it outright.
            repo::delete_booking(tx, &b.id).await?;
        } else {
            repo::end_booking(tx, &b.id, now).await?;
        }
        Ok(())
    }
}

/// Read-back check of a booked display.
fn compare(run: &DisplayRun, stored: Option<&BookingRow>) -> Verification {
    let mut mismatches = Vec::new();
    match stored {
        None => mismatches.push(format!("booking {} is missing", run.booking_id)),
        Some(b) => {
            if b.display_slot_id != run.slot_id
                || b.valid_for_member_tier != run.valid_for_member_tier
                || b.valid_from != run.valid_from
                || b.valid_through != Some(run.valid_through)
                || b.sales_channel != run.channels
            {
                mismatches.push(format!(
                    "booking {} differs from the plan (slot, audience, period or channels)",
                    run.booking_id
                ));
            }
            if b.banner_id.is_some()
                || b.item_list_element.as_ref().map(slot_items) != Some(run.items.clone())
            {
                mismatches.push(format!(
                    "booking {} does not show the planned items",
                    run.booking_id
                ));
            }
        }
    }
    Verification {
        matched: mismatches.is_empty(),
        mismatches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{at, bookings_of};

    fn run() -> DisplayRun {
        DisplayRun {
            booking_id: "dsp_x".into(),
            slot_id: "slot_home_new_in".into(),
            valid_for_member_tier: None,
            valid_from: at("2026-11-13T00:00"),
            valid_through: at("2026-11-20T23:59"),
            channels: Some(vec!["web".into(), "app".into()]),
            items: vec![SlotItem {
                position: 1,
                item_id: "prd_denim_jacket".into(),
            }],
        }
    }

    #[test]
    fn takes_the_spec_input_with_a_closed_schema() {
        let i: ScheduleDisplayInput = serde_json::from_value(json!({
            "slot_id": "slot_home_hero", "audience": "everyone",
            "valid_from": "2026-11-13T00:00:00-05:00", "valid_through": "2026-11-20T23:59:00-05:00",
            "channels": ["web", "mobile_web", "app"]}))
        .unwrap();
        assert_eq!(
            channel_input(i.channels.as_deref()),
            Some(vec!["web".into(), "mobile_web".into(), "app".into()])
        );
        assert!(
            serde_json::from_value::<ScheduleDisplayInput>(json!({
                "slot_id": "slot_home_hero", "audience": "everyone",
                "valid_from": "x", "valid_through": "y", "channels": ["agent"]}))
            .is_err()
        );
        let schema = acom_contract::schema::closed_schema_for::<ScheduleDisplayInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(ScheduleDisplay::META.kind, Kind::WriteApproval);
    }

    #[test]
    fn a_booked_display_reads_back() {
        let r = run();
        let fall = bookings_of("slot_home_hero")
            .into_iter()
            .find(|b| b.id == "dsp_fall_layers")
            .unwrap();
        let stored = BookingRow {
            id: "dsp_x".into(),
            display_slot_id: r.slot_id.clone(),
            banner_id: None,
            valid_for_member_tier: None,
            valid_from: r.valid_from,
            valid_through: Some(r.valid_through),
            display_status: "scheduled".into(),
            item_list_element: Some(json!(r.items)),
            sales_channel: r.channels.clone(),
            ..fall
        };
        assert!(compare(&r, Some(&stored)).matched);
        assert!(!compare(&r, None).matched);
        let other_items = BookingRow {
            item_list_element: Some(json!([])),
            ..stored
        };
        assert!(!compare(&r, Some(&other_items)).matched);
    }
}
