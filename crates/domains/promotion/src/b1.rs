//! Scene B1 of `docs/scenarios.md` §6 against the demo seed and the demo policies, without a
//! database: "Run a 20% weekend sale on summer outerwear for VIP members", then "Drop the
//! low-stock ones. Give the overshirt 15% instead."

use std::collections::{BTreeMap, BTreeSet};

use acom_core::StoreTime;
use acom_core::enums::{CouponApplication, PromotionStatus};
use acom_customer::service::for_promotion::Tier;
use chrono::{DateTime, Days, Utc};
use serde_json::json;

use crate::coupon::coupon_reason;
use crate::model::{
    LeftOut, SaleAsk, SalePlan, SaleRules, check_max_discount, check_min_stock, check_overlap,
    check_price_floor, plan_sale,
};
use crate::offer::{SaleRun, as_other, sale_changes};
use crate::testing::{d, member_count, placed_in, policies, products_with_offers, promotions};

fn store_time() -> StoreTime {
    StoreTime::new(chrono_tz::America::New_York)
}

fn at(s: &str) -> DateTime<Utc> {
    store_time().parse(s).unwrap()
}

fn ask(v2: bool) -> SaleAsk {
    let mut overrides = BTreeMap::new();
    if v2 {
        overrides.insert("prd_seersucker_overshirt".to_owned(), d("15"));
    }
    SaleAsk {
        tier: Some("vip".into()),
        percent_off: d("20"),
        overrides,
        exclude_low_stock: v2,
    }
}

fn summer_outerwear() -> Vec<acom_catalog::service::for_promotion::ProductOffers> {
    products_with_offers(&placed_in("cat_summer_outerwear"))
}

fn sale_price(plan: &SalePlan, product: &str) -> String {
    plan.included
        .iter()
        .find(|p| p.product_id == product)
        .and_then(|p| p.shown())
        .map(|o| o.sale_price.to_string())
        .unwrap_or_default()
}

#[test]
fn summer_outerwear_has_42_products() {
    let products = summer_outerwear();
    assert_eq!(products.len(), 42);
    assert!(products.iter().all(|p| p.approval_status == "active"));
}

#[test]
fn v1_prices_all_42_and_fails_the_floor_and_the_stock_checks() {
    let rules = SaleRules::from_policy(&policies(true)).unwrap();
    let ask = ask(false);
    let plan = plan_sale(&rules, &ask, &summer_outerwear());
    assert_eq!(plan.included.len(), 42);
    assert!(plan.excluded.is_empty());

    let floor = check_price_floor(&rules, &ask, &plan);
    assert!(!floor.passed);
    assert_eq!(
        floor.detail.as_deref(),
        Some(
            "Seersucker overshirt at 20% would be $63.20, under the floor $66.70 (cost $58.00 + 15%)"
        )
    );
    assert_eq!(floor.fix_tool.as_deref(), Some("set_sale_prices"));

    let stock = check_min_stock(&rules, &plan);
    assert!(!stock.passed);
    assert_eq!(
        stock.detail.as_deref(),
        Some("Cotton chore coat (7) and Light rain parka (4) are under 10 units")
    );
    assert!(check_max_discount(&rules, &ask, &plan).passed);
    // The approval sentence counts the products whose price changes.
    assert_eq!(
        format!("Changes {} prices", plan.included.len()),
        "Changes 42 prices"
    );
}

#[test]
fn v2_prices_40_of_42_and_passes_every_check() {
    let rules = SaleRules::from_policy(&policies(true)).unwrap();
    let ask = ask(true);
    let plan = plan_sale(&rules, &ask, &summer_outerwear());
    assert_eq!(plan.included.len(), 40);

    // The named prices.
    assert_eq!(sale_price(&plan, "prd_field_jacket"), "102.40");
    assert_eq!(sale_price(&plan, "prd_windbreaker"), "71.20");
    assert_eq!(sale_price(&plan, "prd_seersucker_overshirt"), "67.15");
    assert_eq!(sale_price(&plan, "prd_utility_jacket"), "94.40");
    // Every other product at 20%.
    let at_20 = plan
        .included
        .iter()
        .filter(|p| p.percent_off == d("20"))
        .count();
    assert_eq!(at_20, 39);

    // The two left out, and why.
    let left: Vec<(&str, LeftOut, i64)> = plan
        .excluded
        .iter()
        .map(|e| (e.product_id.as_str(), e.why, e.stock))
        .collect();
    assert_eq!(
        left,
        vec![
            ("prd_chore_coat", LeftOut::LowStock, 7),
            ("prd_rain_parka", LeftOut::LowStock, 4)
        ]
    );

    let max = check_max_discount(&rules, &ask, &plan);
    assert!(max.passed);
    assert_eq!(
        max.detail.as_deref(),
        Some("Highest is 20%; policy allows 30% for vip")
    );
    let floor = check_price_floor(&rules, &ask, &plan);
    assert!(floor.passed);
    assert_eq!(
        floor.detail.as_deref(),
        Some("Seersucker overshirt moved to 15% ($67.15) to stay at cost + 15% ($66.70)")
    );
    let stock = check_min_stock(&rules, &plan);
    assert!(stock.passed);
    assert_eq!(
        stock.detail.as_deref(),
        Some("2 products under 10 units excluded: Cotton chore coat (7), Light rain parka (4)")
    );
}

#[test]
fn v2_overlap_checks_the_four_running_promotions() {
    let rules = SaleRules::from_policy(&policies(true)).unwrap();
    let plan = plan_sale(&rules, &ask(true), &summer_outerwear());
    let ours: BTreeSet<String> = plan.included.iter().map(|p| p.product_id.clone()).collect();
    let now = at("2026-11-06T11:05");
    let (from, through) = (at("2026-11-07T00:00"), at("2026-11-08T23:59"));
    let by_category = BTreeMap::from([("cat_knitwear".to_owned(), placed_in("cat_knitwear"))]);
    let others: Vec<_> = promotions()
        .iter()
        .filter(|r| r.valid_from <= through && r.valid_through.is_none_or(|t| t >= from))
        .map(|r| as_other(r, now, &by_category))
        .filter(|o| rules.overlap_statuses.contains(&o.status))
        .collect();
    assert_eq!(others.len(), 4);
    assert!(others.iter().all(|o| o.status == PromotionStatus::Running));
    let c = check_overlap(&rules, Some("vip"), &ours, &others, "summer outerwear");
    assert!(c.passed);
    assert_eq!(
        c.detail.as_deref(),
        Some("Checked 4 running promotions; none gives an item price on summer outerwear")
    );
}

#[test]
fn v2_changes_one_promotion_40_prices_and_2_exclusions() {
    let rules = SaleRules::from_policy(&policies(true)).unwrap();
    let plan = plan_sale(&rules, &ask(true), &summer_outerwear());
    let run = SaleRun {
        promotion_id: "promo_test".into(),
        name: "VIP weekend".into(),
        description: String::new(),
        valid_for_member_tier: Some("vip".into()),
        valid_from: at("2026-11-07T00:00"),
        valid_through: at("2026-11-08T23:59"),
        percent_off: d("20"),
        applies_to: json!({}),
        prices: Vec::new(),
    };
    let changes = sale_changes(&store_time(), &run, &plan, rules.promo_min_stock);
    assert_eq!(changes.len(), 43);
    assert_eq!(
        serde_json::to_value(&changes[0].to).unwrap(),
        json!({"name": "VIP weekend", "valid_for_member_tier": "vip", "discount_layer": "item_price",
               "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00",
               "products": 40})
    );
    let windbreaker = changes
        .iter()
        .find(|c| c.to["product_id"] == "prd_windbreaker")
        .unwrap();
    assert_eq!(
        windbreaker.from.to_string(),
        r#"{"product_id":"prd_windbreaker","price":89.00,"price_currency":"USD","price_type":"list_price"}"#
    );
    assert_eq!(
        windbreaker.to.to_string(),
        r#"{"product_id":"prd_windbreaker","price":71.20,"price_currency":"USD","price_type":"sale_price","percent_off":20}"#
    );
    let chore = changes.iter().find(|c| c.id == "prd_chore_coat").unwrap();
    assert_eq!(
        chore.to,
        json!({"why": "promo_min_stock", "inventory_level": 7})
    );
}

#[test]
fn coupon_reaches_the_1284_vip_members() {
    let vip = Tier {
        id: "vip".into(),
        name: "VIP".into(),
        rank: 3,
        member_count: member_count("vip"),
    };
    assert_eq!(vip.member_count, 1284);
    assert_eq!(
        coupon_reason(
            CouponApplication::AutomaticAtCheckout,
            "VIPWEEKEND20",
            Some(&vip)
        ),
        "Applies automatically at checkout for 1,284 VIP members"
    );
}

#[test]
fn undo_until_the_day_after_the_sale_ends() {
    // The change-set engine takes the latest `valid_through` of the items' inputs and adds
    // `approval.change_set.undo_window_after_end_days`, to 23:59 store time.
    let days = policies(true)
        .lookup("approval.change_set.undo_window_after_end_days")
        .and_then(|v| v.as_u64())
        .unwrap();
    let end = at("2026-11-08T23:59");
    let day = end
        .with_timezone(&chrono_tz::America::New_York)
        .date_naive()
        + Days::new(days);
    let until = store_time().parse(&format!("{day}T23:59")).unwrap();
    assert_eq!(store_time().format(until), "2026-11-09T23:59:00-05:00");
}
