//! Contract tests for the partner operations.
//!
//! Each operation's file tests what is refused before anything is read (a context whose pool
//! never connects). The database tests here (`#[ignore]`) run against a migrated database loaded
//! with the demo store (`seed/demo/`, `11_partner.yml`): `DATABASE_URL` (service role) and
//! `MIGRATION_DATABASE_URL` (owner, for the vendor-staff fixture), as in `.env.example`:
//! `cargo test -p acom-partner -- --ignored --test-threads=1`. Every run of a write happens in a
//! transaction that is rolled back, and the one committed fixture is removed again.

use std::collections::BTreeSet;
use std::sync::Arc;

use acom_auth::{Principal, Surface};
use acom_contract::{ApprovalNeed, Job, ReadOp, WriteOp};
use acom_core::enums::ActorKind;
use acom_core::{FixedClock, StoreTime, TenantId};
use acom_store::{Cx, Db};
use serde_json::{Value, json};

use crate::contract::{GetContract, SetContract};
use crate::dashboard::GetSellerOverview;
use crate::grade::{GetSellerGrade, SetSellerGrade};
use crate::jobs::ChangeSellerStatus;
use crate::rules::tests::policy;
use crate::seller::{GetSeller, OnboardSeller, SearchSellers, SetSellerStatus, UpdateSeller};
use crate::{repo, viewer};

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

/// A context for the demo store at a store time, as `subject`.
pub(crate) fn cx_on(db: Db, subject: &str, at: &str) -> Cx {
    let time = StoreTime::new(chrono_tz::America::New_York);
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
        "2026-11-06T11:00",
    )
}

pub(crate) fn input<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

async fn db(var: &str) -> Db {
    let url = std::env::var(var).unwrap_or_else(|_| panic!("{var}"));
    Db::connect(&url, 2).await.expect("connect")
}

/// The demo store at the scene time, as SJ (a store operator).
/// Runs one statement and commits it (a fixture that reads in other transactions must see).
async fn run_sql(cx: &Cx, sql: &'static str) {
    let mut tx = cx.begin().await.unwrap();
    sqlx::query(sql)
        .execute(&mut *tx.tx)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
    tx.commit().await.unwrap();
}

async fn store_cx() -> Cx {
    cx_on(db("DATABASE_URL").await, "adm_sj", "2026-11-06T11:00")
}

#[test]
fn every_operation_is_registered_once_in_spec_order() {
    let mut b = acom_contract::RegistryBuilder::<Cx>::default();
    crate::register(&mut b);
    let r = b.build().unwrap_or_else(|e| panic!("{e:#?}"));
    let tools: Vec<&str> = r.iter().map(|o| o.meta().tool).collect();
    assert_eq!(tools.len(), 10);
    for op in r.iter() {
        let m = op.meta();
        assert_eq!(m.domain(), "partner");
        assert!(m.address.ends_with(m.tool), "{}", m.address);
    }
    assert!(r.job("partner.change_seller_status").is_some());
}

// ── database tests ───────────────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn search_finds_sellers_by_name_tax_id_and_grade_with_tax_ids_masked() {
    let cx = store_cx().await;
    let all = SearchSellers
        .read(&cx, input(json!({"sort": "name"})))
        .await
        .unwrap();
    assert_eq!(all["total"], 3);
    assert_eq!(all["items"][0]["id"], "slr_fieldstone");
    assert_eq!(all["items"][1]["id"], "slr_harbor_goods");
    assert_eq!(all["items"][1]["tax_id"], "***-**-4410");
    let by_tax = SearchSellers
        .read(&cx, input(json!({"query": "4410"})))
        .await
        .unwrap();
    assert_eq!(by_tax["total"], 1);
    assert_eq!(by_tax["items"][0]["legal_name"], "Harbor Goods LLC");
    let gold = SearchSellers
        .read(&cx, input(json!({"acom:grade_level": "gold"})))
        .await
        .unwrap();
    assert_eq!(gold["total"], 1);
    assert_eq!(gold["items"][0]["id"], "slr_ridgeline");
    let review = SearchSellers
        .read(
            &cx,
            input(json!({"acom:seller_status": "under_review", "limit": 1})),
        )
        .await
        .unwrap();
    assert_eq!(review["items"][0]["id"], "slr_harbor_goods");
    assert!(review.get("next_cursor").is_none());
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn get_seller_shows_the_contract_in_force_the_grade_and_the_products() {
    let cx = store_cx().await;
    let s = GetSeller
        .read(&cx, input(json!({"seller_id": "slr_fieldstone"})))
        .await
        .unwrap();
    assert_eq!(s["legal_name"], "Fieldstone Boot Co.");
    assert_eq!(s["tax_id"], "**-***8840");
    assert_eq!(s["acom:contract_summary"]["contract_id"], "ctr_fieldstone");
    assert_eq!(s["acom:contract_summary"]["version"], 2);
    assert_eq!(s["acom:grade_level"], "silver");
    assert_eq!(s["acom:products"][0]["product_id"], "prd_chelsea_boots");
    let small = GetSeller
        .read(
            &cx,
            input(json!({"seller_id": "slr_harbor_goods", "fields": ["acom:seller_status"]})),
        )
        .await
        .unwrap();
    assert_eq!(
        small,
        json!({"id": "slr_harbor_goods", "acom:seller_status": "under_review"})
    );
    let e = GetSeller
        .read(&cx, input(json!({"seller_id": "slr_nobody"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "seller_not_found");
    assert_eq!(e.next_tool.as_deref(), Some("search_sellers"));
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_vendor_organizations_operator_acts_for_its_seller() {
    let owner = cx_on(
        db("MIGRATION_DATABASE_URL").await,
        "adm_sj",
        "2026-11-06T11:00",
    );
    let remove = "DELETE FROM platform.admin_user
                  WHERE tenant_id = 'store_northwind' AND id = 'adm_t9_fieldstone'";
    run_sql(&owner, remove).await;
    run_sql(
        &owner,
        "INSERT INTO platform.admin_user (tenant_id, id, name, email, admin_status, organization_id)
         VALUES ('store_northwind', 'adm_t9_fieldstone', 'Marta Lindqvist', 'marta@fieldstone.example',
                 'active', 'org_fieldstone')",
    )
    .await;
    let cx = cx_on(
        db("DATABASE_URL").await,
        "adm_t9_fieldstone",
        "2026-11-06T11:00",
    );
    let mine = SearchSellers.read(&cx, input(json!({}))).await;
    let own = GetSeller
        .read(&cx, input(json!({"seller_id": "slr_fieldstone"})))
        .await;
    let other = GetSeller
        .read(&cx, input(json!({"seller_id": "slr_ridgeline"})))
        .await;
    let contract = GetContract
        .read(&cx, input(json!({"seller_id": "slr_fieldstone"})))
        .await;
    let overview = GetSellerOverview
        .read(
            &cx,
            input(json!({"seller_id": "slr_fieldstone", "fields": ["sales"]})),
        )
        .await;
    let status = SetSellerStatus
        .preview(
            &cx,
            input(
                json!({"seller_id": "slr_fieldstone", "acom:target_status": "suspended",
                         "reason": "Self-suspension"}),
            ),
        )
        .await;
    run_sql(&owner, remove).await;

    let mine = mine.unwrap();
    assert_eq!(mine["total"], 1);
    assert_eq!(mine["items"][0]["id"], "slr_fieldstone");
    assert_eq!(own.unwrap()["tax_id"], "47-2218840");
    assert_eq!(other.unwrap_err().code, "forbidden");
    assert_eq!(
        contract.unwrap()["acom:payout_account"],
        "pa_fieldstone_checking_7731"
    );
    assert_eq!(overview.unwrap()["acom:role"], "seller");
    assert_eq!(status.unwrap_err().code, "forbidden");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn get_contract_returns_the_version_in_force_and_the_history() {
    let cx = store_cx().await;
    let c = GetContract
        .read(&cx, input(json!({"seller_id": "slr_fieldstone"})))
        .await
        .unwrap();
    assert_eq!(c["id"], "ctr_fieldstone");
    assert_eq!(c["version"], 2);
    assert_eq!(c["acom:contract_status"], "active");
    let payout = c["acom:payout_account"].as_str().unwrap();
    assert!(
        payout.starts_with("****") && payout.ends_with("7731"),
        "{payout}"
    );
    assert_eq!(
        c["acom:commission_rate"],
        json!([{"category_id": null, "rate": 0.12}, {"category_id": "cat_shoes", "rate": 0.11}])
    );
    assert_eq!(c["acom:versions"].as_array().unwrap().len(), 2);
    let v1 = GetContract
        .read(
            &cx,
            input(json!({"seller_id": "slr_fieldstone", "version": 1})),
        )
        .await
        .unwrap();
    assert_eq!(v1["acom:contract_status"], "ended");
    let e = GetContract
        .read(&cx, input(json!({"seller_id": "slr_harbor_goods"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "contract_not_found");
    assert_eq!(e.next_tool.as_deref(), Some("set_contract"));
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn the_grade_rests_on_last_months_orders_parcels_and_claims() {
    let cx = store_cx().await;
    let g = GetSellerGrade
        .read(&cx, input(json!({"seller_id": "slr_ridgeline"})))
        .await
        .unwrap();
    assert_eq!(
        g["acom:graded_period"],
        json!({"valid_from": "2026-10-01", "valid_through": "2026-10-31"})
    );
    assert_eq!(g["acom:grade_level"], "gold");
    assert_eq!(g["acom:next_review_date"], "2026-12-01");
    assert!(g["acom:basis"]["order_count"].as_i64().unwrap() > 100);
    assert!(g["acom:shipping_compliance_rate"].is_number());
    assert_eq!(g["acom:breakdown"].as_array().unwrap().len(), 1);
    let days = GetSellerGrade
        .read(
            &cx,
            input(
                json!({"seller_id": "slr_fieldstone", "valid_from": "2026-10-01",
                         "valid_through": "2026-11-05", "acom:granularity": "day"}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(days["acom:breakdown"].as_array().unwrap().len(), 36);
    let e = GetSellerGrade
        .read(&cx, input(json!({"seller_id": "slr_harbor_goods"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "no_data_for_period");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn the_overview_reads_sales_claims_and_the_payout_estimate() {
    let cx = store_cx().await;
    let o = GetSellerOverview
        .read(&cx, input(json!({"seller_id": "slr_fieldstone"})))
        .await
        .unwrap();
    assert_eq!(
        o["period"],
        json!({"valid_from": "2026-10-07", "valid_through": "2026-11-05"})
    );
    assert_eq!(o["acom:role"], "operator");
    assert!(o["sales"]["order_count"].as_i64().unwrap() >= 1);
    assert_eq!(o["sales"]["revenue"]["currency"], "USD");
    // Alex's boots (#20402) are back in a return request.
    assert!(o["claims"]["open_claims"].as_i64().unwrap() >= 1);
    assert_eq!(o["settlement"]["acom:settlement_cycle"], "monthly");
    assert_eq!(o["settlement"]["next_payout_date"], "2026-11-15");
    assert_eq!(o["grade"]["acom:grade_level"], "silver");
    assert!(o["tasks"]["unanswered_inquiries"].is_number());
    let r = GetSellerOverview
        .read(
            &cx,
            input(
                json!({"seller_id": "slr_ridgeline", "period": "last_7_days",
                         "fields": ["settlement"]}),
            ),
        )
        .await
        .unwrap();
    assert!(r.get("sales").is_none());
    assert_eq!(r["settlement"]["acom:settlement_cycle"], "biweekly");
    assert!(r["settlement"]["pending_amount"]["value"].is_number());
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn onboarding_previews_the_checks_and_refuses_a_known_tax_id() {
    let cx = store_cx().await;
    let application = json!({
        "legal_name": "Tidewater Bags LLC", "tax_id": "61-3302917",
        "acom:seller_type": "marketplace",
        "address": {"street_address": "9 Pier Rd", "address_locality": "Newport",
                    "address_region": "RI", "postal_code": "02840", "address_country": "US"},
        "contact_point": [{"contact_type": "settlement", "name": "Ada Brook"}],
        "acom:business_category": ["cat_bags"],
        "acom:documents": [
            {"document_type": "business_registration", "upload_id": "upl_t9_reg"},
            {"document_type": "payout_account_proof", "upload_id": "upl_t9_bank"}]
    });
    let p = OnboardSeller
        .preview(&cx, input(application.clone()))
        .await
        .unwrap();
    assert!(matches!(p.approval, ApprovalNeed::Needed { .. }));
    assert!(p.checks.iter().all(|c| c.passed), "{:?}", p.checks);
    assert_eq!(p.plan.seller_id, "slr_tidewater_bags");
    assert_eq!(p.changes[0].to["acom:seller_status"], "under_review");
    let mut tx = cx.begin().await.unwrap();
    let done = OnboardSeller.commit(&mut tx, &p.plan).await.unwrap();
    assert_eq!(done.id, "slr_tidewater_bags");
    let s = repo::seller(&mut tx, &done.id).await.unwrap().unwrap();
    assert_eq!(s.seller_status, "under_review");
    OnboardSeller.undo(&mut tx, &p.plan, &done).await.unwrap();
    assert!(repo::seller(&mut tx, &done.id).await.unwrap().is_none());
    drop(tx);

    let mut again = application;
    again["tax_id"] = json!("47 221 8840");
    let e = OnboardSeller.preview(&cx, input(again)).await.unwrap_err();
    assert_eq!(e.code, "duplicate_tax_id");
    assert_eq!(e.next_tool.as_deref(), Some("get_seller"));
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_profile_update_runs_and_undoes() {
    let cx = store_cx().await;
    let p = UpdateSeller
        .preview(
            &cx,
            input(json!({"seller_id": "slr_harbor_goods",
                         "acom:business_category": ["cat_bags", "cat_hats", "cat_socks"]})),
        )
        .await
        .unwrap();
    assert_eq!(p.approval, ApprovalNeed::NotNeeded);
    assert_eq!(p.checks[0].rule, "profile.editable_fields");
    assert!(p.checks.iter().all(|c| c.passed));
    let mut tx = cx.begin().await.unwrap();
    let done = UpdateSeller.commit(&mut tx, &p.plan).await.unwrap();
    let s = repo::seller(&mut tx, &done.id).await.unwrap().unwrap();
    assert_eq!(s.business_category.len(), 3);
    UpdateSeller.undo(&mut tx, &p.plan, &done).await.unwrap();
    let s = repo::seller(&mut tx, &done.id).await.unwrap().unwrap();
    assert_eq!(s.business_category, ["cat_bags", "cat_hats"]);
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn suspending_hides_the_products_and_the_undo_puts_them_back() {
    let cx = store_cx().await;
    let p = SetSellerStatus
        .preview(
            &cx,
            input(
                json!({"seller_id": "slr_fieldstone", "acom:target_status": "suspended",
                         "reason": "Three late shipments in a week"}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        p.approval,
        ApprovalNeed::Needed {
            reason: "Suspends a seller with 1 open order".into()
        }
    );
    assert_eq!(p.changes[0].to["impact"]["products_hidden"], 1);
    assert_eq!(p.changes[1].id, "prd_chelsea_boots");
    assert!(p.checks.iter().any(|c| c.rule == "suspend.hide_products"));
    let mut tx = cx.begin().await.unwrap();
    let done = SetSellerStatus.commit(&mut tx, &p.plan).await.unwrap();
    let s = repo::seller(&mut tx, "slr_fieldstone")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.seller_status, "suspended");
    let boots =
        acom_catalog::service::for_partner::products(&mut tx, &["prd_chelsea_boots".into()])
            .await
            .unwrap();
    assert_eq!(boots[0].approval_status, "draft");
    SetSellerStatus.undo(&mut tx, &p.plan, &done).await.unwrap();
    let s = repo::seller(&mut tx, "slr_fieldstone")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.seller_status, "active");
    let boots =
        acom_catalog::service::for_partner::products(&mut tx, &["prd_chelsea_boots".into()])
            .await
            .unwrap();
    assert_eq!(boots[0].approval_status, "active");
    drop(tx);

    let e = SetSellerStatus
        .preview(
            &cx,
            input(
                json!({"seller_id": "slr_fieldstone", "acom:target_status": "terminated",
                         "reason": "Contract ends"}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "open_orders_exist");
    assert_eq!(e.next_tool.as_deref(), Some("search_orders"));
    let e = SetSellerStatus
        .preview(
            &cx,
            input(
                json!({"seller_id": "slr_harbor_goods", "acom:target_status": "active",
                         "reason": "Skip approval"}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_state");
    let same = SetSellerStatus
        .preview(
            &cx,
            input(
                json!({"seller_id": "slr_fieldstone", "acom:target_status": "active",
                         "reason": "Retry"}),
            ),
        )
        .await
        .unwrap();
    assert!(same.plan.unchanged);
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_booked_status_change_runs_when_its_job_comes_due() {
    let cx = store_cx().await;
    let p = SetSellerStatus
        .preview(
            &cx,
            input(
                json!({"seller_id": "slr_harbor_goods", "acom:target_status": "approved",
                         "reason": "Review passed", "valid_from": "2026-11-09T09:00"}),
            ),
        )
        .await
        .unwrap();
    assert!(p.plan.scheduled_at.is_some());
    assert_eq!(p.changes[0].op, acom_contract::ChangeOp::Schedule);
    let mut tx = cx.begin().await.unwrap();
    SetSellerStatus.commit(&mut tx, &p.plan).await.unwrap();
    let waiting = repo::waiting_events(&mut tx, "slr_harbor_goods")
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1);
    let s = repo::seller(&mut tx, "slr_harbor_goods")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.seller_status, "under_review");
    let payload = json!({"seller_id": "slr_harbor_goods", "position": waiting[0].position});
    ChangeSellerStatus.run(&mut tx, &payload).await.unwrap();
    let s = repo::seller(&mut tx, "slr_harbor_goods")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s.seller_status, "approved");
    assert!(
        repo::waiting_events(&mut tx, "slr_harbor_goods")
            .await
            .unwrap()
            .is_empty()
    );
    // A second run of the same job does nothing.
    ChangeSellerStatus.run(&mut tx, &payload).await.unwrap();
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_contract_amendment_adds_a_version_and_overlaps_are_caught() {
    let cx = store_cx().await;
    let p = SetContract
        .preview(
            &cx,
            input(json!({"seller_id": "slr_fieldstone",
                         "acom:commission_rate": [{"category_id": "cat_shoes", "rate": 0.10}],
                         "acom:settlement_cycle": "monthly",
                         "acom:settlement_method": "on_confirmation",
                         "acom:payout_account_ref": "pa_fieldstone_checking_7731",
                         "valid_from": "2027-01-01", "valid_through": "2027-12-31T23:59"})),
        )
        .await
        .unwrap();
    assert!(p.checks.iter().all(|c| c.passed), "{:?}", p.checks);
    assert_eq!(
        p.checks[1].detail.as_deref(),
        Some("Version 2 ends the day before")
    );
    assert_eq!(p.changes[0].from["version"], 2);
    assert_eq!(p.changes[0].to["version"], 3);
    // The default carries over from version 2.
    assert_eq!(p.changes[0].to["default_rate"], 0.12);
    let mut tx = cx.begin().await.unwrap();
    let done = SetContract.commit(&mut tx, &p.plan).await.unwrap();
    assert_eq!(done.produced["version"], 3);
    assert_eq!(
        repo::versions(&mut tx, "ctr_fieldstone")
            .await
            .unwrap()
            .len(),
        3
    );
    SetContract.undo(&mut tx, &p.plan, &done).await.unwrap();
    assert_eq!(
        repo::versions(&mut tx, "ctr_fieldstone")
            .await
            .unwrap()
            .len(),
        2
    );
    drop(tx);

    let overlap = SetContract
        .preview(
            &cx,
            input(json!({"seller_id": "slr_fieldstone",
                         "acom:commission_rate": [{"rate": 0.45}],
                         "acom:settlement_cycle": "monthly",
                         "acom:settlement_method": "on_delivery",
                         "acom:payout_account_ref": "pa_fieldstone_checking_7731",
                         "valid_from": "2026-12-01", "valid_through": "2027-06-30T23:59"})),
        )
        .await
        .unwrap();
    assert!(!overlap.checks[0].passed);
    assert!(!overlap.checks[1].passed);
    let mut tx = cx.begin().await.unwrap();
    let e = SetContract
        .commit(&mut tx, &overlap.plan)
        .await
        .unwrap_err();
    assert_eq!(e.code, "commission_out_of_bounds");
    drop(tx);
    let e = SetContract
        .preview(
            &cx,
            input(json!({"seller_id": "slr_harbor_goods",
                         "acom:commission_rate": [{"rate": 0.15}],
                         "acom:settlement_cycle": "weekly",
                         "acom:settlement_method": "on_delivery",
                         "acom:payout_account_ref": "pa_harbor_checking_4410",
                         "valid_from": "2026-12-01", "valid_through": "2027-11-30T23:59"})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "seller_not_active");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_grade_other_than_the_computed_one_needs_a_reason() {
    let cx = store_cx().await;
    let base = json!({"seller_id": "slr_ridgeline", "valid_from": "2026-12-01",
                      "valid_through": "2026-12-31T23:59"});
    let mut confirm = base.clone();
    confirm["acom:grade_level"] = json!("watch");
    let p = SetSellerGrade
        .preview(&cx, input(confirm.clone()))
        .await
        .unwrap();
    let computed = p.plan.grade.computed_grade_level.clone().unwrap();
    assert_eq!(p.approval, ApprovalNeed::NotNeeded);
    if computed != "watch" {
        assert!(p.plan.needs_reason);
        let mut tx = cx.begin().await.unwrap();
        let e = SetSellerGrade.commit(&mut tx, &p.plan).await.unwrap_err();
        assert_eq!(e.code, "override_not_allowed");
        drop(tx);
    }
    confirm["acom:grade_level"] = json!(computed);
    let p = SetSellerGrade.preview(&cx, input(confirm)).await.unwrap();
    assert!(p.checks.iter().all(|c| c.passed), "{:?}", p.checks);
    let mut tx = cx.begin().await.unwrap();
    let done = SetSellerGrade.commit(&mut tx, &p.plan).await.unwrap();
    assert!(
        repo::find_grade(&mut tx, "slr_ridgeline", &p.plan.grade)
            .await
            .unwrap()
            .is_some()
    );
    SetSellerGrade.undo(&mut tx, &p.plan, &done).await.unwrap();
    assert!(
        repo::find_grade(&mut tx, "slr_ridgeline", &p.plan.grade)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn store_operators_are_store_viewers() {
    let cx = store_cx().await;
    let mut tx = cx.read().await.unwrap();
    assert_eq!(
        viewer::viewer(&cx, &mut tx).await.unwrap(),
        viewer::Viewer::Store
    );
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn integration_reads_a_sellers_status() {
    let cx = store_cx().await;
    let mut tx = cx.read().await.unwrap();
    let status = crate::service::for_integration::seller_status;
    assert_eq!(
        status(&mut tx, "slr_fieldstone").await.unwrap().as_deref(),
        Some("active")
    );
    assert_eq!(
        status(&mut tx, "slr_harbor_goods")
            .await
            .unwrap()
            .as_deref(),
        Some("under_review")
    );
    assert_eq!(status(&mut tx, "slr_nobody").await.unwrap(), None);
}
