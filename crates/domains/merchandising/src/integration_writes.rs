//! Integration tests of the slot, popup, curated page and cache operations against the demo
//! store in `DATABASE_URL` (migrated, seeded, demo clock Fri Nov 6 11:00). Each test previews
//! through the operation, then runs the plan, reads it back and undoes it inside a transaction
//! it never commits, so the store is left as it was. Run on request:
//! `cargo test -p acom-merchandising -- --ignored --test-threads=1`.

use acom_contract::{ApprovalNeed, ChangeOp, ReadOp, WriteOp};
use acom_store::TxCx;
use serde_json::{Value, json};

use crate::cache::invalidate_display_cache::{ClearPlan, InvalidateDisplayCache};
use crate::collection_page::create_curation::CreateCuration;
use crate::collection_page::get_curation::GetCuration;
use crate::collection_page::list_curations::ListCurations;
use crate::collection_page::publish_curation::PublishCuration;
use crate::collection_page::update_curation::UpdateCuration;
use crate::display_slot::arrange_display_slot::ArrangeDisplaySlot;
use crate::display_slot::get_display_slot::GetDisplaySlot;
use crate::display_slot::list_display_slots::ListDisplaySlots;
use crate::display_slot::schedule_display::{DisplayRun, ScheduleDisplay};
use crate::popup::PopupRow;
use crate::popup::list_popups::ListPopups;
use crate::popup::set_popup::{SetPopup, SetPopupPlan};
use crate::testing::{at, operator_cx, shopper_cx};

fn input<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

fn ids(v: &Value) -> Vec<String> {
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["id"].as_str().map(str::to_owned))
        .collect()
}

/// Marks a product discontinued inside `tx` (gone again when `tx` is dropped).
async fn discontinue(tx: &mut TxCx, product: &str) {
    sqlx::query(
        "UPDATE catalog.product_group SET approval_status = 'discontinued'
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(product)
    .execute(&mut *tx.tx)
    .await
    .unwrap();
}

/// mk-hero-free-next-weekend: Sat Nov 14 – Sun Nov 15 the home hero shows Fall layers to
/// everyone with no end date and is free all weekend for VIP, Gold and Member.
#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn hero_is_free_for_every_tier_next_weekend() {
    let cx = operator_cx("adm_sj").await;
    let page = ListDisplaySlots
        .read(
            &cx,
            input(json!({"page": "main", "slot_type": "hero",
                         "valid_from": "2026-11-14T00:00", "valid_through": "2026-11-15T23:59"})),
        )
        .await
        .unwrap();
    let hero = &page["items"][0];
    assert_eq!(hero["id"], "slot_home_hero");
    let bookings = hero["bookings"].as_array().unwrap();
    assert_eq!(bookings.len(), 1, "{hero:#}");
    assert_eq!(bookings[0]["display_booking_id"], "dsp_fall_layers");
    assert_eq!(bookings[0]["headline"], "Fall layers");
    assert_eq!(bookings[0]["audience"], "everyone");
    assert_eq!(bookings[0]["valid_through"], Value::Null);
    let weekend = |a: &str| {
        json!({"audience": a, "free_from": "2026-11-14T00:00:00-05:00",
               "free_until": "2026-11-15T23:59:00-05:00"})
    };
    assert_eq!(
        hero["availability"],
        json!([weekend("vip"), weekend("gold"), weekend("member")])
    );
    let now = GetDisplaySlot
        .read(&cx, input(json!({"slot_id": "slot_home_hero"})))
        .await
        .unwrap();
    assert_eq!(now["bookings"][0]["banner"]["headline"], "Fall layers");
}

/// `arrange_display_slot`: checks, kinds per slot, and a run read back and undone.
#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn slot_items_are_arranged() {
    let cx = operator_cx("adm_sj").await;
    let p = ArrangeDisplaySlot
        .preview(
            &cx,
            input(
                json!({"slot_id": "slot_home_new_in", "slot_type": "product_grid",
                "item_list_element": [{"position": 2, "item_id": "prd_wool_overcoat"},
                                      {"position": 1, "item_id": "prd_denim_jacket"},
                                      {"position": 3, "item_id": "prd_chore_coat"}]}),
            ),
        )
        .await
        .unwrap();
    assert!(p.checks.iter().all(|c| c.passed), "{:#?}", p.checks);
    assert_eq!(p.approval, ApprovalNeed::NotNeeded);
    assert_eq!(p.plan.items[0].item_id, "prd_denim_jacket");
    let brands = ArrangeDisplaySlot
        .preview(
            &cx,
            input(json!({"slot_id": "slot_home_brands",
                "item_list_element": [{"position": 1, "item_id": "brn_fieldstone"},
                                      {"position": 2, "item_id": "brn_northwind"}]})),
        )
        .await
        .unwrap();
    assert!(brands.checks.iter().all(|c| c.passed));
    for (bad, code) in [
        (
            json!({"slot_id": "slot_home_new_in",
                   "item_list_element": [{"position": 1, "item_id": "bnr_fall_layers"}]}),
            "slot_type_mismatch",
        ),
        (
            json!({"slot_id": "slot_home_new_in", "slot_type": "hero", "item_list_element": []}),
            "slot_type_mismatch",
        ),
        (
            json!({"slot_id": "slot_home_new_in",
                   "item_list_element": [{"position": 1, "item_id": "prd_nothing"}]}),
            "not_found",
        ),
        (
            json!({"slot_id": "slot_nothing", "item_list_element": []}),
            "slot_not_found",
        ),
    ] {
        let e = ArrangeDisplaySlot
            .preview(&cx, input(bad))
            .await
            .unwrap_err();
        assert_eq!(e.code, code);
    }

    let mut tx = cx.begin().await.unwrap();
    let done = ArrangeDisplaySlot.commit(&mut tx, &p.plan).await.unwrap();
    assert_eq!(done.produced["slot_id"], "slot_home_new_in");
    let slot = crate::repo::slot(&mut tx, "slot_home_new_in")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(slot.item_list_element, json!(p.plan.items));
    ArrangeDisplaySlot
        .undo(&mut tx, &p.plan, &done)
        .await
        .unwrap();
    let slot = crate::repo::slot(&mut tx, "slot_home_new_in")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(slot.item_list_element[0]["item_id"], "prd_wool_overcoat");
    // A product discontinued after the preview stops the run.
    discontinue(&mut tx, "prd_chore_coat").await;
    let e = ArrangeDisplaySlot
        .commit(&mut tx, &p.plan)
        .await
        .unwrap_err();
    assert_eq!(e.code, "item_discontinued");
}

/// `schedule_display`: a booking of the slot's items for everyone, approved by a person; the
/// time is then taken, and undo cancels the booking that never showed.
#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn slot_items_are_booked_for_a_period() {
    let cx = operator_cx("adm_sj").await;
    let p = ScheduleDisplay
        .preview(
            &cx,
            input(
                json!({"slot_id": "slot_home_new_in", "audience": "everyone",
                "valid_from": "2026-11-13T00:00", "valid_through": "2026-11-20T23:59",
                "channels": ["web", "app"]}),
            ),
        )
        .await
        .unwrap();
    assert!(p.checks.iter().all(|c| c.passed), "{:#?}", p.checks);
    assert_eq!(
        p.checks.iter().map(|c| c.rule.as_str()).collect::<Vec<_>>(),
        [
            "slot_free",
            "display.start_not_in_past",
            "display.exclude_discontinued"
        ]
    );
    assert_eq!(
        p.approval,
        ApprovalNeed::Needed {
            reason: "Shows on the main page to everyone".into()
        }
    );
    assert_eq!(p.changes[0].op, ChangeOp::Schedule);
    assert_eq!(p.plan.items.len(), 4);
    let vip = ScheduleDisplay
        .preview(
            &cx,
            input(json!({"slot_id": "slot_home_new_in", "audience": "vip",
                "valid_from": "2026-11-13T00:00", "valid_through": "2026-11-20T23:59"})),
        )
        .await
        .unwrap();
    assert_eq!(
        vip.approval,
        ApprovalNeed::Needed {
            reason: "Shows on the main page to 1,284 VIP members".into()
        }
    );
    for (bad, code) in [
        (json!({"slot_id": "slot_home_stories"}), "empty_slot"),
        (
            json!({"valid_from": "2026-11-01T00:00"}),
            "valid_from_in_past",
        ),
        (json!({"audience": "platinum"}), "tier_not_found"),
    ] {
        let mut i = json!({"slot_id": "slot_home_new_in", "audience": "everyone",
            "valid_from": "2026-11-13T00:00", "valid_through": "2026-11-20T23:59"});
        for (k, v) in bad.as_object().unwrap() {
            i[k] = v.clone();
        }
        let e = ScheduleDisplay.preview(&cx, input(i)).await.unwrap_err();
        assert_eq!(e.code, code);
    }

    let mut tx = cx.begin().await.unwrap();
    let done = ScheduleDisplay.commit(&mut tx, &p.plan).await.unwrap();
    let b = crate::repo::booking(&mut tx, &done.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(b.banner_id, None);
    assert_eq!(b.display_status, "scheduled");
    assert_eq!(
        b.sales_channel,
        Some(vec!["web".to_owned(), "app".to_owned()])
    );
    assert_eq!(b.item_list_element, Some(json!(p.plan.items)));
    let again = ScheduleDisplay
        .commit(
            &mut tx,
            &DisplayRun {
                booking_id: "dsp_second".into(),
                ..p.plan.clone()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(again.code, "slot_taken");
    ScheduleDisplay.undo(&mut tx, &p.plan, &done).await.unwrap();
    assert!(
        crate::repo::booking(&mut tx, &done.id)
            .await
            .unwrap()
            .is_none()
    );
}

/// `list_popups` and `set_popup`: the seeded popups by kind, page and moment; shoppers see only
/// what opens now; a new popup takes the first free priority; a third popup at once is refused.
#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn popups_open_on_their_pages() {
    let sj = operator_cx("adm_sj").await;
    let all = ListPopups.read(&sj, input(json!({}))).await.unwrap();
    assert_eq!(ids(&all), ["pop_free_shipping", "pop_holiday_hours"]);
    let christmas = ListPopups
        .read(&sj, input(json!({"as_of": "2026-12-24T10:00"})))
        .await
        .unwrap();
    assert_eq!(ids(&christmas), ["pop_holiday_hours"]);
    assert_eq!(christmas["items"][0]["status"], "active");
    let product_pages = ListPopups
        .read(&sj, input(json!({"page": "product"})))
        .await
        .unwrap();
    assert_eq!(ids(&product_pages), ["pop_free_shipping"]);
    let notices = ListPopups
        .read(&sj, input(json!({"popup_type": "notice"})))
        .await
        .unwrap();
    assert_eq!(ids(&notices), ["pop_holiday_hours"]);
    for who in ["ses_guest", "cus_jamie"] {
        let cx = shopper_cx(who).await;
        let now = ListPopups.read(&cx, input(json!({}))).await.unwrap();
        assert_eq!(ids(&now), ["pop_free_shipping"], "{who}");
        let e = ListPopups
            .read(&cx, input(json!({"as_of": "2026-12-24T10:00"})))
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    // Gift cards Dec 21–23 on the main page: holiday hours is open then too, two at once.
    let p = SetPopup
        .preview(
            &sj,
            input(json!({"name": "Gift cards", "popup_type": "event",
                "content": {"text": "Last-minute gift cards, sent by email.", "url": "/gift-cards"},
                "pages": ["main"], "valid_from": "2026-12-21T00:00",
                "valid_through": "2026-12-23T23:59"})),
        )
        .await
        .unwrap();
    assert!(p.checks.iter().all(|c| c.passed), "{:#?}", p.checks);
    assert_eq!(p.plan.popup.id, "pop_gift_cards");
    assert_eq!(p.plan.popup.priority, 2);
    assert_eq!(
        p.approval,
        ApprovalNeed::Needed {
            reason: "Opens on the main page for every shopper Dec 21–23".into()
        }
    );
    // Changing a popup that is open keeps its start; a new start in the past is refused.
    let change = SetPopup
        .preview(
            &sj,
            input(
                json!({"popup_id": "pop_free_shipping", "popup_type": "event",
                "content": {"text": "Free shipping over $80 until Nov 30."},
                "pages": ["main", "product"], "suppress_days": 7,
                "valid_from": "2026-11-01T00:00:00-04:00", "valid_through": "2026-11-30T23:59"}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(change.changes[0].op, ChangeOp::Update);
    assert_eq!(change.plan.popup.name, "Free shipping over $80");
    assert!(
        change.checks.iter().all(|c| c.passed),
        "{:#?}",
        change.checks
    );
    let mut late = json!({"popup_type": "notice", "content": {"text": "x"}, "pages": ["main"],
        "valid_from": "2026-11-02T00:00", "valid_through": "2026-11-03T23:59"});
    let e = SetPopup
        .preview(&sj, input(late.clone()))
        .await
        .unwrap_err();
    assert_eq!(e.code, "valid_from_in_past");
    late["valid_from"] = json!("2026-11-10T00:00");
    late["valid_through"] = json!("2026-11-12T23:59");
    late["suppress_days"] = json!(8);
    let e = SetPopup.preview(&sj, input(late)).await.unwrap_err();
    assert_eq!(e.code, "invalid_input");

    let mut tx = sj.begin().await.unwrap();
    let done = SetPopup.commit(&mut tx, &p.plan).await.unwrap();
    assert_eq!(done.produced["popup_id"], "pop_gift_cards");
    let third = SetPopupPlan {
        popup: PopupRow {
            id: "pop_third".into(),
            priority: 3,
            valid_from: at("2026-12-22T00:00"),
            valid_through: at("2026-12-22T23:59"),
            ..p.plan.popup.clone()
        },
        before: None,
    };
    let e = SetPopup.commit(&mut tx, &third).await.unwrap_err();
    assert_eq!(e.code, "too_many_popups");
    SetPopup.undo(&mut tx, &p.plan, &done).await.unwrap();
    assert!(
        crate::popup::popup(&mut tx, "pop_gift_cards")
            .await
            .unwrap()
            .is_none()
    );
}

/// The curated pages of the seed as operators and shoppers see them; a new draft; a
/// publication; and when `update_curation` needs a person.
#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn curated_pages_are_made_published_and_read() {
    let sj = operator_cx("adm_sj").await;
    let all = ListCurations.read(&sj, input(json!({}))).await.unwrap();
    let status: Vec<(&str, &str)> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| (i["id"].as_str().unwrap(), i["status"].as_str().unwrap()))
        .collect();
    assert_eq!(
        status,
        [
            ("col_travel_light", "active"),
            ("col_denim_bundle", "scheduled"),
            ("col_winter_edit", "draft"),
        ]
    );
    let drafts = ListCurations
        .read(&sj, input(json!({"status": "draft"})))
        .await
        .unwrap();
    assert_eq!(ids(&drafts), ["col_winter_edit"]);
    let winter = GetCuration
        .read(&sj, input(json!({"collection_page_id": "col_winter_edit"})))
        .await
        .unwrap();
    assert_eq!(winter["has_part"][0]["name"], "Coats");
    assert_eq!(winter["product_count"], 3);
    assert_eq!(winter["keywords"], json!(["winter coat"]));
    for who in ["ses_guest", "cus_jamie"] {
        let cx = shopper_cx(who).await;
        let now = ListCurations.read(&cx, input(json!({}))).await.unwrap();
        assert_eq!(ids(&now), ["col_travel_light"], "{who}");
        let e = GetCuration
            .read(&cx, input(json!({"collection_page_id": "col_winter_edit"})))
            .await
            .unwrap_err();
        assert_eq!(e.code, "collection_page_not_found");
    }

    let gifts = json!({"name": "Holiday gifts",
        "has_part": [
            {"name": "Knits", "item_list_element": ["prd_cashmere_sweater", "prd_ribbed_beanie"]},
            {"name": "Small things", "item_list_element": ["prd_hiking_socks", "prd_canvas_tote"]}],
        "banner_ids": ["bnr_fall_layers"], "keywords": ["Gifts"]});
    let c = CreateCuration
        .preview(&sj, input(gifts.clone()))
        .await
        .unwrap();
    assert!(c.checks.iter().all(|x| x.passed), "{:#?}", c.checks);
    assert_eq!(c.approval, ApprovalNeed::NotNeeded);
    assert_eq!(c.plan.page.id, "col_holiday_gifts");
    assert_eq!(c.changes.len(), 3);
    let mut empty = gifts.clone();
    empty["has_part"][1]["item_list_element"] = json!([]);
    let e = CreateCuration.preview(&sj, input(empty)).await.unwrap();
    let check = e
        .checks
        .iter()
        .find(|x| x.rule == "curation.no_empty_section")
        .unwrap();
    assert!(!check.passed);
    assert_eq!(check.fix_tool.as_deref(), Some("create_curation"));
    let mut no_banner = gifts;
    no_banner["banner_ids"] = json!(["bnr_nothing"]);
    let e = CreateCuration
        .preview(&sj, input(no_banner))
        .await
        .unwrap_err();
    assert_eq!(e.code, "banner_not_found");

    let mut tx = sj.begin().await.unwrap();
    let done = CreateCuration.commit(&mut tx, &c.plan).await.unwrap();
    let stored = crate::collection_page::page(&mut tx, "col_holiday_gifts")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.display_status, "draft");
    assert_eq!(stored.keywords, ["gifts"]);
    CreateCuration.undo(&mut tx, &c.plan, &done).await.unwrap();
    assert!(
        crate::collection_page::page(&mut tx, "col_holiday_gifts")
            .await
            .unwrap()
            .is_none()
    );
    drop(tx);

    let p = PublishCuration
        .preview(
            &sj,
            input(json!({"collection_page_id": "col_winter_edit",
                "valid_from": "2026-11-20T00:00", "valid_through": "2026-11-30T23:59"})),
        )
        .await
        .unwrap();
    assert!(p.checks.iter().all(|x| x.passed), "{:#?}", p.checks);
    assert_eq!(
        p.approval,
        ApprovalNeed::Needed {
            reason: "Publishes a page with 3 products to everyone".into()
        }
    );
    assert_eq!(p.changes[0].to["status"], "scheduled");
    for (page, from, code) in [
        ("col_travel_light", "2026-11-20T00:00", "already_published"),
        ("col_winter_edit", "2026-11-01T00:00", "valid_from_in_past"),
        (
            "col_nothing",
            "2026-11-20T00:00",
            "collection_page_not_found",
        ),
    ] {
        let e = PublishCuration
            .preview(
                &sj,
                input(json!({"collection_page_id": page, "valid_from": from,
                             "valid_through": "2026-11-30T23:59"})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, code);
    }
    let mut tx = sj.begin().await.unwrap();
    let done = PublishCuration.commit(&mut tx, &p.plan).await.unwrap();
    let published = crate::collection_page::page(&mut tx, "col_winter_edit")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(published.display_status, "scheduled");
    assert_eq!(published.valid_from, p.plan.page.valid_from);
    PublishCuration.undo(&mut tx, &p.plan, &done).await.unwrap();
    let draft = crate::collection_page::page(&mut tx, "col_winter_edit")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (draft.display_status.as_str(), draft.valid_from),
        ("draft", None)
    );
    // The chore coat discontinued after the preview stops the publication.
    discontinue(&mut tx, "prd_chore_coat").await;
    let e = PublishCuration.commit(&mut tx, &p.plan).await.unwrap_err();
    assert_eq!(e.code, "product_discontinued");
    drop(tx);

    // Changing a draft needs no one; changing a scheduled page reaches shoppers.
    let keywords = |id: &str| json!({"collection_page_id": id, "keywords": ["new words"]});
    let draft = UpdateCuration
        .preview(&sj, input(keywords("col_winter_edit")))
        .await
        .unwrap();
    assert_eq!(draft.approval, ApprovalNeed::NotNeeded);
    let scheduled = UpdateCuration
        .preview(&sj, input(keywords("col_denim_bundle")))
        .await
        .unwrap();
    assert!(matches!(scheduled.approval, ApprovalNeed::Needed { .. }));
    let e = UpdateCuration
        .preview(&sj, input(json!({"collection_page_id": "col_winter_edit"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_input");
    let mut tx = sj.begin().await.unwrap();
    let done = UpdateCuration.commit(&mut tx, &draft.plan).await.unwrap();
    let changed = crate::collection_page::page(&mut tx, "col_winter_edit")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(changed.keywords, ["new words"]);
    UpdateCuration
        .undo(&mut tx, &draft.plan, &done)
        .await
        .unwrap();
    let back = crate::collection_page::page(&mut tx, "col_winter_edit")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(back.keywords, ["winter coat"]);
}

/// `invalidate_display_cache`: the keys a clear covers, recorded and not run; clearing
/// everything is limited per hour.
#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn display_cache_clears_are_recorded_not_run() {
    let sj = operator_cx("adm_sj").await;
    let p = InvalidateDisplayCache
        .preview(
            &sj,
            input(
                json!({"resource_type": "display_slot", "resource_id": "slot_home_hero",
                         "reason": "VIP weekend banner"}),
            ),
        )
        .await
        .unwrap();
    assert!(p.checks.iter().all(|c| c.passed), "{:#?}", p.checks);
    assert_eq!(p.approval, ApprovalNeed::NotNeeded);
    assert_eq!(
        p.changes[0].to["cache_keys"],
        json!(["display_slot:slot_home_hero", "page:main"])
    );
    assert_eq!(p.changes[0].to["keys_cleared"], 0);
    let popup = InvalidateDisplayCache
        .preview(
            &sj,
            input(json!({"resource_type": "popup", "resource_id": "pop_free_shipping"})),
        )
        .await
        .unwrap();
    assert_eq!(
        popup.changes[0].to["cache_keys"],
        json!(["popup:pop_free_shipping", "page:main", "page:product"])
    );
    for (bad, code) in [
        (
            json!({"resource_type": "display_slot", "resource_id": "slot_nothing"}),
            "not_found",
        ),
        (
            json!({"resource_type": "all", "resource_id": "slot_home_hero"}),
            "invalid_input",
        ),
        (
            json!({"resource_type": "popup", "resource_id": "slot_home_hero"}),
            "invalid_input",
        ),
    ] {
        let e = InvalidateDisplayCache
            .preview(&sj, input(bad))
            .await
            .unwrap_err();
        assert_eq!(e.code, code);
    }

    let all = InvalidateDisplayCache
        .preview(&sj, input(json!({"resource_type": "all"})))
        .await
        .unwrap();
    assert!(all.checks.iter().all(|c| c.passed), "{:#?}", all.checks);
    let mut tx = sj.begin().await.unwrap();
    let done = InvalidateDisplayCache
        .commit(&mut tx, &p.plan)
        .await
        .unwrap();
    let row = crate::cache::clear(&mut tx, &done.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.cache_key, p.plan.keys);
    for n in 0..2 {
        InvalidateDisplayCache
            .commit(
                &mut tx,
                &ClearPlan {
                    id: format!("cin_all_{n}"),
                    ..all.plan.clone()
                },
            )
            .await
            .unwrap();
    }
    let e = InvalidateDisplayCache
        .commit(&mut tx, &all.plan)
        .await
        .unwrap_err();
    assert_eq!(e.code, "rate_limited");
}
