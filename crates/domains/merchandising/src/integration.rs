//! Integration tests: every operation against the demo store in `DATABASE_URL` (migrated,
//! seeded, demo clock Fri Nov 6 11:00), and scene B1 end to end through the change-set engine
//! (preview v1 → v2, coupon and banner with placeholders, approve, verify, undo). Run on
//! request: `cargo test -p acom-merchandising -- --ignored`. The B1 test writes and then undoes
//! its change set, so run it on a scratch copy of the demo store. The slot, popup, curated page
//! and cache writes are tested in `integration_writes`.

use acom_changeset::NewChangeSet;
use acom_contract::{ApprovalNeed, ReadOp, Registry, RegistryBuilder, WriteOp};
use acom_store::Cx;
use serde_json::{Value, json};

use crate::banner::list_banners::ListBanners;
use crate::banner::schedule_banner::ScheduleBanner;
use crate::banner::set_banner::SetBanner;
use crate::category::get_category::GetCategory;
use crate::category::list_categories::ListCategories;
use crate::collection_page::get_curation::GetCuration;
use crate::collection_page::list_curations::ListCurations;
use crate::display_slot::get_display_slot::GetDisplaySlot;
use crate::display_slot::list_display_slots::ListDisplaySlots;
use crate::testing::{operator_cx, shopper_cx};

fn input<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

fn registry() -> Registry<Cx> {
    let mut b = RegistryBuilder::default();
    acom_promotion::register(&mut b);
    crate::register(&mut b);
    b.build().unwrap()
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn categories_of_the_demo_store() {
    let cx = operator_cx("adm_sj").await;
    let standard = ListCategories
        .read(&cx, input(json!({"term_set": "standard", "limit": 100})))
        .await
        .unwrap();
    assert_eq!(standard["total"], 24);
    let under = ListCategories
        .read(
            &cx,
            input(json!({"parent_id": "cat_outerwear", "depth": 1})),
        )
        .await
        .unwrap();
    assert_eq!(under["items"][0]["id"], "cat_summer_outerwear");
    let one = GetCategory
        .read(&cx, input(json!({"category_id": "cat_summer_outerwear"})))
        .await
        .unwrap();
    assert_eq!(one["product_count"], 42);
    let e = GetCategory
        .read(&cx, input(json!({"category_id": "cat_nothing"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "category_not_found");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn main_page_slots_and_the_hero() {
    let cx = operator_cx("adm_sj").await;
    let slots = ListDisplaySlots
        .read(
            &cx,
            input(json!({"page": "main", "valid_from": "2026-11-07T00:00",
                         "valid_through": "2026-11-08T23:59"})),
        )
        .await
        .unwrap();
    assert_eq!(slots["total"], 4);
    let hero = &slots["items"][0];
    assert_eq!(hero["id"], "slot_home_hero");
    assert_eq!(hero["bookings"][0]["display_booking_id"], "dsp_fall_layers");
    let free: Vec<&str> = hero["availability"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|a| a["audience"].as_str())
        .collect();
    assert_eq!(free, ["vip", "gold", "member"]);
    let one = GetDisplaySlot
        .read(&cx, input(json!({"slot_id": "slot_home_hero"})))
        .await
        .unwrap();
    assert_eq!(
        one["showing"][0],
        json!({"audience": "everyone", "display_booking_id": "dsp_fall_layers"})
    );
    assert_eq!(one["live_now"], true);
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn banners_and_curations() {
    let cx = operator_cx("adm_sj").await;
    let banners = ListBanners
        .read(&cx, input(json!({"page": "main"})))
        .await
        .unwrap();
    assert_eq!(banners["total"], 2);
    let set = SetBanner
        .preview(
            &cx,
            input(json!({"banner_id": "bnr_vip_weekend",
                "content_url": "https://cdn.northwind.example/b/vip-weekend.webp",
                "url": "/sale/vip-weekend", "channels": ["web", "app"]})),
        )
        .await
        .unwrap();
    assert_eq!(set.approval, ApprovalNeed::NotNeeded);
    assert!(set.checks.iter().all(|c| c.passed));
    // Curated pages are stored now (migration 0027): the seed has three.
    let curations = ListCurations.read(&cx, input(json!({}))).await.unwrap();
    assert_eq!(curations["total"], 3);
    let winter = GetCuration
        .read(&cx, input(json!({"collection_page_id": "col_winter_edit"})))
        .await
        .unwrap();
    assert_eq!(winter["status"], "draft");
    let e = GetCuration
        .read(&cx, input(json!({"collection_page_id": "col_nothing"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "collection_page_not_found");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn schedule_banner_previews_the_vip_hero() {
    let cx = operator_cx("adm_sj").await;
    let p = ScheduleBanner
        .preview(
            &cx,
            input(json!({"slot_id": "slot_home_hero", "audience": "vip",
                "valid_from": "2026-11-07T00:00", "valid_through": "2026-11-08T23:59",
                "content": {"eyebrow": "VIP WEEKEND", "headline": "20% off summer outerwear",
                            "cta_label": "Shop the sale", "url": "/sale/vip-weekend",
                            "image_source": "product", "image_product_id": "prd_field_jacket",
                            "image_color": "Sand"},
                "promotion_id": "$item[1].promotion_id"})),
        )
        .await
        .unwrap();
    assert_eq!(
        p.checks[0].detail.as_deref(),
        Some("Hero slot open Nov 7–8 for VIP")
    );
    assert_eq!(
        p.approval,
        ApprovalNeed::Needed {
            reason: "Shows on the main page to 1,284 VIP members".into()
        }
    );
    let taken = ScheduleBanner
        .preview(
            &cx,
            input(json!({"slot_id": "slot_home_hero", "audience": "everyone",
                "valid_from": "2026-11-07T00:00", "valid_through": "2026-11-08T23:59",
                "content": {"headline": "x", "cta_label": "y", "url": "/",
                            "image_source": "image", "image_id": "bnr_fall_layers"}})),
        )
        .await
        .unwrap();
    assert!(!taken.checks[0].passed);
    let past = ScheduleBanner
        .preview(
            &cx,
            input(json!({"slot_id": "slot_home_hero", "audience": "vip",
                "valid_from": "2026-11-01T00:00", "valid_through": "2026-11-08T23:59",
                "content": {"headline": "x", "cta_label": "y", "url": "/",
                            "image_source": "image", "image_id": "bnr_fall_layers"}})),
        )
        .await
        .unwrap_err();
    assert_eq!(past.code, "valid_from_in_past");
}

/// Scene B1: v1 fails two checks, v2 passes; the set runs, verifies, and undoes.
#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn b1_vip_weekend_sale_end_to_end() {
    let registry = registry();
    let sj = operator_cx("adm_sj").await;
    let created = acom_changeset::change_set::create(
        &sj,
        NewChangeSet {
            title: "VIP weekend sale".into(),
            request_text: vec!["Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page.".into()],
            interpretation: vec![],
            conversation_id: None,
            task_id: None,
        },
    )
    .await
    .unwrap();
    let cs = created["id"].as_str().unwrap().to_owned();
    let sale = |v2: bool| {
        let mut i = json!({"name": "VIP weekend", "valid_for_member_tier": "vip",
            "valid_from": "2026-11-07T00:00", "valid_through": "2026-11-08T23:59",
            "scope_mode": "category", "category_id": "cat_summer_outerwear", "percent_off": 20});
        if v2 {
            i["overrides"] = json!([{"product_id": "prd_seersucker_overshirt", "percent_off": 15}]);
            i["exclude_low_stock"] = json!(true);
        }
        i
    };
    let v1 = acom_changeset::preview(&registry, &sj, "set_sale_prices", sale(false), None)
        .await
        .unwrap();
    let failed: Vec<&str> = v1
        .checks
        .iter()
        .filter(|c| !c.passed)
        .map(|c| c.rule.as_str())
        .collect();
    assert_eq!(failed, ["price_floor", "promo_min_stock"]);
    assert_eq!(v1.approval_reason.as_deref(), Some("Changes 42 prices"));

    let v2 = acom_changeset::preview(&registry, &sj, "set_sale_prices", sale(true), Some(&cs))
        .await
        .unwrap();
    assert!(v2.checks.iter().all(|c| c.passed), "{:#?}", v2.checks);
    assert_eq!(v2.approval_reason.as_deref(), Some("Changes 40 prices"));
    let coupon = acom_changeset::preview(
        &registry,
        &sj,
        "create_coupon",
        json!({"code": "VIPWEEKEND20", "promotion_id": "$item[1].promotion_id",
               "valid_for_member_tier": "vip", "application": "automatic_at_checkout",
               "stacking": "no_tier_coupons"}),
        Some(&cs),
    )
    .await
    .unwrap();
    assert!(coupon.checks.iter().all(|c| c.passed));
    let banner = acom_changeset::preview(
        &registry,
        &sj,
        "schedule_banner",
        json!({"slot_id": "slot_home_hero", "audience": "vip",
               "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00",
               "content": {"eyebrow": "VIP WEEKEND", "headline": "20% off summer outerwear",
                           "cta_label": "Shop the sale", "url": "/sale/vip-weekend",
                           "image_source": "product", "image_product_id": "prd_field_jacket",
                           "image_color": "Sand"},
               "promotion_id": "$item[1].promotion_id"}),
        Some(&cs),
    )
    .await
    .unwrap();
    assert!(banner.checks.iter().all(|c| c.passed));

    let head = acom_changeset::read::get(&sj, &cs, &[]).await.unwrap();
    let version = head["version"].as_i64().unwrap() as i32;
    acom_changeset::change_set::submit(&sj, &cs, version)
        .await
        .unwrap();
    let done = acom_changeset::change_set::approve(&registry, &sj, &cs, version)
        .await
        .unwrap();
    assert_eq!(done["state"], "live", "{done:#}");
    assert_eq!(done["undo_until"], "2026-11-09T23:59:00-05:00");

    let get_promotion = registry.get("get_promotion").unwrap();
    let promotion_id = done["verification"]["items"][0]["result_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let p = get_promotion
        .read(&sj, json!({"promotion_id": promotion_id}))
        .await
        .unwrap();
    assert_eq!(p["applies_to"]["product_count"], 40);
    assert_eq!(p["coupon_ids"], json!(["cpn_vipweekend20"]));
    assert_eq!(p["status"], "scheduled");

    acom_changeset::change_set::undo(&registry, &sj, &cs)
        .await
        .unwrap();
    let p = get_promotion
        .read(&sj, json!({"promotion_id": promotion_id}))
        .await
        .unwrap();
    assert_eq!(p["price_specification_count"], 0);
    assert_eq!(p["coupon_ids"], json!([]));
    assert_eq!(p["status"], "ended");
    let hero = GetDisplaySlot
        .read(
            &sj,
            input(json!({"slot_id": "slot_home_hero", "as_of": "2026-11-07T10:00"})),
        )
        .await
        .unwrap();
    assert_eq!(hero["bookings"].as_array().unwrap().len(), 1, "{hero:#}");
}

/// A shopper sees the page's slots without bookings or free time, and in a slot only the
/// booking that shows to them now; `as_of` and `audience` are for operators.
#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn shoppers_see_only_what_shows_to_them() {
    for who in ["ses_guest", "cus_jamie"] {
        let cx = shopper_cx(who).await;
        let page = ListDisplaySlots
            .read(
                &cx,
                input(json!({"page": "main", "valid_from": "2026-11-06T00:00",
                             "valid_through": "2026-11-08T23:59"})),
            )
            .await
            .unwrap();
        let slots = page["items"].as_array().unwrap();
        assert!(!slots.is_empty());
        assert!(
            slots
                .iter()
                .all(|s| s.get("availability").is_none() && s.get("bookings").is_none()),
            "{page:#}"
        );
        let hero = GetDisplaySlot
            .read(&cx, input(json!({"slot_id": "slot_home_hero"})))
            .await
            .unwrap();
        let ids: Vec<&str> = hero["bookings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| b["display_booking_id"].as_str())
            .collect();
        assert_eq!(ids, ["dsp_fall_layers"], "{who}: {hero:#}");
        assert_eq!(hero["showing"].as_array().unwrap().len(), 1);
        let e = GetDisplaySlot
            .read(
                &cx,
                input(json!({"slot_id": "slot_home_hero", "as_of": "2026-11-07T10:00"})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }
}

/// Removes the category and mapping [`categories_are_created_mapped_and_switched_off`] makes.
async fn clean_gift_ideas(cx: &Cx) {
    let mut tx = cx.begin().await.unwrap();
    for sql in [
        "DELETE FROM merchandising.category_mapping WHERE tenant_id = $1 AND display_category_id = 'cat_gift_ideas'",
        "DELETE FROM merchandising.defined_term WHERE tenant_id = $1 AND id = 'cat_gift_ideas'",
    ] {
        sqlx::query(sql)
            .bind(cx.tenant.as_str())
            .execute(&mut *tx.tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
}

/// Previews and runs a write that needs no approval; returns the preview.
async fn run(registry: &Registry<Cx>, cx: &Cx, tool: &str, input: Value) -> acom_contract::Preview {
    let p = acom_changeset::preview(registry, cx, tool, input, None)
        .await
        .unwrap_or_else(|e| panic!("{tool}: {e}"));
    assert!(!p.needs_approval);
    acom_changeset::commit(registry, cx, tool, &p.preview_id)
        .await
        .unwrap_or_else(|e| panic!("{tool}: {e}"));
    p
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn categories_are_created_mapped_and_switched_off() {
    let registry = registry();
    let sj = operator_cx("adm_sj").await;
    clean_gift_ideas(&sj).await;

    let p = run(
        &registry,
        &sj,
        "create_category",
        json!({"term_set": "display", "name": "Gift ideas"}),
    )
    .await;
    assert!(p.checks.iter().all(|c| c.passed), "{:#?}", p.checks);
    let e = acom_changeset::preview(
        &registry,
        &sj,
        "create_category",
        json!({"term_set": "display", "name": "Gift ideas"}),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "duplicate_term_code");
    let e = acom_changeset::preview(
        &registry,
        &sj,
        "create_category",
        json!({"term_set": "standard", "name": "Gift boxes", "parent_id": "cat_gift_ideas"}),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "cross_set_violation");

    // Standard → display only; the summer outerwear products show under the new category.
    let e = acom_changeset::preview(
        &registry,
        &sj,
        "map_category",
        json!({"standard_category_id": "cat_new_in", "display_category_ids": ["cat_gift_ideas"]}),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "cross_set_violation");
    let m = run(
        &registry,
        &sj,
        "map_category",
        json!({"standard_category_id": "cat_summer_outerwear",
        "display_category_ids": ["cat_gift_ideas"], "is_primary": true}),
    )
    .await;
    assert_eq!(m.changes[0].to["products_shown"], 42);
    let one = GetCategory
        .read(&sj, input(json!({"category_id": "cat_summer_outerwear"})))
        .await
        .unwrap();
    assert_eq!(one["mappings"][0]["id"], "map_summer_outerwear_gift_ideas");
    assert_eq!(one["mappings"][0]["is_primary"], true);

    // A category with a mapping cannot be switched off; without one it can.
    let off = acom_changeset::preview(
        &registry,
        &sj,
        "update_category",
        json!({"category_id": "cat_gift_ideas", "active": false}),
        None,
    )
    .await
    .unwrap();
    let c = off
        .checks
        .iter()
        .find(|c| c.rule == "category.delete_only_if")
        .unwrap();
    assert!(!c.passed);
    assert_eq!(c.fix_tool.as_deref(), Some("unmap_category"));
    run(
        &registry,
        &sj,
        "unmap_category",
        json!({"category_mapping_id": "map_summer_outerwear_gift_ideas"}),
    )
    .await;
    run(
        &registry,
        &sj,
        "update_category",
        json!({"category_id": "cat_gift_ideas", "active": false, "name": "Gifts"}),
    )
    .await;
    let after = GetCategory
        .read(&sj, input(json!({"category_id": "cat_gift_ideas"})))
        .await
        .unwrap();
    assert_eq!(
        (after["name"].clone(), after["active"].clone()),
        (json!("Gifts"), json!(false))
    );
    let e = acom_changeset::preview(
        &registry,
        &sj,
        "update_category",
        json!({"category_id": "cat_outerwear", "parent_id": "cat_summer_outerwear"}),
        None,
    )
    .await
    .unwrap();
    assert!(
        !e.checks
            .iter()
            .find(|c| c.rule == "category.no_cycle")
            .unwrap()
            .passed
    );

    clean_gift_ideas(&sj).await;
}
