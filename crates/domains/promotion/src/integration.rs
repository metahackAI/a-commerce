//! Integration tests: every operation against the demo store in `DATABASE_URL` (migrated,
//! seeded, demo clock Fri Nov 6 11:00). Run on request:
//! `cargo test -p acom-promotion -- --ignored`. Previews change nothing, so these leave the
//! store as they found it. The full B1 change set (with merchandising's category placement)
//! runs in `acom-merchandising`.

use std::future::Future;
use std::pin::Pin;

use acom_contract::{ApprovalNeed, ChangeOp, ReadOp, ToolError, WriteOp};
use acom_store::TxCx;
use serde_json::{Value, json};

use crate::coupon::{CreateCoupon, GetCoupon, IssueCoupon, ListMemberCoupons, SearchCoupons};
use crate::loyalty::{GetLoyaltyBalance, ListLoyaltyTransactions};
use crate::offer::SetSalePrices;
use crate::promotion::{
    EndPromotion, GetPromotion, ListActivePromotions, PausePromotion, SearchPromotions,
};
use crate::service::for_merchandising::{CategoryProducts, provide_category_products};
use crate::testing::{operator_cx, shopper_cx};

/// Stands in for merchandising's category placement: categories exist and hold nothing.
fn empty_categories<'a>(
    _: &'a mut TxCx,
    ids: &'a [String],
) -> Pin<Box<dyn Future<Output = Result<Vec<CategoryProducts>, ToolError>> + Send + 'a>> {
    Box::pin(async move {
        Ok(ids
            .iter()
            .map(|id| CategoryProducts {
                category_id: id.clone(),
                name: id.clone(),
                product_ids: Vec::new(),
            })
            .collect())
    })
}

fn input<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn list_active_promotions_on_the_sale_weekend() {
    provide_category_products(empty_categories);
    let cx = operator_cx().await;
    let out = ListActivePromotions
        .read(
            &cx,
            input(json!({"valid_from": "2026-11-07T00:00", "valid_through": "2026-11-08T23:59"})),
        )
        .await
        .unwrap();
    assert_eq!(out["total"], 4, "{out:#}");
    let e = ListActivePromotions
        .read(
            &cx,
            input(json!({"valid_from": "2026-11-09T00:00", "valid_through": "2026-11-08T23:59"})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_period");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn search_promotions_finds_the_ended_linen_clearance() {
    let cx = operator_cx().await;
    let out = SearchPromotions
        .read(&cx, input(json!({"status": ["ended"]})))
        .await
        .unwrap();
    let ids: Vec<&str> = out["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(ids.contains(&"promo_linen_clearance"), "{out:#}");
    assert!(
        out["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["status"] == "ended")
    );
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn get_promotion_and_its_coupons() {
    provide_category_products(empty_categories);
    let cx = operator_cx().await;
    let out = GetPromotion
        .read(&cx, input(json!({"promotion_id": "promo_birthday"})))
        .await
        .unwrap();
    assert_eq!(out["coupon_ids"], json!(["cpn_birthday"]));
    assert_eq!(out["valid_for_member_tier"], "member");
    let e = GetPromotion
        .read(&cx, input(json!({"promotion_id": "promo_nothing"})))
        .await
        .unwrap_err();
    assert_eq!(
        (e.code.as_ref(), e.next_tool.as_deref()),
        ("not_found", Some("search_promotions"))
    );
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn pause_and_end_preview_a_status_change() {
    let cx = operator_cx().await;
    let p = PausePromotion
        .preview(&cx, input(json!({"promotion_id": "promo_autumn_knit"})))
        .await
        .unwrap();
    assert_eq!(
        (p.changes[0].from.clone(), p.changes[0].to.clone()),
        (json!("running"), json!("paused"))
    );
    assert_eq!(p.approval, ApprovalNeed::NotNeeded);
    let e = PausePromotion
        .preview(&cx, input(json!({"promotion_id": "promo_denim_bundle"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_state");
    let p = EndPromotion
        .preview(&cx, input(json!({"promotion_id": "promo_denim_bundle"})))
        .await
        .unwrap();
    assert_eq!(p.changes[0].to, json!("ended"));
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn set_sale_prices_previews_a_product_list() {
    provide_category_products(empty_categories);
    let cx = operator_cx().await;
    let p = SetSalePrices
        .preview(
            &cx,
            input(
                json!({"name": "VIP weekend", "valid_for_member_tier": "vip",
                "valid_from": "2026-11-07T00:00", "valid_through": "2026-11-08T23:59",
                "scope_mode": "products",
                "product_ids": ["prd_windbreaker", "prd_seersucker_overshirt", "prd_chore_coat"],
                "percent_off": 20,
                "overrides": [{"product_id": "prd_seersucker_overshirt", "percent_off": 15}],
                "exclude_low_stock": true}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        p.approval,
        ApprovalNeed::Needed {
            reason: "Changes 2 prices".into()
        }
    );
    assert!(p.checks.iter().all(|c| c.passed), "{:#?}", p.checks);
    assert!(
        p.changes
            .iter()
            .any(|c| c.op == ChangeOp::Exclude && c.id == "prd_chore_coat")
    );
    assert!(!p.plan.prices.is_empty());
    let e = SetSalePrices
        .preview(
            &cx,
            input(json!({"name": "x", "valid_for_member_tier": "platinum",
                "valid_from": "2026-11-07T00:00", "valid_through": "2026-11-08T23:59",
                "scope_mode": "products", "product_ids": ["prd_windbreaker"], "percent_off": 20})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "tier_not_found");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn create_coupon_previews_with_a_placeholder_promotion() {
    let cx = operator_cx().await;
    let p = CreateCoupon
        .preview(
            &cx,
            input(
                json!({"code": "VIPWEEKEND20", "promotion_id": "$item[1].promotion_id",
                "valid_for_member_tier": "vip", "application": "automatic_at_checkout"}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(p.changes[0].to["eligible_members"], 1284);
    assert_eq!(p.plan.coupon_id, "cpn_vipweekend20");
    assert_eq!(
        p.approval,
        ApprovalNeed::Needed {
            reason: "Applies automatically at checkout for 1,284 VIP members".into()
        }
    );
    let e = CreateCoupon
        .preview(
            &cx,
            input(
                json!({"code": "BIRTHDAY10", "promotion_id": "promo_birthday",
                "valid_for_member_tier": "member", "application": "download"}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "code_taken");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn issue_coupon_refuses_automatic_coupons_and_wrong_tiers() {
    let cx = operator_cx().await;
    let e = IssueCoupon
        .preview(
            &cx,
            input(
                json!({"coupon_id": "cpn_birthday", "issue_mode": "customers",
                "customer_ids": ["cus_jamie"]}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "recipient_tier_mismatch");
    let p = IssueCoupon
        .preview(
            &cx,
            input(
                json!({"coupon_id": "cpn_birthday", "issue_mode": "customers",
                "customer_ids": ["cus_alex"]}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(p.changes[0].to["customers"], 1);
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn coupon_reads() {
    let cx = operator_cx().await;
    let found = SearchCoupons
        .read(&cx, input(json!({"code": "birthday10"})))
        .await
        .unwrap();
    assert_eq!(found["items"][0]["id"], "cpn_birthday");
    let one = GetCoupon
        .read(&cx, input(json!({"coupon_id": "cpn_birthday"})))
        .await
        .unwrap();
    assert!(one["counts"]["used"].as_i64().unwrap() >= 3, "{one:#}");
    let held = ListMemberCoupons
        .read(&cx, input(json!({"customer_id": "cus_elena"})))
        .await
        .unwrap();
    assert_eq!(held["items"][0]["order_id"], "ord_20352");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn loyalty_reads_an_empty_ledger() {
    let cx = operator_cx().await;
    let b = GetLoyaltyBalance
        .read(&cx, input(json!({"customer_id": "cus_jamie"})))
        .await
        .unwrap();
    assert_eq!(b["balance"]["currency"], "USD");
    let t = ListLoyaltyTransactions
        .read(&cx, input(json!({"customer_id": "cus_jamie"})))
        .await
        .unwrap();
    assert_eq!(t["total"], 0);
    let e = GetLoyaltyBalance
        .read(&cx, input(json!({"customer_id": "cus_nobody"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_found");
}

/// A shopper sees only the promotions running now for everyone or for their tier; the tier and
/// statuses are operator inputs.
#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn shoppers_see_the_sales_on_for_them() {
    provide_category_products(empty_categories);
    let ids = |out: &Value| -> Vec<String> {
        let mut v: Vec<String> = out["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p["id"].as_str().map(str::to_owned))
            .collect();
        v.sort();
        v
    };
    let period = json!({"valid_from": "2026-11-06T00:00", "valid_through": "2026-11-06T23:59"});
    let guest = ListActivePromotions
        .read(&shopper_cx("ses_guest").await, input(period.clone()))
        .await
        .unwrap();
    assert_eq!(
        ids(&guest),
        ["promo_autumn_knit", "promo_free_ship_80"],
        "{guest:#}"
    );
    let jamie = ListActivePromotions
        .read(&shopper_cx("cus_jamie").await, input(period.clone()))
        .await
        .unwrap();
    assert!(
        ids(&jamie).iter().all(|i| i != "promo_gold_10" && i != "promo_denim_bundle"),
        "{jamie:#}"
    );
    let mut asked = period.clone();
    asked["valid_for_member_tier"] = json!("gold");
    let e = ListActivePromotions
        .read(&shopper_cx("cus_jamie").await, input(asked))
        .await
        .unwrap_err();
    assert_eq!(e.code, "forbidden");
}
