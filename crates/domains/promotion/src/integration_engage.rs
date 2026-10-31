//! Integration tests of the operations added with the ledger, events, segments and gifts,
//! against the demo store in `DATABASE_URL` (migrated, seeded, demo clock Fri Nov 6 11:00). Run
//! on request: `cargo test -p acom-promotion -- --ignored --test-threads=1`.
//!
//! Previews change nothing. A commit runs inside a transaction the test opens and drops, so it
//! rolls back and the store stays as it was. Sales' order readers are stood in for: one order of
//! Jamie's and two members' purchases.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use acom_contract::{ApprovalNeed, ChangeOp, Job, JobOutcome, ReadOp, ToolError, WriteOp};
use acom_core::{Currency, FixedClock, Money, TenantId};
use acom_store::{Cx, TxCx};
use serde_json::{Value, json};

use crate::conditional_gift::create_conditional_gift::CreateConditionalGift;
use crate::conditional_gift::list_conditional_gifts::ListConditionalGifts;
use crate::coupon::redeem_coupon::RedeemCoupon;
use crate::coupon::restore_coupon::RestoreCoupon;
use crate::coupon::revoke_coupon::RevokeCoupon;
use crate::event::dispatch_reward::DispatchReward;
use crate::event::enter_event::EnterEvent;
use crate::event::get_draw_record::GetDrawRecord;
use crate::event::get_event::GetEvent;
use crate::event::list_event_types::ListEventTypes;
use crate::event::list_my_entries::ListMyEntries;
use crate::event::list_reward_types::ListRewardTypes;
use crate::event::run_draw::RunDraw;
use crate::event::search_events::SearchEvents;
use crate::event::update_event::UpdateEvent;
use crate::event::{create_event::CreateEvent, seeded_order};
use crate::loyalty::expire_loyalty::{ExpireLoyalty, LoyaltyExpires};
use crate::loyalty::get_loyalty_balance::GetLoyaltyBalance;
use crate::loyalty::grant_loyalty::GrantLoyalty;
use crate::loyalty::list_loyalty_transactions::ListLoyaltyTransactions;
use crate::loyalty::spend_loyalty::SpendLoyalty;
use crate::loyalty::{balance, ledger};
use crate::promotion::create_promotion::CreatePromotion;
use crate::promotion::update_promotion::UpdatePromotion;
use crate::segment::define_segment::DefineSegment;
use crate::segment::list_segments::ListSegments;
use crate::segment::preview_segment::PreviewSegment;
use crate::service::for_merchandising::{CategoryProducts, provide_category_products};
use crate::service::for_sales::{MemberPurchases, OrderFacts, Reading, provide_order_history};
use crate::testing::{d, operator_cx, policies, shopper_cx};

fn input<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

fn usd(v: &str) -> Money {
    Money::new(d(v), Currency::USD)
}

fn at(cx: &Cx, s: &str) -> chrono::DateTime<chrono::Utc> {
    cx.time.parse(s).unwrap()
}

/// Categories exist and hold nothing (merchandising's placement is not loaded here).
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

/// One order of Jamie's, placed Fri Nov 6 10:00 for $89.00.
fn orders<'a>(tx: &'a mut TxCx, ids: &'a [String]) -> Reading<'a, Vec<OrderFacts>> {
    Box::pin(async move {
        let when = tx.time.parse("2026-11-06T10:00").unwrap();
        Ok(ids
            .iter()
            .filter(|id| *id == "ord_test_jamie")
            .map(|id| OrderFacts {
                order_id: id.clone(),
                customer_id: Some("cus_jamie".into()),
                order_date: when,
                order_status: "processing".into(),
                sales_channel: "web".into(),
                subtotal: usd("89.00"),
                discount: usd("0.00"),
                coupon_id: None,
                variant_ids: vec!["var_windbreaker_m_slate".into()],
            })
            .collect())
    })
}

/// Jamie and Alex bought recently; Noor last bought in the spring.
fn purchases<'a>(tx: &'a mut TxCx) -> Reading<'a, Vec<MemberPurchases>> {
    Box::pin(async move {
        let t = |s: &str| tx.time.parse(s).unwrap();
        Ok(vec![
            MemberPurchases {
                customer_id: "cus_jamie".into(),
                last_order_at: t("2026-10-24T16:12"),
                spend: usd("700.00"),
                variant_ids: vec!["var_windbreaker_m_black".into()],
            },
            MemberPurchases {
                customer_id: "cus_alex".into(),
                last_order_at: t("2026-10-27T19:12"),
                spend: usd("312.00"),
                variant_ids: Vec::new(),
            },
            MemberPurchases {
                customer_id: "cus_noor".into(),
                last_order_at: t("2026-04-02T12:00"),
                spend: usd("296.00"),
                variant_ids: Vec::new(),
            },
        ])
    })
}

fn stand_ins() {
    provide_category_products(empty_categories);
    provide_order_history(orders, purchases);
}

/// The scheduler, for system-only work.
async fn system_cx() -> Cx {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let db = acom_store::Db::connect(&url, 2).await.expect("connect");
    let runtime = acom_store::Runtime::new(db, policies(false), true);
    runtime
        .context(
            acom_auth::Principal::system(
                TenantId::parse("store_northwind").unwrap(),
                "sys_scheduler",
            ),
            "req_test".into(),
        )
        .await
        .expect("context")
}

fn ids(out: &Value) -> Vec<String> {
    out["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["id"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn the_ledger_shows_jamies_points_and_what_expires_soon() {
    let cx = operator_cx().await;
    let b = GetLoyaltyBalance
        .read(&cx, input(json!({"customer_id": "cus_jamie"})))
        .await
        .unwrap();
    assert_eq!(
        b["balance"],
        json!({"value": 13.94, "currency": "USD"}),
        "{b:#}"
    );
    let soon = &b["expiring_soon"];
    assert_eq!(
        (
            soon[0]["amount"]["value"].to_string(),
            soon[0]["expires"].clone(),
            soon.as_array().map(Vec::len)
        ),
        ("2.00".to_owned(), json!("2026-11-14"), Some(1))
    );
    let t = ListLoyaltyTransactions
        .read(
            &cx,
            input(json!({"customer_id": "cus_jamie", "type": ["spend"]})),
        )
        .await
        .unwrap();
    assert_eq!(t["total"], 1);
    assert_eq!(t["items"][0]["order_id"], "ord_20151");
    let all = ListLoyaltyTransactions
        .read(&cx, input(json!({"customer_id": "cus_jamie", "limit": 5})))
        .await
        .unwrap();
    assert_eq!(
        (all["total"].clone(), all["next_cursor"].clone()),
        (json!(9), json!("o5"))
    );
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_grant_needs_a_person_and_undoing_it_adds_a_reversal() {
    let cx = operator_cx().await;
    let p = GrantLoyalty
        .preview(&cx, input(json!({"customer_id": "cus_alex", "amount": {"value": 5.00, "currency": "USD"}, "reason": "Sorry for the late parcel."})))
        .await
        .unwrap();
    assert_eq!(
        p.approval,
        ApprovalNeed::Needed {
            reason: "Grants $5.00 store points".into()
        }
    );
    assert_eq!(p.changes[0].op, ChangeOp::Grant);
    let big = GrantLoyalty
        .preview(&cx, input(json!({"customer_id": "cus_alex", "amount": {"value": 80.00, "currency": "USD"}, "reason": "Goodwill."})))
        .await
        .unwrap();
    assert_eq!(
        big.approval,
        ApprovalNeed::Needed {
            reason: "Grants $80.00 store points; amount over $50.00".into()
        }
    );
    let e = GrantLoyalty
        .preview(&cx, input(json!({"customer_id": "cus_alex", "amount": {"value": 600.00, "currency": "USD"}, "reason": "Too much."})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "over_limit");

    let today = at(&cx, "2026-11-06").date_naive();
    let mut tx = cx.begin().await.unwrap();
    let before = balance(&ledger(&mut tx, "cus_alex").await.unwrap(), today);
    let done = GrantLoyalty.commit(&mut tx, &p.plan).await.unwrap();
    assert!(done.id.starts_with("lyt_"));
    assert_eq!(
        balance(&ledger(&mut tx, "cus_alex").await.unwrap(), today),
        before + d("5.00")
    );
    GrantLoyalty.undo(&mut tx, &p.plan, &done).await.unwrap();
    let after = ledger(&mut tx, "cus_alex").await.unwrap();
    assert_eq!(balance(&after, today), before);
    assert!(after.iter().any(|e| e.transaction_type == "reversal"));
    drop(tx);
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn spending_needs_the_points_and_happens_once_per_order() {
    let cx = operator_cx().await;
    let e = SpendLoyalty
        .preview(&cx, input(json!({"customer_id": "cus_jamie", "order_id": "ord_test_jamie", "amount": {"value": 20.00, "currency": "USD"}})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "insufficient_balance");
    let p = SpendLoyalty
        .preview(&cx, input(json!({"customer_id": "cus_jamie", "order_id": "ord_test_jamie", "amount": {"value": 5.00, "currency": "USD"}})))
        .await
        .unwrap();
    assert_eq!(p.approval, ApprovalNeed::NotNeeded);
    let mut tx = cx.begin().await.unwrap();
    let first = SpendLoyalty.commit(&mut tx, &p.plan).await.unwrap();
    let again = SpendLoyalty.commit(&mut tx, &p.plan).await.unwrap();
    assert_eq!(first.id, again.id);
    let today = at(&cx, "2026-11-06").date_naive();
    assert_eq!(
        balance(&ledger(&mut tx, "cus_jamie").await.unwrap(), today),
        d("8.94")
    );
    drop(tx);
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn the_nightly_job_expires_alexs_birthday_points_and_books_the_next_night() {
    let operator = operator_cx().await;
    let e = ExpireLoyalty
        .preview(&operator, input(json!({})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "forbidden");
    let cx = system_cx().await;
    let today = ExpireLoyalty.preview(&cx, input(json!({}))).await.unwrap();
    assert!(
        today.plan.entries.is_empty(),
        "Alex's points are good through Nov 6"
    );
    let p = ExpireLoyalty
        .preview(&cx, input(json!({"as_of": "2026-11-07"})))
        .await
        .unwrap();
    assert_eq!(p.plan.entries.len(), 1);
    assert_eq!(p.plan.entries[0].amount, usd("1.50"));

    let mut tx = cx.begin().await.unwrap();
    tx.clock = Arc::new(FixedClock::at(at(&cx, "2026-11-07T00:00")));
    let JobOutcome::Done { result, events } =
        LoyaltyExpires.run(&mut tx, &json!({})).await.unwrap()
    else {
        panic!("a done job");
    };
    assert_eq!(result["expired"], 1, "{result:#}");
    assert_eq!(result["next_run_at"], "2026-11-08T00:00:00-05:00");
    assert_eq!(events[0].topic, "loyalty.expired");
    let booked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform.job WHERE tenant_id = $1 AND id = 'job_loyalty_expire_20261108'",
    )
    .bind(tx.tenant.as_str())
    .fetch_one(&mut *tx.tx)
    .await
    .unwrap();
    assert_eq!(booked, 1);
    // Running it again for the same day expires nothing more.
    let JobOutcome::Done { result, .. } = LoyaltyExpires.run(&mut tx, &json!({})).await.unwrap()
    else {
        panic!("a done job");
    };
    assert_eq!(result["expired"], 0);
    drop(tx);
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn used_coupons_come_back_and_unused_ones_can_be_revoked() {
    stand_ins();
    let cx = operator_cx().await;
    let p = RestoreCoupon
        .preview(
            &cx,
            input(json!({"coupon_id": "cpn_birthday", "order_id": "ord_20352", "scope": "full"})),
        )
        .await
        .unwrap();
    assert_eq!(
        (p.changes[0].from.clone(), p.changes[0].to.clone()),
        (json!("used"), json!("restored"))
    );
    // Expired on Oct 31: one grace day from today.
    assert_eq!(
        cx.time.format(p.plan.owned_through_after.unwrap()),
        "2026-11-07T23:59:00-05:00"
    );
    let partial = RestoreCoupon
        .preview(
            &cx,
            input(
                json!({"coupon_id": "cpn_birthday", "order_id": "ord_20352", "scope": "partial"}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(partial.changes[0].op, ChangeOp::Exclude);
    let mut tx = cx.begin().await.unwrap();
    let done = RestoreCoupon.commit(&mut tx, &p.plan).await.unwrap();
    assert_eq!(done.id, "mcp_elena_birthday");
    RestoreCoupon.undo(&mut tx, &p.plan, &done).await.unwrap();
    drop(tx);

    let e = RevokeCoupon
        .preview(&cx, input(json!({"coupon_id": "cpn_birthday", "revoke_mode": "member_coupon", "member_coupon_id": "mcp_elena_birthday", "reason": "Test."})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "already_used");
    let all = RevokeCoupon
        .preview(&cx, input(json!({"coupon_id": "cpn_gold_10", "revoke_mode": "all_unused", "reason": "Issued twice."})))
        .await
        .unwrap();
    assert_eq!(all.changes[0].op, ChangeOp::Revoke);
    assert!(all.plan.items.len() > 2000, "{}", all.plan.items.len());

    // Redeeming checks the member's tier against the coupon's.
    let e = RedeemCoupon
        .preview(
            &cx,
            input(json!({"coupon_id": "cpn_gold_10", "order_id": "ord_test_jamie"})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_eligible");
    let e = RedeemCoupon
        .preview(
            &cx,
            input(json!({"coupon_id": "cpn_gold_10", "order_id": "ord_missing"})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_found");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn create_and_update_promotion_check_overlaps_and_running_promotions() {
    stand_ins();
    let cx = operator_cx().await;
    let p = CreatePromotion
        .preview(
            &cx,
            input(
                json!({"name": "$10 off $120", "promotion_type": "threshold",
            "discount_layer": "order_discount", "amount_off": {"value": 10.00, "currency": "USD"},
            "threshold": {"value": 120.00, "currency": "USD"}, "threshold_benefit": "amount_off",
            "valid_from": "2026-11-20T00:00", "valid_through": "2026-11-30T23:59",
            "scope_mode": "order", "cost_share": {"store": 1.0, "seller": 0.0}}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        p.approval,
        ApprovalNeed::Needed {
            reason: "New order discount for everyone, Nov 20–30".into()
        }
    );
    let check = |rule: &str| p.checks.iter().find(|c| c.rule == rule).cloned().unwrap();
    assert_eq!(
        check("max_discount").detail.as_deref(),
        Some("At most 8.3% on a $120.00 order; limit 20%")
    );
    // The standing birthday and Gold order discounts reach the same members in that period.
    let overlap = check("promotion_overlap");
    assert!(
        !overlap.passed
            && overlap
                .detail
                .as_deref()
                .unwrap()
                .contains("Birthday coupon"),
        "{overlap:?}"
    );
    let e = CreatePromotion
        .preview(&cx, input(json!({"name": "x", "promotion_type": "amount_off", "discount_layer": "order_discount",
            "amount_off": {"value": 10.00, "currency": "USD"}, "valid_from": "2026-11-20T00:00",
            "valid_through": "2026-11-30T23:59", "scope_mode": "order", "cost_share": {"store": 0.5, "seller": 0.4}})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "cost_share_sum");
    let e = CreatePromotion
        .preview(
            &cx,
            input(
                json!({"name": "x", "promotion_type": "amount_off", "discount_layer": "item_price",
            "amount_off": {"value": 10.00, "currency": "USD"}, "valid_from": "2026-11-20T00:00",
            "valid_through": "2026-11-30T23:59", "scope_mode": "order", "percent_off": 20}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(
        (e.code.as_ref(), e.next_tool.as_deref()),
        ("percent_off_for_tier", Some("set_sale_prices"))
    );

    let u = UpdatePromotion
        .preview(
            &cx,
            input(
                json!({"promotion_id": "promo_denim_bundle", "valid_through": "2026-11-22T23:59"}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        u.changes[0].to["valid_through"],
        "2026-11-22T23:59:00-05:00"
    );
    assert_eq!(u.approval, ApprovalNeed::NotNeeded);
    let mut tx = cx.begin().await.unwrap();
    let done = UpdatePromotion.commit(&mut tx, &u.plan).await.unwrap();
    UpdatePromotion.undo(&mut tx, &u.plan, &done).await.unwrap();
    drop(tx);
    for (id, change) in [
        (
            "promo_linen_clearance",
            json!({"name": "Linen clearance 2"}),
        ),
        (
            "promo_autumn_knit",
            json!({"valid_from": "2026-11-02T00:00"}),
        ),
        ("promo_autumn_knit", json!({"category_ids": ["cat_denim"]})),
    ] {
        let mut i = change;
        i["promotion_id"] = json!(id);
        let e = UpdatePromotion.preview(&cx, input(i)).await.unwrap_err();
        assert_eq!(e.code, "invalid_state", "{id}");
    }
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn gifts_and_segments() {
    stand_ins();
    let cx = operator_cx().await;
    let gifts = ListConditionalGifts
        .read(&cx, input(json!({"status": ["scheduled"]})))
        .await
        .unwrap();
    assert_eq!(ids(&gifts), ["cgf_holiday_120"]);
    let e = CreateConditionalGift
        .preview(&cx, input(json!({"name": "x", "tiers": [{"trigger_threshold": {"value": 150.00, "currency": "USD"}, "gift_product_id": "prd_nothing"}],
            "valid_from": "2026-12-01T00:00", "valid_through": "2026-12-10T23:59"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_found");
    let p = CreateConditionalGift
        .preview(&cx, input(json!({"name": "December gift over $150", "tiers": [{"trigger_threshold": {"value": 150.00, "currency": "USD"}, "gift_product_id": "prd_merino_tee"}],
            "valid_from": "2026-12-01T00:00", "valid_through": "2026-12-10T23:59"})))
        .await
        .unwrap();
    assert_eq!(p.plan.id, "cgf_december_gift_over_150");
    let mut tx = cx.begin().await.unwrap();
    let done = CreateConditionalGift
        .commit(&mut tx, &p.plan)
        .await
        .unwrap();
    CreateConditionalGift
        .undo(&mut tx, &p.plan, &done)
        .await
        .unwrap();
    drop(tx);

    let list = ListSegments.read(&cx, input(json!({}))).await.unwrap();
    assert_eq!(list["total"], 2);
    let vip = PreviewSegment
        .read(
            &cx,
            input(json!({"condition_rules": {"member_tiers": ["vip"]}})),
        )
        .await
        .unwrap();
    assert_eq!(vip["estimated_size"], 1284, "{vip:#}");
    assert_eq!(vip["by_tier"], json!({"vip": 1284}));
    let recent = PreviewSegment
        .read(&cx, input(json!({"segment_id": "seg_recent_buyers"})))
        .await
        .unwrap();
    assert_eq!(
        recent["sample_customer_ids"],
        json!(["cus_alex", "cus_jamie"])
    );
    let e = PreviewSegment
        .read(
            &cx,
            input(json!({"condition_rules": {"member_tiers": ["platinum"]}})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "tier_not_found");
    let again = DefineSegment
        .preview(
            &cx,
            input(
                json!({"name": "Recent buyers", "condition_rules": {"last_order_within_days": 14}}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(again.changes[0].op, ChangeOp::Update);
    assert_eq!(again.changes[0].to["version"], 2);
    let new = DefineSegment
        .preview(&cx, input(json!({"name": "Big spenders", "condition_rules": {"min_spend": {"value": 500.00, "currency": "USD"}}})))
        .await
        .unwrap();
    assert_eq!(
        (new.plan.segment_id.as_str(), new.plan.estimated_size),
        ("seg_big_spenders", 1)
    );
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn event_reads_and_the_raffle_draw_repeats_from_its_seed() {
    let cx = operator_cx().await;
    let types = ListEventTypes.read(&cx, input(json!({}))).await.unwrap();
    assert_eq!(
        (
            types["total"].clone(),
            types["allowed_combinations"].clone()
        ),
        (json!(12), json!("any"))
    );
    let rewards = ListRewardTypes.read(&cx, input(json!({}))).await.unwrap();
    assert_eq!(rewards["total"], 7);
    let found = SearchEvents
        .read(&cx, input(json!({"status": ["scheduled"]})))
        .await
        .unwrap();
    assert_eq!(ids(&found), ["evt_0311"]);
    let raffle = GetEvent
        .read(&cx, input(json!({"event_id": "evt_1001"})))
        .await
        .unwrap();
    assert_eq!(raffle["entrant_count"], 5);
    assert_eq!(raffle["draw_record_ids"], json!(["drw_1001_01"]));
    assert_eq!(raffle["rewards"]["dispatched"], 1);
    let record = GetDrawRecord
        .read(
            &cx,
            input(json!({"event_id": "evt_1001", "draw_record_id": "drw_1001_01"})),
        )
        .await
        .unwrap();
    // Anyone with the seed and the entrants gets the same winners.
    let pool: Vec<String> = [
        "ent_1001_alex",
        "ent_1001_elena",
        "ent_1001_jamie",
        "ent_1001_noor",
        "ent_1001_theo",
    ]
    .map(str::to_owned)
    .to_vec();
    let order = seeded_order(record["seed"].as_str().unwrap(), 1, &pool);
    assert_eq!(record["results"][0]["winners"], json!([order[0]]));
    assert_eq!(record["results"][0]["waitlist"], json!([order[1]]));
    assert_eq!(record["results"][1]["winners"], json!([order[2]]));
    assert_eq!(
        record["entrant_snapshot_digest"],
        json!(crate::event::snapshot_digest(&pool))
    );
    assert_eq!(
        record["seed_commitment"],
        json!(crate::event::sha256(record["seed"].as_str().unwrap()))
    );
    let public = GetDrawRecord
        .read(
            &shopper_cx("cus_jamie").await,
            input(json!({"event_id": "evt_1001", "draw_record_id": "drw_1001_01"})),
        )
        .await
        .unwrap();
    assert!(
        public.get("seed").is_none() && public["method"] == "seeded_shuffle",
        "{public:#}"
    );
    let mine = ListMyEntries
        .read(&shopper_cx("cus_jamie").await, input(json!({})))
        .await
        .unwrap();
    assert_eq!(mine["items"][0]["reward"]["reward_status"], "dispatched");
    let e = ListMyEntries
        .read(&shopper_cx("ses_guest").await, input(json!({})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "forbidden");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn events_are_made_entered_drawn_and_rewarded() {
    stand_ins();
    let cx = operator_cx().await;
    let spec = json!({"name": "Fall check-in week", "start_date": "2026-11-09T00:00", "end_date": "2026-11-15T23:59",
        "entry_mechanic": "check_in", "selection_method": "random_draw",
        "reward_tiers": [{"rank": 1, "reward_type": "product", "reward_ref": "prd_wool_overcoat", "reward_value": {"value": 289.00, "currency": "USD"}, "winner_count": 1, "waitlist_count": 2},
                         {"rank": 2, "reward_type": "loyalty_points", "reward_value": {"value": 5.00, "currency": "USD"}, "winner_count": 100}],
        "eligibility": {"valid_for_member_tier": "everyone"}, "entry_limit": {"count": 1, "period": "day"},
        "schedule": {"draw_at": "2026-11-16T10:00"}});
    let p = CreateEvent.preview(&cx, input(spec.clone())).await.unwrap();
    assert!(p.checks.iter().all(|c| c.passed), "{:#?}", p.checks);
    assert_eq!(p.approval, ApprovalNeed::NotNeeded);
    let mut tx = cx.begin().await.unwrap();
    let done = CreateEvent.commit(&mut tx, &p.plan).await.unwrap();
    let made = crate::event::event(&mut tx, &done.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(made.seed_commitment, crate::event::sha256(&made.draw_seed));
    CreateEvent.undo(&mut tx, &p.plan, &done).await.unwrap();
    assert!(
        crate::event::event(&mut tx, &done.id)
            .await
            .unwrap()
            .is_none()
    );
    drop(tx);
    let mut bad = spec.clone();
    bad["entry_mechanic"] = json!("lottery");
    assert_eq!(
        CreateEvent.preview(&cx, input(bad)).await.unwrap_err().code,
        "type_disabled"
    );
    let mut odds = spec.clone();
    odds["selection_method"] = json!("instant_win");
    odds["reward_tiers"][0]["odds"] = json!(0.6);
    odds["reward_tiers"][1]["odds"] = json!(0.5);
    assert_eq!(
        CreateEvent
            .preview(&cx, input(odds))
            .await
            .unwrap_err()
            .code,
        "odds_exceed_one"
    );
    let mut late = spec;
    late["schedule"] = json!({"draw_at": "2026-11-14T10:00"});
    assert_eq!(
        CreateEvent
            .preview(&cx, input(late))
            .await
            .unwrap_err()
            .code,
        "invalid_schedule"
    );

    let paused = UpdateEvent
        .preview(
            &cx,
            input(json!({"event_id": "evt_0311", "status": "paused"})),
        )
        .await
        .unwrap();
    assert_eq!(
        (paused.changes[0].from.clone(), paused.changes[0].to.clone()),
        (json!("scheduled"), json!("paused"))
    );
    let e = UpdateEvent
        .preview(
            &cx,
            input(json!({"event_id": "evt_1001", "name": "Renamed"})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_state");

    // Entries: closed before Nov 9; on Nov 10 Jamie may enter once that day.
    let jamie = shopper_cx("cus_jamie").await;
    let consent = json!({"entry_terms": true, "winner_contact": true});
    let e = EnterEvent
        .preview(
            &jamie,
            input(json!({"event_id": "evt_0311", "entry_consent": consent})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "event_not_active");
    let mut tuesday = shopper_cx("cus_jamie").await;
    tuesday.clock = Arc::new(FixedClock::at(at(&cx, "2026-11-10T09:00")));
    let e = EnterEvent
        .preview(
            &tuesday,
            input(json!({"event_id": "evt_0311", "entry_consent": {"entry_terms": true}})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "mandatory_consent_missing");
    let entry = EnterEvent
        .preview(
            &tuesday,
            input(json!({"event_id": "evt_0311", "entry_consent": consent})),
        )
        .await
        .unwrap();
    assert_eq!(entry.plan.bucket, "2026-11-10");
    let mut tx = cx.begin().await.unwrap();
    tx.clock = tuesday.clock.clone();
    let done = EnterEvent.commit(&mut tx, &entry.plan).await.unwrap();
    assert_eq!(done.produced["entry_status"], "entered");
    drop(tx);

    // Draws: not before entries close; a cancelled win needs a reason.
    let e = RunDraw
        .preview(&cx, input(json!({"event_id": "evt_0311", "mode": "draw"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "entries_open");
    let e = RunDraw
        .preview(&cx, input(json!({"event_id": "evt_1001", "mode": "cancel_winner", "entry_ids": ["ent_1001_alex"]})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "reason_required");
    let cancel = RunDraw
        .preview(&cx, input(json!({"event_id": "evt_1001", "mode": "cancel_winner", "entry_ids": ["ent_1001_jamie"], "reason": "Could not be reached."})))
        .await
        .unwrap();
    assert_eq!(
        cancel.approval,
        ApprovalNeed::Needed {
            reason: "Cancels 1 win".into()
        }
    );
    let mut tx = cx.begin().await.unwrap();
    RunDraw.commit(&mut tx, &cancel.plan).await.unwrap();
    let after: Vec<_> = crate::event::entries(&mut tx, "evt_1001").await.unwrap();
    assert!(
        after
            .iter()
            .any(|e| e.id == "ent_1001_jamie" && e.entry_status == "cancelled")
    );
    drop(tx);

    // Alex's merino tee is the one reward left to send.
    let send = DispatchReward
        .preview(&cx, input(json!({"event_id": "evt_1001"})))
        .await
        .unwrap();
    assert_eq!(
        send.approval,
        ApprovalNeed::Needed {
            reason: "Sends rewards to 1 winner worth $58.00".into()
        }
    );
    assert_eq!(
        send.plan.rewards[0].fulfillment_ref.as_deref(),
        Some("stub_shipment:adr_alex_home")
    );
    let mut tx = cx.begin().await.unwrap();
    DispatchReward.commit(&mut tx, &send.plan).await.unwrap();
    let e = crate::event::event(&mut tx, "evt_1001")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(e.event_status, "completed");
    drop(tx);
}
