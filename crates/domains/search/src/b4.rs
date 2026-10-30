//! Scene B4 of `docs/scenarios.md` §6 against a database (`DATABASE_URL`, migrated and seeded):
//! SJ's supplier sheet becomes 30 drafts through a change set, the drafts stay out of every
//! shopper read, one of them is published in a second change set and shoppers find it, and
//! undoing both change sets leaves the catalog as it was. Run on request:
//! `cargo test -p acom-search -- --ignored --test-threads=1`.
//!
//! The test removes anything a failed earlier run left behind before it starts, and the undo at
//! the end removes every draft it made. The change sets themselves stay, as `undone` history.

use std::sync::Arc;

use acom_changeset::NewChangeSet;
use acom_contract::{ReadOp, Registry, RegistryBuilder};
use acom_core::FixedClock;
use acom_core::enums::ActorKind;
use acom_store::Cx;
use serde_json::{Value, json};

use crate::ops::products::{SearchProducts, SearchProductsInput};
use crate::ops::testing;

const SHEET: &str = "upl_fw26_supplier";
const COAT: &str = "prd_fw26_ot_014_chr";

fn registry() -> Registry<Cx> {
    let mut b = RegistryBuilder::default();
    acom_catalog::register(&mut b);
    acom_promotion::register(&mut b);
    acom_merchandising::register(&mut b);
    crate::register(&mut b);
    b.build().unwrap_or_else(|e| panic!("{e:?}"))
}

async fn sj() -> Cx {
    let mut cx = testing::db(ActorKind::Operator, "adm_sj").await;
    let at = cx.time.parse("2026-11-06T10:58").unwrap();
    cx.clock = Arc::new(FixedClock::at(at));
    cx
}

/// Removes drafts an earlier, interrupted run of this test left.
async fn clean(cx: &Cx) {
    let mut tx = cx.begin().await.unwrap();
    for sql in [
        "DELETE FROM catalog.data_feed_item WHERE tenant_id = $1 AND product_group_id LIKE 'prd_fw26%'",
        "DELETE FROM catalog.product_import WHERE tenant_id = $1 AND data_feed_id = 'upl_fw26_supplier'",
        "DELETE FROM merchandising.product_group_category WHERE tenant_id = $1 AND product_group_id LIKE 'prd_fw26%'",
        "DELETE FROM catalog.offer o USING catalog.product p WHERE o.tenant_id = $1 AND p.tenant_id = $1
           AND o.product_id = p.id AND p.product_group_id LIKE 'prd_fw26%'",
        "DELETE FROM catalog.product WHERE tenant_id = $1 AND product_group_id LIKE 'prd_fw26%'",
        "DELETE FROM catalog.product_group WHERE tenant_id = $1 AND id LIKE 'prd_fw26%'",
    ] {
        sqlx::query(sql)
            .bind(cx.tenant.as_str())
            .execute(&mut *tx.tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
}

async fn fw26_count(cx: &Cx) -> i64 {
    let mut tx = cx.read().await.unwrap();
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM catalog.product_group WHERE tenant_id = $1 AND id LIKE 'prd_fw26%'",
    )
    .bind(cx.tenant.as_str())
    .fetch_one(&mut *tx.tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    n
}

/// What Jamie, a shopper, finds for some words.
async fn shopper_finds(query: &str) -> Vec<String> {
    let jamie = testing::db(ActorKind::Shopper, "cus_jamie").await;
    let input: SearchProductsInput =
        serde_json::from_value(json!({"query": query, "limit": 50})).unwrap();
    let out = SearchProducts.read(&jamie, input).await.unwrap();
    out["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["id"].as_str().map(str::to_owned))
        .collect()
}

async fn new_set(cx: &Cx, title: &str, request: &str) -> String {
    let created = acom_changeset::change_set::create(
        cx,
        NewChangeSet {
            title: title.into(),
            request_text: vec![request.into()],
            interpretation: vec![],
            conversation_id: None,
        },
    )
    .await
    .unwrap();
    created["id"].as_str().unwrap().to_owned()
}

async fn submit_and_approve(registry: &Registry<Cx>, cx: &Cx, cs: &str) -> Value {
    let head = acom_changeset::read::get(cx, cs, &[]).await.unwrap();
    let version = i32::try_from(head["version"].as_i64().unwrap()).unwrap();
    acom_changeset::change_set::submit(cx, cs, version)
        .await
        .unwrap();
    acom_changeset::change_set::approve(registry, cx, cs, version)
        .await
        .unwrap()
}

/// Previews and runs a write that needs no approval.
async fn run(registry: &Registry<Cx>, cx: &Cx, tool: &str, input: Value) {
    let p = acom_changeset::preview(registry, cx, tool, input, None)
        .await
        .unwrap_or_else(|e| panic!("{tool}: {e}"));
    assert!(!p.needs_approval, "{tool}: {:?}", p.approval_reason);
    acom_changeset::commit(registry, cx, tool, &p.preview_id)
        .await
        .unwrap_or_else(|e| panic!("{tool}: {e}"));
}

/// Removes what [`a_product_made_by_hand`] makes, before and after it runs.
async fn clean_hand_made(cx: &Cx) {
    let mut tx = cx.begin().await.unwrap();
    for sql in [
        "DELETE FROM catalog.unit_price_specification s USING catalog.offer o, catalog.product p
           WHERE s.tenant_id = $1 AND o.tenant_id = $1 AND p.tenant_id = $1 AND s.offer_id = o.id
             AND o.product_id = p.id AND p.product_group_id = 'prd_trail_shell'",
        "DELETE FROM merchandising.product_group_category WHERE tenant_id = $1 AND product_group_id = 'prd_trail_shell'",
        "DELETE FROM catalog.offer o USING catalog.product p WHERE o.tenant_id = $1 AND p.tenant_id = $1
           AND o.product_id = p.id AND p.product_group_id = 'prd_trail_shell'",
        "DELETE FROM catalog.product WHERE tenant_id = $1 AND product_group_id = 'prd_trail_shell'",
        "DELETE FROM catalog.product_group WHERE tenant_id = $1 AND id = 'prd_trail_shell'",
        "DELETE FROM catalog.brand WHERE tenant_id = $1 AND id = 'brn_harbor_knit_co'",
        "UPDATE catalog.review SET creative_work_status = 'visible' WHERE tenant_id = $1 AND id = 'rev_8812'",
    ] {
        sqlx::query(sql)
            .bind(cx.tenant.as_str())
            .execute(&mut *tx.tx)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_product_made_by_hand() {
    let registry = registry();
    let cx = sj().await;
    clean_hand_made(&cx).await;
    let read = |tool: &str| registry.get(tool).unwrap_or_else(|| panic!("{tool}"));

    run(
        &registry,
        &cx,
        "create_brand",
        json!({"name": "Harbor Knit Co."}),
    )
    .await;
    let e = acom_changeset::preview(
        &registry,
        &cx,
        "create_brand",
        json!({"name": "harbor knit co."}),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "duplicate_brand");
    run(
        &registry,
        &cx,
        "update_brand",
        json!({"brand_id": "brn_harbor_knit_co", "logo": "https://cdn.example/hk.svg"}),
    )
    .await;

    let e = acom_changeset::preview(
        &registry,
        &cx,
        "create_product_group",
        json!({"name": "Trail shell", "category_id": "cat_new_in"}),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(
        e.code, "invalid_category",
        "a display category cannot hold products"
    );
    run(
        &registry,
        &cx,
        "create_product_group",
        json!({"name": "Trail shell", "category_id": "cat_outerwear",
        "brand_id": "brn_harbor_knit_co", "varies_by": ["size"]}),
    )
    .await;

    // Variants with prices: a price under the floor fails its check and cannot run.
    let low = acom_changeset::preview(&registry, &cx, "set_variants", json!({
        "product_id": "prd_trail_shell", "varies_by": {"size": ["S", "M"]},
        "has_variant": [{"sku": "ts-s", "additional_property": {"size": "S"},
                         "offers": {"price": 50.00, "price_currency": "USD"}, "acom:cost": {"price": 48.00, "price_currency": "USD"}}]}), None)
        .await
        .unwrap();
    assert!(
        !low.checks
            .iter()
            .find(|c| c.rule == "price_floor")
            .unwrap()
            .passed
    );
    run(&registry, &cx, "set_variants", json!({
        "product_id": "prd_trail_shell", "varies_by": {"size": ["S", "M"]},
        "has_variant": [
            {"sku": "ts-s", "additional_property": {"size": "S"}, "offers": {"price": 119.00, "price_currency": "USD"},
             "acom:cost": {"price": 48.00, "price_currency": "USD"}},
            {"sku": "ts-m", "additional_property": {"size": "M"}, "offers": {"price": 119.00, "price_currency": "USD"},
             "acom:cost": {"price": 48.00, "price_currency": "USD"}}]})).await;
    let g = read("get_product_group")
        .read(&cx, json!({"product_id": "prd_trail_shell"}))
        .await
        .unwrap();
    assert_eq!(g["approval_status"], "draft");
    assert_eq!(g["category_id"], "cat_outerwear");
    assert_eq!(g["has_variant"].as_array().unwrap().len(), 2);

    run(
        &registry,
        &cx,
        "set_variant_stock",
        json!({"variant_id": "var_ts_m", "inventory_level": 20}),
    )
    .await;
    run(
        &registry,
        &cx,
        "set_safety_stock",
        json!({"variant_id": "var_ts_m", "acom:safety_stock": 3}),
    )
    .await;
    run(
        &registry,
        &cx,
        "update_product",
        json!({"variant_id": "var_ts_m", "name": "Trail shell, medium"}),
    )
    .await;
    run(
        &registry,
        &cx,
        "schedule_price_change",
        json!({"variant_id": "var_ts_m", "price": 99.00,
        "valid_from": "2026-12-01T00:00", "valid_through": "2026-12-31T23:59"}),
    )
    .await;
    let now = read("get_price_breakdown")
        .read(&cx, json!({"variant_id": "var_ts_m"}))
        .await
        .unwrap();
    assert_eq!(now["list_price"].to_string(), "119.00");
    let december = read("get_price_breakdown")
        .read(
            &cx,
            json!({"variant_id": "var_ts_m", "as_of": "2026-12-15T12:00"}),
        )
        .await
        .unwrap();
    assert_eq!(
        december["list_price"].to_string(),
        "99.00",
        "the scheduled price applies on demand"
    );
    let inv = read("get_inventory")
        .read(&cx, json!({"product_id": "prd_trail_shell"}))
        .await
        .unwrap();
    let m = inv["variants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["variant_id"] == "var_ts_m")
        .unwrap()
        .clone();
    assert_eq!(
        (m["inventory_level"].clone(), m["acom:safety_stock"].clone()),
        (json!(20), json!(3))
    );

    // Reviews are moderated after posting.
    run(
        &registry,
        &cx,
        "moderate_review",
        json!({"review_id": "rev_8812", "action": "hide", "reason": "Test"}),
    )
    .await;
    let e = acom_changeset::preview(
        &registry,
        &cx,
        "moderate_review",
        json!({"review_id": "rev_8812", "action": "hide"}),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "invalid_state");
    run(
        &registry,
        &cx,
        "moderate_review",
        json!({"review_id": "rev_8812", "action": "restore"}),
    )
    .await;

    clean_hand_made(&cx).await;
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn b4_supplier_sheet_to_drafts_publish_one_and_undo() {
    let registry = registry();
    let cx = sj().await;
    clean(&cx).await;
    let read = |tool: &str| registry.get(tool).unwrap_or_else(|| panic!("{tool}"));

    // 10:58 "List the 30 products in this supplier sheet…": one import in a new change set.
    let cs = new_set(
        &cx,
        "FW26 supplier sheet",
        "List the 30 products in this supplier sheet. Use our usual pricing and write the descriptions in our tone.",
    )
    .await;
    let p = acom_changeset::preview(
        &registry,
        &cx,
        "batch_import",
        json!({"acom:file": SHEET, "acom:mode": "create"}),
        Some(&cs),
    )
    .await
    .unwrap();
    assert!(p.needs_approval);
    assert_eq!(
        p.approval_reason.as_deref(),
        Some("Creates 30 product drafts")
    );
    let summary = &p.changes[0].to;
    assert_eq!(
        (
            &summary["drafts"],
            &summary["ready"],
            &summary["needs_you"],
            &summary["working"]
        ),
        (&json!(30), &json!(26), &json!(4), &json!(0))
    );
    let check = |rule: &str| p.checks.iter().find(|c| c.rule == rule).unwrap().clone();
    // The three checks of the scene: no duplicate SKU, every price at or above the floor, and
    // 4 required fields left open (saved as flagged drafts, so the check warns, not blocks).
    let skus = check("product.sku_unique");
    assert!(skus.passed);
    assert!(
        skus.detail
            .unwrap()
            .starts_with("No duplicate SKU: 30 products")
    );
    let floor = check("price_floor");
    assert!(floor.passed);
    assert_eq!(
        floor.detail.as_deref(),
        Some("30 of 30 prices at or above the floor (cost + 15%)")
    );
    let open = check("import.required_fields");
    assert!(open.passed);
    assert_eq!(
        open.detail.as_deref(),
        Some(
            "4 drafts need you: fabric mix missing (2), photo shows more than one item (2). They are saved as drafts and cannot be published until filled"
        )
    );
    assert!(check("product.required_on_create").passed);
    // Prices come from the pricing rule: $96 → $239, $52 → $129 (§3.3).
    let row = |id: &str| {
        p.changes
            .iter()
            .find(|c| c.to["product_id"] == id)
            .unwrap()
            .to
            .clone()
    };
    assert_eq!(row(COAT)["price"].to_string(), "239.00");
    assert_eq!(row("prd_fw26_ot_022_olv")["price"].to_string(), "129.00");
    assert_eq!(
        row("prd_fw26_ot_022_olv")["acom:open_fields"],
        json!(["material"])
    );

    // A person saves the drafts by approving the set.
    let done = submit_and_approve(&registry, &cx, &cs).await;
    assert_eq!(done["state"], "live", "{done:#}");
    assert_eq!(fw26_count(&cx).await, 30);
    let drafts = read("search_product_groups")
        .read(&cx, json!({"import_id": "imp_fw26_supplier", "limit": 100}))
        .await
        .unwrap();
    assert_eq!(drafts["total"], 30);
    assert!(
        drafts["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["approval_status"] == "draft")
    );
    let waiting = read("search_product_groups")
        .read(
            &cx,
            json!({"import_id": "imp_fw26_supplier", "draft_status": "needs_you"}),
        )
        .await
        .unwrap();
    assert_eq!(waiting["total"], 4);

    // Drafts are invisible to shoppers: search, the product page, the category.
    assert!(
        !shopper_finds("wool blend overcoat")
            .await
            .iter()
            .any(|i| i.starts_with("prd_fw26"))
    );
    let jamie = testing::db(ActorKind::Shopper, "cus_jamie").await;
    let e = read("get_product")
        .read(&jamie, json!({"product_id": COAT}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_found", "{e}");
    let outerwear = SearchProducts
        .read(
            &jamie,
            serde_json::from_value(json!({"category_id": "cat_outerwear", "limit": 100})).unwrap(),
        )
        .await
        .unwrap();
    assert!(
        !outerwear["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"].as_str().unwrap().starts_with("prd_fw26"))
    );

    // Publishing one is a separate change set: it needs its description first.
    let e = acom_changeset::preview(
        &registry,
        &cx,
        "submit_for_review",
        json!({"product_id": COAT}),
        None,
    )
    .await
    .unwrap();
    let publish = e
        .checks
        .iter()
        .find(|c| c.rule == "product.publish_requires")
        .unwrap();
    assert!(!publish.passed);
    assert_eq!(publish.fix_tool.as_deref(), Some("update_product_group"));
    run(
        &registry,
        &cx,
        "update_product_group",
        json!({"product_id": COAT, "description": "A wool blend coat that keeps its shape through the season. Notch lapel, two buttons, a full lining. Dry clean."}),
    )
    .await;
    run(
        &registry,
        &cx,
        "submit_for_review",
        json!({"product_id": COAT}),
    )
    .await;
    let publish_cs = new_set(
        &cx,
        "Publish the wool blend overcoat",
        "Publish the overcoat.",
    )
    .await;
    let a = acom_changeset::preview(
        &registry,
        &cx,
        "approve_product",
        json!({"product_id": COAT}),
        Some(&publish_cs),
    )
    .await
    .unwrap();
    assert!(a.checks.iter().all(|c| c.passed), "{:#?}", a.checks);
    assert_eq!(
        a.approval_reason.as_deref(),
        Some("Publishes 1 product: Wool blend overcoat, charcoal")
    );
    let published = submit_and_approve(&registry, &cx, &publish_cs).await;
    assert_eq!(published["state"], "live", "{published:#}");
    assert!(
        shopper_finds("wool blend overcoat")
            .await
            .contains(&COAT.to_owned())
    );
    let page = read("get_product")
        .read(&jamie, json!({"product_id": COAT}))
        .await
        .unwrap();
    assert_eq!(page["price"].to_string(), "239.00");
    assert!(page.get("cost").is_none(), "shoppers never see the cost");

    // Undo: the product leaves the store again, then the drafts go.
    acom_changeset::change_set::undo(&registry, &cx, &publish_cs)
        .await
        .unwrap();
    assert!(
        !shopper_finds("wool blend overcoat")
            .await
            .contains(&COAT.to_owned())
    );
    run(
        &registry,
        &cx,
        "reject_product",
        json!({"product_id": COAT, "reason": "Not this season."}),
    )
    .await;
    acom_changeset::change_set::undo(&registry, &cx, &cs)
        .await
        .unwrap();
    assert_eq!(fw26_count(&cx).await, 0);
    let gone = read("search_product_groups")
        .read(&cx, json!({"import_id": "imp_fw26_supplier"}))
        .await
        .unwrap();
    assert_eq!(gone["total"], 0);
}
