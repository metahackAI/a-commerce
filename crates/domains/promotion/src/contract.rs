//! Contract tests without a database: every operation's metadata and input schema, the spec's
//! example inputs, plans that carry `$item[n]` placeholders through binding, and read-back
//! comparisons.

use std::collections::BTreeMap;
use std::sync::Arc;

use acom_auth::{Principal, Surface};
use acom_contract::{Kind, Operation, RegistryBuilder};
use acom_core::enums::ActorKind;
use acom_core::{FixedClock, StoreTime, TenantId};
use acom_store::{Cx, Db};
use serde_json::{Map, Value, json};

use crate::coupon::{
    CouponRun, CreateCouponInput, IssueCouponInput, ListMemberCouponsInput, SearchCouponsInput,
    compare_coupon,
};
use crate::loyalty::{LoyaltyBalanceInput, LoyaltyTransactionsInput};
use crate::offer::{PlannedPrice, SaleRun, SetSalePricesInput, compare_sale};
use crate::promotion::{ListActivePromotionsInput, PromotionStateInput, SearchPromotionsInput};
use crate::repo::{CouponRow, PriceRow, PromotionRow};
use crate::testing::{d, policies};

fn parses<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap_or_else(|e| panic!("{e}"))
}

fn refuses<T: serde::de::DeserializeOwned + std::fmt::Debug>(v: Value) {
    assert!(serde_json::from_value::<T>(v).is_err());
}

/// A context that never reaches the database (the pool connects lazily).
fn offline_cx(subject: &str, kind: ActorKind) -> Cx {
    let tenant = TenantId::parse("store_northwind").unwrap();
    Cx {
        tenant: tenant.clone(),
        principal: Arc::new(Principal {
            subject: subject.into(),
            kind,
            tenant,
            surface: Surface::Fo,
            scopes: acom_auth::scopes::shopper(),
            name: None,
            session_id: None,
        }),
        request_id: "req_test".into(),
        clock: Arc::new(FixedClock::at(
            StoreTime::new(chrono_tz::America::New_York)
                .parse("2026-11-06T11:00")
                .unwrap(),
        )),
        time: StoreTime::new(chrono_tz::America::New_York),
        currency: acom_core::Currency::parse("USD").unwrap(),
        policy: Arc::new(policies(true)),
        db: Db::connect_lazy("postgres://nobody@localhost:1/none", 1).unwrap(),
    }
}

#[test]
fn registers_every_operation_with_closed_schemas() {
    let mut b = RegistryBuilder::<Cx>::default();
    crate::register(&mut b);
    let registry = b.build().unwrap_or_else(|e| panic!("{e:#?}"));
    let tools: Vec<&str> = registry.iter().map(|o| o.meta().tool).collect();
    for t in [
        "list_active_promotions",
        "search_promotions",
        "get_promotion",
        "pause_promotion",
        "end_promotion",
        "set_sale_prices",
        "create_coupon",
        "issue_coupon",
        "search_coupons",
        "get_coupon",
        "list_member_coupons",
        "get_loyalty_balance",
        "list_loyalty_transactions",
    ] {
        assert!(tools.contains(&t), "{t} is not registered");
    }
    let schema = registry.get("set_sale_prices").unwrap().input_schema();
    assert_eq!(
        schema["properties"]["scope_mode"]["enum"],
        json!(["category", "products"])
    );
    assert_eq!(
        schema["properties"]["overrides"]["items"]["additionalProperties"],
        false
    );
}

#[test]
fn writes_that_reach_customers_always_need_approval() {
    assert_eq!(crate::offer::SetSalePrices::META.kind, Kind::WriteApproval);
    assert_eq!(crate::coupon::CreateCoupon::META.kind, Kind::WriteApproval);
    assert_eq!(crate::coupon::IssueCoupon::META.kind, Kind::WriteApproval);
    assert_eq!(crate::promotion::PausePromotion::META.kind, Kind::Write);
    assert_eq!(crate::promotion::EndPromotion::META.kind, Kind::Write);
    assert_eq!(
        crate::offer::SetSalePrices::META.produces[0].name,
        "promotion_id"
    );
    // pause and end match no approval threshold: they run after their preview.
    let p = policies(true);
    assert!(
        !p.approval_for("pause_promotion", &json!({"reaches_customers": true}))
            .needed
    );
    assert!(
        !p.approval_for("end_promotion", &json!({"reaches_customers": true}))
            .needed
    );
}

#[test]
fn reads_take_the_spec_inputs() {
    let _: ListActivePromotionsInput = parses(json!({"valid_from": "2026-11-07T00:00",
        "valid_through": "2026-11-08T23:59", "status": ["running", "scheduled"],
        "valid_for_member_tier": "vip", "limit": 20}));
    refuses::<ListActivePromotionsInput>(
        json!({"valid_from": "x", "valid_through": "y", "tier": "vip"}),
    );
    let _: SearchPromotionsInput =
        parses(json!({"status": ["ended"], "discount_layer": "item_price",
        "query": "linen"}));
    refuses::<SearchPromotionsInput>(json!({"status": ["finished"]}));
    let _: SearchCouponsInput =
        parses(json!({"code": "VIPWEEKEND20", "application": "automatic_at_checkout"}));
    let _: ListMemberCouponsInput = parses(json!({"customer_id": "me", "status": ["used"]}));
    let _: LoyaltyBalanceInput = parses(json!({"customer_id": "cus_jamie"}));
    let t: LoyaltyTransactionsInput = parses(json!({"customer_id": "me", "type": ["accrual"]}));
    assert!(t.r#type.is_some());
}

#[test]
fn writes_take_the_spec_inputs() {
    let v2: SetSalePricesInput = parses(json!({
        "name": "VIP weekend", "valid_for_member_tier": "vip",
        "valid_from": "2026-11-07T00:00", "valid_through": "2026-11-08T23:59",
        "scope_mode": "category", "category_id": "cat_summer_outerwear", "percent_off": 20,
        "overrides": [{"product_id": "prd_seersucker_overshirt", "percent_off": 15}],
        "exclude_low_stock": true}));
    assert_eq!(v2.percent_off, d("20"));
    assert_eq!(v2.overrides.unwrap()[0].percent_off, d("15"));
    refuses::<SetSalePricesInput>(json!({"name": "x", "valid_for_member_tier": "vip",
        "valid_from": "a", "valid_through": "b", "scope_mode": "everything", "percent_off": 20}));
    let _: CreateCouponInput = parses(json!({"code": "VIPWEEKEND20",
        "promotion_id": "$item[1].promotion_id", "valid_for_member_tier": "vip",
        "application": "automatic_at_checkout", "stacking": "no_tier_coupons"}));
    let _: IssueCouponInput = parses(json!({"coupon_id": "cpn_birthday",
        "issue_mode": "on_condition", "condition": "birthday"}));
    refuses::<IssueCouponInput>(json!({"coupon_id": "cpn_birthday", "issue_mode": "everyone"}));
    let _: PromotionStateInput = parses(json!({"promotion_id": "promo_autumn_knit",
        "reason": "Stock ran out."}));
}

#[test]
fn ids_accept_placeholders_only_where_asked() {
    assert!(crate::check_id("promotion_id", "promo", "$item[1].promotion_id", true).unwrap());
    assert!(!crate::check_id("promotion_id", "promo", "promo_1042", true).unwrap());
    assert!(crate::check_id("promotion_id", "promo", "$item[1].promotion_id", false).is_err());
    assert!(crate::check_id("promotion_id", "promo", "cpn_x", true).is_err());
}

#[test]
fn a_coupon_plan_binds_the_promotion_the_sale_produced() {
    let run = CouponRun {
        coupon_id: "cpn_vipweekend20".into(),
        code: "VIPWEEKEND20".into(),
        promotion_id: "$item[1].promotion_id".into(),
        valid_for_member_tier: Some("vip".into()),
        application: "automatic_at_checkout".into(),
        stacking: "no_tier_coupons".into(),
        records_benefit_only: true,
    };
    let plan = serde_json::to_value(&run).unwrap();
    assert_eq!(acom_changeset::bind::positions(&plan), vec![1]);
    let produced = BTreeMap::from([(
        1,
        Map::from_iter([("promotion_id".to_owned(), json!("promo_01jc8x"))]),
    )]);
    let bound: CouponRun = parses(acom_changeset::bind::bind(&plan, &produced).unwrap());
    assert_eq!(bound.promotion_id, "promo_01jc8x");
}

#[test]
fn a_sale_plan_round_trips_and_reads_back() {
    let t = |s: &str| {
        StoreTime::new(chrono_tz::America::New_York)
            .parse(s)
            .unwrap()
    };
    let run = SaleRun {
        promotion_id: "promo_x".into(),
        name: "VIP weekend".into(),
        description: "20% off".into(),
        valid_for_member_tier: Some("vip".into()),
        valid_from: t("2026-11-07T00:00"),
        valid_through: t("2026-11-08T23:59"),
        percent_off: d("20"),
        applies_to: json!({"scope": "products"}),
        prices: vec![PlannedPrice {
            id: "ups_windbreaker_m_slate_x".into(),
            offer_id: "off_windbreaker_m_slate".into(),
            product_id: "prd_windbreaker".into(),
            price: d("71.20"),
            price_currency: "USD".into(),
            percent_off: d("20"),
        }],
    };
    let back: SaleRun = parses(serde_json::to_value(&run).unwrap());
    assert_eq!(back, run);
    let promotion = PromotionRow {
        id: "promo_x".into(),
        name: "VIP weekend".into(),
        description: None,
        promotion_status: "scheduled".into(),
        discount_layer: "item_price".into(),
        valid_for_member_tier: Some("vip".into()),
        valid_from: run.valid_from,
        valid_through: Some(run.valid_through),
        percent_off: Some(d("20")),
        amount_off_value: None,
        amount_off_currency: None,
        applies_to: json!({}),
        change_set_id: None,
        version: 1,
    };
    let price = PriceRow {
        id: "ups_windbreaker_m_slate_x".into(),
        offer_id: "off_windbreaker_m_slate".into(),
        price: d("71.20"),
        price_currency: "USD".into(),
        percent_off: Some(d("20")),
        valid_for_member_tier: Some("vip".into()),
        valid_from: run.valid_from,
        valid_through: Some(run.valid_through),
    };
    assert!(compare_sale(&run, Some(&promotion), std::slice::from_ref(&price)).matched);
    let wrong = PriceRow {
        price: d("71.30"),
        ..price
    };
    assert!(!compare_sale(&run, Some(&promotion), &[wrong]).matched);
    let missing = compare_sale(&run, None, &[]);
    assert_eq!(missing.mismatches.len(), 2);
}

#[test]
fn a_coupon_reads_back() {
    let run = CouponRun {
        coupon_id: "cpn_vipweekend20".into(),
        code: "VIPWEEKEND20".into(),
        promotion_id: "promo_x".into(),
        valid_for_member_tier: Some("vip".into()),
        application: "automatic_at_checkout".into(),
        stacking: "no_tier_coupons".into(),
        records_benefit_only: true,
    };
    let row = CouponRow {
        id: "cpn_vipweekend20".into(),
        name: None,
        discount_code: "VIPWEEKEND20".into(),
        promotion_id: "promo_x".into(),
        promotion_name: "VIP weekend".into(),
        valid_for_member_tier: Some("vip".into()),
        application: "automatic_at_checkout".into(),
        stacking: "no_tier_coupons".into(),
        is_benefit_record_only: true,
        valid_from: None,
        valid_through: None,
        version: 1,
    };
    assert!(compare_coupon(&run, Some(&row)).matched);
    let other = CouponRow {
        stacking: "with_tier_coupons".into(),
        ..row
    };
    assert!(!compare_coupon(&run, Some(&other)).matched);
}

#[tokio::test]
async fn shoppers_only_see_their_own_coupons_and_points() {
    let jamie = offline_cx("cus_jamie", ActorKind::Shopper);
    assert_eq!(crate::whose(&jamie, "me").unwrap(), "cus_jamie");
    assert_eq!(crate::whose(&jamie, "cus_jamie").unwrap(), "cus_jamie");
    assert_eq!(
        crate::whose(&jamie, "cus_alex").unwrap_err().code,
        "forbidden"
    );
    let guest = offline_cx("ses_guest", ActorKind::Shopper);
    assert_eq!(crate::whose(&guest, "me").unwrap_err().code, "forbidden");
    let sj = offline_cx("adm_sj", ActorKind::Operator);
    assert_eq!(crate::whose(&sj, "cus_alex").unwrap(), "cus_alex");
    assert_eq!(crate::whose(&sj, "alex").unwrap_err().code, "invalid_input");
}
