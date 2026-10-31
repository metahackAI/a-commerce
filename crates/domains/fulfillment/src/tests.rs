//! Contract tests for every fulfillment operation.
//!
//! The first group runs without a database: a context whose pool never connects, and inputs
//! that are refused before any query. The second group (`#[ignore]`) replays scenes B2 and F5
//! against a migrated database loaded with the demo store (`seed/demo/`); run it on request
//! with `DATABASE_URL` (service role) and `MIGRATION_DATABASE_URL` (owner, for the F5 replay
//! that removes the seeded request first) set, as in `.env.example`:
//! `cargo test -p acom-fulfillment -- --ignored`.

use std::collections::BTreeSet;
use std::sync::Arc;

use acom_auth::{Principal, Surface};
use acom_contract::{PersonOp, ReadOp, WriteOp};
use acom_core::enums::ActorKind;
use acom_core::{FixedClock, StoreTime, TenantId};
use acom_store::{Cx, Db};
use serde_json::{Value, json};

use crate::claim::{
    ApproveClaim, CancelMyOrder, EvaluateReturnRequest, GetCancelOptions, GetClaim,
    GetReturnOptions, RejectClaim, SearchClaims, SearchReturnRequests, SendReturnRequest,
    WithdrawClaim,
};
use crate::model::tests::policy;
use crate::parcel_delivery::{GetDelivery, GetDeliveryDates, TrackShipment};
use crate::refund::{GetRefund, RefundReturn};

mod operations;

pub(crate) const NY: chrono_tz::Tz = chrono_tz::America::New_York;

pub(crate) fn principal(subject: &str) -> Principal {
    let (kind, surface) = if subject.starts_with("cus_") {
        (ActorKind::Shopper, Surface::Fo)
    } else {
        (ActorKind::Operator, Surface::Bo)
    };
    Principal {
        subject: subject.to_owned(),
        kind,
        tenant: TenantId::parse("store_northwind").unwrap(),
        surface,
        scopes: BTreeSet::new(),
        name: None,
        session_id: None,
    }
}

/// A context for the demo store at a scene time, as `subject`.
pub(crate) fn cx_on(db: Db, subject: &str, at: &str) -> Cx {
    let time = StoreTime::new(NY);
    Cx {
        tenant: TenantId::parse("store_northwind").unwrap(),
        principal: Arc::new(principal(subject)),
        request_id: "req_test".into(),
        clock: Arc::new(FixedClock::at(time.parse(at).unwrap())),
        time,
        currency: acom_core::Currency::parse("USD").unwrap(),
        policy: Arc::new(policy()),
        db,
    }
}

/// A context whose pool never connects: for inputs refused before any query.
pub(crate) fn offline(subject: &str) -> Cx {
    cx_on(
        Db::connect_lazy("postgres://offline.invalid/acom", 1).unwrap(),
        subject,
        "2026-11-06T10:40",
    )
}

pub(crate) fn input<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
async fn search_return_requests_refuses_a_cursor_it_did_not_make() {
    let e = SearchReturnRequests
        .read(&offline("adm_sj"), input(json!({"cursor": "c_2"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_input");
}

#[tokio::test]
async fn search_claims_refuses_a_backwards_period() {
    let e = SearchClaims
        .read(
            &offline("adm_sj"),
            input(json!({"created_from": "2026-11-06", "created_through": "2026-11-01"})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_filter");
    assert_eq!(e.next_tool.as_deref(), Some("search_claims"));
    let e = SearchClaims
        .read(
            &offline("adm_sj"),
            input(json!({"created_from": "last week"})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_filter");
}

#[tokio::test]
async fn reject_claim_needs_a_reason() {
    let e = RejectClaim
        .preview(&offline("adm_sj"), input(json!({"claim_id": "clm_20290"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "reason_required");
    assert_eq!(e.next_tool.as_deref(), Some("reject_claim"));
}

#[tokio::test]
async fn approve_claim_refuses_a_charge_against_store_fault() {
    let e = ApproveClaim
        .preview(
            &offline("adm_sj"),
            input(
                json!({"claim_ids": ["clm_4410"], "liability": "store_fault",
                         "cost_override": {"amount": {"value": 6.00, "currency": "USD"},
                                           "note": "Shipping"}}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "liability_cost_conflict");
    let e = ApproveClaim
        .preview(
            &offline("adm_sj"),
            input(json!({"claim_ids": ["clm_4410"],
                         "cost_override": {"amount": {"value": 0.00, "currency": "USD"},
                                           "note": " "}})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_input");
}

#[tokio::test]
async fn get_delivery_dates_reads_the_by_date_in_store_time() {
    let e = GetDeliveryDates
        .read(
            &offline("cus_jamie"),
            input(
                json!({"product_id": "prd_windbreaker", "address_mode": "saved",
                         "address_id": "adr_home", "by": "next friday"}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_input");
}

#[tokio::test]
async fn track_shipment_refuses_a_cursor_it_did_not_make() {
    let e = TrackShipment
        .read(
            &offline("cus_alex"),
            input(json!({"mode": "delivery", "delivery_id": "dlv_20402", "cursor": "x"})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_input");
}

#[test]
fn inputs_reject_unknown_fields_and_bad_words() {
    assert!(
        serde_json::from_value::<crate::refund::refund_return::RefundReturnInput>(
            json!({"claim_id": "clm_20402", "amount": 1})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<crate::claim::get_return_options::GetReturnOptionsInput>(
            json!({"order_id": "ord_20402", "order_item_id": "oln_20402_1", "reason": "broken"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<crate::claim::send_return_request::SendReturnRequestInput>(
            json!({
                "order_id": "ord_20402", "items": [{"order_item_id": "oln_20402_1", "quantity": 1}],
                "reason": "defect", "kind": "refund", "send_back_method": "pickup",
                "refund_to": {"type": "store_credit", "ref": null}
            })
        )
        .is_ok()
    );
}

// ── Scenes B2 and F5 on the demo store (integration) ──────────────────────────

pub(crate) async fn db(var: &str) -> Db {
    let url = std::env::var(var).unwrap_or_else(|_| panic!("{var}"));
    Db::connect(&url, 2).await.expect("connect")
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn b2_queue_has_twelve_requests_over_200_and_the_policy_sorts_them() {
    let cx = cx_on(db("DATABASE_URL").await, "adm_sj", "2026-11-06T10:40");
    let queue = SearchReturnRequests
        .read(
            &cx,
            input(json!({"min_amount": {"value": 200.00, "currency": "USD"}, "limit": 100})),
        )
        .await
        .unwrap();
    assert_eq!(queue["total"], 12);
    let boots = queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["claim_id"] == "clm_20402")
        .unwrap();
    assert_eq!(boots["order_number"], "20402");
    assert_eq!(boots["photos"], 2);
    assert_eq!(boots["delivered_at"], "2026-10-31");
    assert_eq!(boots["days_since_delivery"], 6);
    assert_eq!(boots["amount"]["value"].to_string(), "312.00");

    let mut refunds = rust_decimal::Decimal::ZERO;
    let (mut refund_count, mut photos, mut declines) = (0, 0, 0);
    for item in queue["items"].as_array().unwrap() {
        let id = item["claim_id"].as_str().unwrap();
        let e = EvaluateReturnRequest
            .read(&cx, input(json!({"claim_id": id})))
            .await
            .unwrap();
        let s = &e["items"][0];
        match s["suggested"].as_str().unwrap() {
            "refund" => {
                refund_count += 1;
                refunds += s["refund_amount"]["value"]
                    .to_string()
                    .parse::<rust_decimal::Decimal>()
                    .unwrap();
            }
            "ask_for_photos" => photos += 1,
            _ => declines += 1,
        }
        if id == "clm_20431" {
            assert_eq!(s["flags"][0]["detail"], "4 returns in the last 90 days");
        }
    }
    assert_eq!((refund_count, photos, declines), (9, 2, 1));
    assert_eq!(refunds.to_string(), "2463.00");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn b2_refund_for_the_boots_needs_a_person_and_runs_once() {
    let cx = cx_on(db("DATABASE_URL").await, "adm_sj", "2026-11-06T10:40");
    let p = RefundReturn
        .preview(&cx, input(json!({"claim_id": "clm_20402"})))
        .await
        .unwrap();
    assert_eq!(
        p.approval,
        acom_contract::ApprovalNeed::Needed {
            reason: "Refund $312.00 is over the $200.00 limit for refunds without a person".into()
        }
    );
    assert_eq!(
        p.checks[0].detail.as_deref(),
        Some("$312.00 back to card ending 0937")
    );
    assert_eq!(
        p.checks[1].detail.as_deref(),
        Some("No coupon on order 20402")
    );
    let coupon = RefundReturn
        .preview(&cx, input(json!({"claim_id": "clm_20352"})))
        .await
        .unwrap();
    assert_eq!(coupon.plan.amount.value.to_string(), "256.00");
    // Run it in a transaction that is never committed.
    let mut tx = cx.as_system("sys_runner").begin().await.unwrap();
    let done = RefundReturn.commit(&mut tx, &p.plan).await.unwrap();
    let again = RefundReturn.commit(&mut tx, &p.plan).await.unwrap();
    assert_eq!(done, again);
    let status: String = sqlx::query_scalar(
        "SELECT claim_status FROM fulfillment.claim WHERE tenant_id = $1 AND id = 'clm_20402'",
    )
    .bind("store_northwind")
    .fetch_one(&mut *tx.tx)
    .await
    .unwrap();
    assert_eq!(status, "approved");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn b2_decline_and_pickup_previews() {
    let cx = cx_on(db("DATABASE_URL").await, "adm_sj", "2026-11-06T10:40");
    let p = RejectClaim
        .preview(
            &cx,
            input(json!({"claim_id": "clm_20290", "reason": "outside_return_window"})),
        )
        .await
        .unwrap();
    assert_eq!(
        p.checks[0].detail.as_deref(),
        Some("Delivered Sep 28, 39 days ago (limit 30)")
    );
    assert!(p.checks[0].passed);
    let a = ApproveClaim
        .preview(&cx, input(json!({"claim_ids": ["clm_20402"]})))
        .await
        .unwrap();
    assert_eq!(a.plan.steps[0].to_status, "pickup_instructed");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn claims_refunds_and_parcels_read_back() {
    let cx = cx_on(db("DATABASE_URL").await, "adm_sj", "2026-11-06T10:40");
    let c = GetClaim
        .read(&cx, input(json!({"claim_id": "clm_20402"})))
        .await
        .unwrap();
    assert_eq!(c["status"], "requested");
    assert_eq!(c["evidence"].as_array().unwrap().len(), 2);
    let done = SearchClaims
        .read(
            &cx,
            input(
                json!({"claim_type": "return", "status": ["completed"], "customer_id": "cus_4431"}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(done["total"], 3);
    let r = GetRefund
        .read(&cx, input(json!({"refund_id": "rfd_20298"})))
        .await
        .unwrap();
    assert_eq!(r["refund_status"], "completed");
    assert_eq!(r["to"], "card ending 7302");
    let alex = cx_on(db("DATABASE_URL").await, "cus_alex", "2026-11-06T09:50");
    let d = GetDelivery
        .read(&alex, input(json!({"delivery_id": "dlv_20402"})))
        .await
        .unwrap();
    assert_eq!(d["expected_arrival_until"], "2026-10-31");
    let t = TrackShipment
        .read(
            &alex,
            input(json!({"mode": "delivery", "delivery_id": "dlv_20402"})),
        )
        .await
        .unwrap();
    assert_eq!(t["items"][1]["location"], "Portland, ME");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn f2_windbreaker_arrives_tuesday_and_ships_free() {
    let cx = cx_on(db("DATABASE_URL").await, "cus_jamie", "2026-11-07T10:00");
    let d = GetDeliveryDates
        .read(
            &cx,
            input(
                json!({"product_id": "prd_windbreaker", "variant_id": "var_windbreaker_m_slate",
                         "address_mode": "saved", "address_id": "adr_home", "by": "2026-11-14"}),
            ),
        )
        .await
        .unwrap();
    let i = &d["items"][0];
    assert_eq!(i["ships_on"], "2026-11-07");
    assert_eq!(i["earliest_arrival"], "2026-11-10");
    assert_eq!(i["shipping"]["value"].to_string(), "0");
    assert_eq!(i["arrives_by_requested_date"], true);
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn f5_alex_sends_the_refund_request() {
    // F5 starts before the request exists: take the seeded one away inside a transaction that
    // is never committed (the owner role may delete).
    let cx = cx_on(
        db("MIGRATION_DATABASE_URL").await,
        "cus_alex",
        "2026-11-06T09:50",
    );
    let seeded = GetReturnOptions
        .read(
            &cx,
            input(json!({"order_id": "ord_20402", "order_item_id": "oln_20402_1", "reason": "defect"})),
        )
        .await
        .unwrap_err();
    assert_eq!(seeded.code, "claim_in_progress");
    let mut tx = cx.begin().await.unwrap();
    for table in ["claim_event", "claim_item"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM fulfillment.{table} WHERE tenant_id = 'store_northwind' AND claim_id = 'clm_20402'"
        )))
        .execute(&mut *tx.tx)
        .await
        .unwrap();
    }
    sqlx::query(
        "DELETE FROM fulfillment.claim WHERE tenant_id = 'store_northwind' AND id = 'clm_20402'",
    )
    .execute(&mut *tx.tx)
    .await
    .unwrap();
    let now = cx.clock.now();
    let view = crate::claim::return_view(&mut tx, "ord_20402", "oln_20402_1", None, now, NY)
        .await
        .unwrap();
    let rules = crate::rules(&cx.policy).unwrap();
    let (_, options) =
        crate::claim::options_for(&rules, &cx.policy, &view, "defect", None, now, &cx.time)
            .unwrap();
    assert_eq!(
        options[0].facts,
        ["Ships today", "Arrives Tue Nov 10", "No cost to you"]
    );
    assert_eq!(options[0].available_quantity, Some(6));
    assert_eq!(options[1].refund_to.as_deref(), Some("card ending 0937"));
    let sent = SendReturnRequest
        .run(
            &mut tx,
            input(json!({
                "order_id": "ord_20402",
                "items": [{"order_item_id": "oln_20402_1", "quantity": 1}],
                "reason": "defect", "kind": "refund",
                "reason_text": "The sole is coming apart after a week.",
                "evidence_ids": ["evd_877", "evd_878"],
                "send_back_method": "pickup",
                "pickup_window": {"start": "2026-11-09T09:00:00-05:00", "end": "2026-11-09T12:00:00-05:00"},
                "expected_refund": {"value": 312.00, "currency": "USD"}
            })),
        )
        .await
        .unwrap();
    assert_eq!(sent["status"], "requested");
    assert_eq!(sent["confirmed_by"], "store_person");
    assert_eq!(
        sent["refund_estimate"]["breakdown"][0]["to"],
        "card ending 0937"
    );
    let changed = SendReturnRequest
        .run(
            &mut tx,
            input(json!({
                "order_id": "ord_20402",
                "items": [{"order_item_id": "oln_20402_1", "quantity": 1}],
                "reason": "defect", "kind": "refund", "send_back_method": "drop_off",
                "expected_refund": {"value": 312.00, "currency": "USD"}
            })),
        )
        .await
        .unwrap_err();
    assert_eq!(changed.code, "claim_in_progress");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn shoppers_cannot_cancel_a_delivered_order_but_can_withdraw_a_return() {
    let cx = cx_on(db("DATABASE_URL").await, "cus_alex", "2026-11-06T10:40");
    let e = GetCancelOptions
        .read(&cx, input(json!({"order_id": "ord_20402"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "already_shipped");
    let mut tx = cx.begin().await.unwrap();
    let e = CancelMyOrder
        .run(
            &mut tx,
            input(json!({"order_id": "ord_20402", "reason": "changed_mind",
                         "expected_refund": {"value": 0.00, "currency": "USD"}})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "already_shipped");
    let w = WithdrawClaim
        .run(&mut tx, input(json!({"claim_id": "clm_20402"})))
        .await
        .unwrap();
    assert_eq!(w["status"], "withdrawn");
    let other = WithdrawClaim
        .run(&mut tx, input(json!({"claim_id": "clm_20366"})))
        .await
        .unwrap_err();
    assert_eq!(other.code, "not_your_order");
}
