//! Contract tests without a database: every operation's metadata and input schema, the spec's
//! example inputs, plans that carry `$item[n]` placeholders, content rules, read-back
//! comparisons and visibility.

use std::collections::BTreeMap;

use acom_catalog::service::for_merchandising::ProductCard;
use acom_contract::{Kind, Operation, RegistryBuilder};
use acom_store::Cx;
use serde_json::{Map, Value, json};

use crate::banner::{
    BannerRun, ListBannersInput, PlannedBanner, ScheduleBannerInput, SetBannerInput,
    compare_booking,
};
use crate::category::{GetCategoryInput, ListCategoriesInput};
use crate::curation::{GetCurationInput, ListCurationsInput};
use crate::display_slot::{GetDisplaySlotInput, ListDisplaySlotsInput, hidden_reason};
use crate::model::DisplayRules;
use crate::testing::{at, bookings_of, policies};

fn parses<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap_or_else(|e| panic!("{e}"))
}

fn refuses<T: serde::de::DeserializeOwned + std::fmt::Debug>(v: Value) {
    assert!(serde_json::from_value::<T>(v).is_err());
}

fn b1_banner() -> Value {
    json!({"slot_id": "slot_home_hero", "audience": "vip",
           "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00",
           "content": {"eyebrow": "VIP WEEKEND", "headline": "20% off summer outerwear",
                       "cta_label": "Shop the sale", "url": "/sale/vip-weekend",
                       "image_source": "product", "image_product_id": "prd_field_jacket",
                       "image_color": "Sand"},
           "promotion_id": "$item[1].promotion_id"})
}

#[test]
fn registers_every_operation_with_closed_schemas() {
    let mut b = RegistryBuilder::<Cx>::default();
    crate::register(&mut b);
    let registry = b.build().unwrap_or_else(|e| panic!("{e:#?}"));
    for t in [
        "list_categories",
        "get_category",
        "list_display_slots",
        "get_display_slot",
        "schedule_banner",
        "list_banners",
        "set_banner",
        "list_curations",
        "get_curation",
    ] {
        assert!(registry.get(t).is_some(), "{t} is not registered");
    }
    let content = &registry.get("schedule_banner").unwrap().input_schema()["properties"]["content"];
    assert_eq!(content["additionalProperties"], false);
    assert_eq!(
        content["properties"]["image_source"]["enum"],
        json!(["product", "image"])
    );
    assert_eq!(
        crate::banner::ScheduleBanner::META.kind,
        Kind::WriteApproval
    );
    assert_eq!(crate::banner::SetBanner::META.kind, Kind::Write);
}

#[test]
fn reads_take_the_spec_inputs() {
    let _: ListCategoriesInput =
        parses(json!({"term_set": "standard", "parent_id": "cat_outerwear", "depth": 1}));
    refuses::<ListCategoriesInput>(json!({"term_set": "menu"}));
    let _: GetCategoryInput = parses(json!({"category_id": "cat_summer_outerwear"}));
    let _: ListDisplaySlotsInput = parses(
        json!({"page": "main", "valid_from": "2026-11-07T00:00:00-05:00",
        "valid_through": "2026-11-08T23:59:00-05:00", "audience": "vip", "slot_type": "hero"}),
    );
    refuses::<ListDisplaySlotsInput>(
        json!({"page": "home", "valid_from": "x", "valid_through": "y"}),
    );
    let _: GetDisplaySlotInput =
        parses(json!({"slot_id": "slot_home_hero", "as_of": "2026-11-07T10:00"}));
    let _: ListBannersInput = parses(json!({"page": "main", "channel": "web"}));
    let _: ListCurationsInput = parses(json!({"status": "draft", "audience": "everyone"}));
    refuses::<ListCurationsInput>(json!({"status": "published"}));
    let _: GetCurationInput = parses(json!({"collection_page_id": "col_winter_edit"}));
}

#[test]
fn writes_take_the_spec_inputs() {
    let b: ScheduleBannerInput = parses(b1_banner());
    assert_eq!(b.content.image_color.as_deref(), Some("Sand"));
    let mut extra = b1_banner();
    extra["content"]["subtitle"] = json!("x");
    refuses::<ScheduleBannerInput>(extra);
    let _: SetBannerInput = parses(json!({"banner_id": "bnr_vip_weekend",
        "content_url": "https://cdn.northwind.example/b/vip-weekend.webp",
        "url": "/sale/vip-weekend", "channels": ["web", "app"], "priority": 1}));
    refuses::<SetBannerInput>(
        json!({"banner_id": "bnr_x", "content_url": "https://x/y.png",
        "channels": ["tv"]}),
    );
}

#[test]
fn ids_accept_placeholders_only_where_asked() {
    assert!(
        crate::check_id_or_placeholder("promotion_id", "promo", "$item[1].promotion_id").unwrap()
    );
    assert!(!crate::check_id_or_placeholder("promotion_id", "promo", "promo_1042").unwrap());
    assert!(crate::check_id("slot_id", "slot", "$item[1].slot_id").is_err());
}

fn run() -> BannerRun {
    BannerRun {
        booking_id: "dsp_x".into(),
        slot_id: "slot_home_hero".into(),
        valid_for_member_tier: Some("vip".into()),
        valid_from: at("2026-11-07T00:00"),
        valid_through: at("2026-11-08T23:59"),
        promotion_id: Some("$item[1].promotion_id".into()),
        banner: PlannedBanner {
            id: "bnr_x".into(),
            eyebrow: Some("VIP WEEKEND".into()),
            headline: "20% off summer outerwear".into(),
            cta_label: "Shop the sale".into(),
            url: "/sale/vip-weekend".into(),
            image: None,
            image_product_id: Some("prd_field_jacket".into()),
            image_color: Some("Sand".into()),
        },
    }
}

#[test]
fn a_banner_plan_binds_the_promotion_and_reads_back() {
    let plan = serde_json::to_value(run()).unwrap();
    let produced = BTreeMap::from([(
        1,
        Map::from_iter([("promotion_id".to_owned(), json!("promo_01jc8x"))]),
    )]);
    let bound: BannerRun = parses(acom_changeset::bind::bind(&plan, &produced).unwrap());
    assert_eq!(bound.promotion_id.as_deref(), Some("promo_01jc8x"));

    let fall = bookings_of("slot_home_hero")
        .into_iter()
        .find(|b| b.id == "dsp_fall_layers")
        .unwrap();
    let stored = crate::repo::BookingRow {
        id: "dsp_x".into(),
        banner_id: Some("bnr_x".into()),
        valid_for_member_tier: Some("vip".into()),
        valid_from: bound.valid_from,
        valid_through: Some(bound.valid_through),
        display_status: "scheduled".into(),
        promotion_id: Some("promo_01jc8x".into()),
        headline: Some("20% off summer outerwear".into()),
        url: Some("/sale/vip-weekend".into()),
        ..fall
    };
    assert!(compare_booking(&bound, Some(&stored)).matched);
    assert!(!compare_booking(&bound, None).matched);
    let moved = crate::repo::BookingRow {
        valid_for_member_tier: None,
        ..stored
    };
    assert!(!compare_booking(&bound, Some(&moved)).matched);
}

#[test]
fn banner_text_follows_the_policy_limits() {
    let rules = DisplayRules::from_policy(&policies(true)).unwrap();
    let ok: ScheduleBannerInput = parses(b1_banner());
    assert!(crate::banner::check_content(&rules, &ok.content).is_ok());
    let mut long = ok.content.clone();
    long.headline = "x".repeat(rules.headline_max + 1);
    let e = crate::banner::check_content(&rules, &long).unwrap_err();
    assert_eq!(e.detail.unwrap()["field"], "content.headline");
    let mut no_product = ok.content.clone();
    no_product.image_product_id = None;
    let e = crate::banner::check_content(&rules, &no_product).unwrap_err();
    assert_eq!(e.detail.unwrap()["field"], "content.image_product_id");
    let mut outside = ok.content;
    outside.url = "https://elsewhere.example".into();
    assert!(crate::banner::check_content(&rules, &outside).is_err());
}

#[test]
fn sold_out_and_unsellable_products_are_hidden() {
    let rules = DisplayRules::from_policy(&policies(true)).unwrap();
    let card = |status: &str, stock: i64| ProductCard {
        product_id: "prd_x".into(),
        name: "X".into(),
        approval_status: status.into(),
        image: None,
        colors: vec![],
        inventory_level: stock,
    };
    assert_eq!(
        hidden_reason(&rules, Some(&card("active", 0))),
        Some("out_of_stock")
    );
    assert_eq!(
        hidden_reason(&rules, Some(&card("discontinued", 9))),
        Some("not_sellable")
    );
    assert_eq!(hidden_reason(&rules, Some(&card("active", 9))), None);
    assert_eq!(hidden_reason(&rules, None), Some("not_found"));
}
