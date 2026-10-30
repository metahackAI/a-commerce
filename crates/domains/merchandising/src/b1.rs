//! Scene B1 of `docs/scenarios.md` §6 against the demo seed and policies, without a database:
//! the main-page hero is free for VIP on Nov 7–8 while everyone keeps "Fall layers", and the
//! banner reaches the 1,284 VIP members.

use acom_customer::service::for_merchandising::Tier;
use serde_json::json;

use crate::banner::{banner_reason, check_slot_free};
use crate::display_slot::{availability, showing};
use crate::model::{DisplayRules, period_words};
use crate::repo::BookingRow;
use crate::testing::{at, bookings_of, member_count, policies, rows, store_time};

fn vip() -> Tier {
    Tier {
        id: "vip".into(),
        name: "VIP".into(),
        rank: 3,
        member_count: member_count("vip"),
    }
}

#[test]
fn hero_is_free_for_every_tier_but_not_for_everyone() {
    let rules = DisplayRules::from_policy(&policies(true)).unwrap();
    let bookings = bookings_of("slot_home_hero");
    let audiences: Vec<String> = ["everyone", "vip", "gold", "member"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let free = availability(
        &rules,
        &audiences,
        &bookings,
        at("2026-11-07T00:00"),
        at("2026-11-08T23:59"),
        &store_time(),
    );
    assert_eq!(
        free,
        vec![
            json!({"audience": "vip", "free_from": "2026-11-07T00:00:00-05:00", "free_until": "2026-11-08T23:59:00-05:00"}),
            json!({"audience": "gold", "free_from": "2026-11-07T00:00:00-05:00", "free_until": "2026-11-08T23:59:00-05:00"}),
            json!({"audience": "member", "free_from": "2026-11-07T00:00:00-05:00", "free_until": "2026-11-08T23:59:00-05:00"}),
        ]
    );
}

#[test]
fn slot_free_passes_for_vip_and_names_the_holder_for_everyone() {
    let rules = DisplayRules::from_policy(&policies(true)).unwrap();
    let (from, through) = (at("2026-11-07T00:00"), at("2026-11-08T23:59"));
    let bookings: Vec<BookingRow> = bookings_of("slot_home_hero")
        .into_iter()
        .filter(|b| b.valid_from <= through && b.valid_through.is_none_or(|t| t >= from))
        .collect();
    let period = period_words(from, Some(through), store_time().zone);
    let vip = check_slot_free(&rules, "hero", "vip", "VIP", &period, &bookings);
    assert!(vip.passed);
    assert_eq!(
        vip.detail.as_deref(),
        Some("Hero slot open Nov 7–8 for VIP")
    );
    let everyone = check_slot_free(&rules, "hero", "everyone", "everyone", &period, &bookings);
    assert!(!everyone.passed);
    assert_eq!(
        everyone.detail.as_deref(),
        Some("dsp_fall_layers holds everyone")
    );
    assert_eq!(everyone.fix_tool.as_deref(), Some("list_display_slots"));
}

#[test]
fn the_banner_reaches_1284_vip_members() {
    assert_eq!(
        banner_reason("main", Some(&vip())),
        "Shows on the main page to 1,284 VIP members"
    );
    assert_eq!(
        banner_reason("main", None),
        "Shows on the main page to everyone"
    );
}

#[test]
fn on_saturday_vip_sees_the_sale_and_everyone_else_fall_layers() {
    let mut bookings = bookings_of("slot_home_hero");
    let fall = bookings
        .iter()
        .find(|b| b.id == "dsp_fall_layers")
        .unwrap()
        .clone();
    bookings.push(BookingRow {
        id: "dsp_1042".into(),
        valid_for_member_tier: Some("vip".into()),
        valid_from: at("2026-11-07T00:00"),
        valid_through: Some(at("2026-11-08T23:59")),
        display_status: "scheduled".into(),
        ..fall
    });
    let sat = at("2026-11-07T10:00");
    let live: Vec<&BookingRow> = bookings
        .iter()
        .filter(|b| {
            crate::model::display_status(&b.display_status, b.valid_from, b.valid_through, sat)
                == acom_core::enums::DisplayStatus::Active
        })
        .collect();
    let audiences: Vec<String> = ["everyone", "vip", "gold"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert_eq!(
        showing(&audiences, &live),
        vec![
            json!({"audience": "everyone", "display_booking_id": "dsp_fall_layers"}),
            json!({"audience": "vip", "display_booking_id": "dsp_1042"}),
            json!({"audience": "gold", "display_booking_id": "dsp_fall_layers"}),
        ]
    );
}

#[test]
fn summer_outerwear_holds_42_products() {
    let placed = rows("merchandising.product_group_category")
        .filter(|r| r["category_id"] == "cat_summer_outerwear")
        .count();
    assert_eq!(placed, 42);
    assert_eq!(
        rows("merchandising.defined_term")
            .filter(|r| r["in_defined_term_set"] == "standard")
            .count(),
        24
    );
}
