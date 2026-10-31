//! Contract tests for every analytics operation. The first group needs no database: a pool that
//! never connects, and inputs refused before any query. The `#[ignore]` group reads the seeded
//! demo store at `DATABASE_URL` and reproduces scene B3 (docs/scenarios.md §6):
//! `cargo test -p acom-analytics -- --ignored --test-threads=1`. Those tests only read, so they
//! depend on nothing but the seed, and on no other test or order.

use std::collections::BTreeSet;
use std::sync::Arc;

use acom_auth::{Principal, Surface};
use acom_contract::{Kind, Method, Operation, ReadOp, RegistryBuilder, ToolError};
use acom_core::enums::ActorKind;
use acom_core::{FixedClock, StoreTime, TenantId};
use acom_policy::PolicySet;
use acom_store::{Cx, Db};
use serde_json::{Value, json};

use crate::customer::GetCustomerOverview;
use crate::sales::{CODES, ExplainSalesChange, GetSalesOverview, GetSalesTrend};
use crate::service::GetCsOverview;

fn policy() -> PolicySet {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
    let texts: Vec<(&str, String)> = acom_policy::FILES
        .iter()
        .map(|f| {
            (
                *f,
                std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
            )
        })
        .collect();
    PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str()))).unwrap()
}

/// Operator SJ at the B3 moment: Fri Nov 6 2026, 11:20 store time.
fn cx_on(db: Db) -> Cx {
    let time = StoreTime::new(chrono_tz::America::New_York);
    let tenant = TenantId::parse("store_northwind").unwrap();
    Cx {
        tenant: tenant.clone(),
        principal: Arc::new(Principal {
            subject: "adm_sj".to_owned(),
            kind: ActorKind::Operator,
            tenant,
            surface: Surface::Bo,
            scopes: BTreeSet::from(["tools:read:analytics".to_owned()]),
            name: None,
            session_id: None,
        }),
        request_id: "req_test".into(),
        clock: Arc::new(FixedClock::at(time.parse("2026-11-06T11:20").unwrap())),
        time,
        currency: acom_core::Currency::parse("USD").unwrap(),
        policy: Arc::new(policy()),
        db,
    }
}

fn offline() -> Cx {
    cx_on(Db::connect_lazy("postgres://offline.invalid/acom", 1).unwrap())
}

fn input<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

fn assert_code(e: &ToolError, code: &str, next: &str) {
    assert_eq!(e.code, code, "{e:?}");
    assert_eq!(e.next_tool.as_deref(), Some(next), "{e:?}");
}

#[test]
fn every_operation_is_a_closed_operator_read_matching_its_spec() {
    let mut b = RegistryBuilder::default();
    crate::register(&mut b);
    let registry = b.build().unwrap_or_else(|e| panic!("{e:#?}"));
    let tools: Vec<&str> = registry.iter().map(|op| op.meta().tool).collect();
    assert_eq!(tools.len(), 5, "{tools:?}");
    for op in registry.iter() {
        let m = op.meta();
        assert_eq!(m.kind, Kind::Read, "{}", m.tool);
        assert_eq!(
            m.method,
            Method::Post,
            "{}: aggregation input is a body",
            m.tool
        );
        assert_eq!(m.domain(), "analytics");
        assert!(m.path.starts_with("/v1/analytics/"), "{}", m.path);
        let schema = op.input_schema();
        assert_eq!(schema["additionalProperties"], false, "{}", m.tool);
        for (field, s) in schema["properties"].as_object().unwrap() {
            assert!(
                s.get("description").is_some(),
                "{}.{field} has no description",
                m.tool
            );
            assert!(
                s.get("examples").is_some(),
                "{}.{field} has no example",
                m.tool
            );
        }
        for e in m.errors {
            assert!(
                CODES.contains(e),
                "{}: {e} is not an analytics code",
                m.tool
            );
        }
    }
    let explain = registry.get("explain_sales_change").unwrap().input_schema();
    assert_eq!(
        explain["properties"]["compare_mode"]["enum"],
        json!(["same_weekday_last_4_weeks", "previous_period", "custom"])
    );
    assert_eq!(explain["properties"]["valid_from"]["format"], "date");
    assert_eq!(
        explain["properties"]["acom:metric"]["enum"],
        json!(["revenue", "order_count"])
    );
    assert_eq!(
        ExplainSalesChange::META.address,
        "analytics.sales.explain_sales_change"
    );
    assert_eq!(GetSalesTrend::META.path, "/v1/analytics/sales/trend");
}

#[tokio::test]
async fn explain_refuses_bad_periods_before_reading() {
    let cx = offline();
    let ask = |v: Value| ExplainSalesChange.read(&cx, input(v));
    let e = ask(
        json!({"valid_from": "2026-11-05", "valid_through": "2026-11-04",
                       "compare_mode": "previous_period"}),
    )
    .await
    .unwrap_err();
    assert_code(&e, "invalid_period", "explain_sales_change");
    assert_eq!(e.detail.as_deref().unwrap()["field"], "valid_through");
    let e = ask(
        json!({"valid_from": "2026-11-06", "valid_through": "2026-11-07",
                       "compare_mode": "previous_period"}),
    )
    .await
    .unwrap_err();
    assert_code(&e, "invalid_period", "explain_sales_change");
    assert!(e.message.contains("after today"), "{}", e.message);
    let e = ask(
        json!({"valid_from": "2026-10-01", "valid_through": "2026-10-31",
                       "compare_mode": "same_weekday_last_4_weeks"}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.detail.as_deref().unwrap()["field"], "compare_mode");
    let e = ask(
        json!({"valid_from": "2026-11-05", "valid_through": "2026-11-05",
                       "compare_mode": "custom", "compare_from": "2026-10-01"}),
    )
    .await
    .unwrap_err();
    assert_code(&e, "invalid_period", "explain_sales_change");
    assert_eq!(e.detail.as_deref().unwrap()["field"], "compare_from");
}

#[test]
fn explain_takes_the_spec_input_and_nothing_else() {
    let ok: Result<crate::sales::ExplainSalesChangeInput, _> = serde_json::from_value(json!({
        "valid_from": "2026-11-05", "valid_through": "2026-11-05",
        "compare_mode": "same_weekday_last_4_weeks", "acom:metric": "order_count", "max_causes": 3}));
    assert!(ok.is_ok());
    let alias: Result<crate::sales::ExplainSalesChangeInput, _> = serde_json::from_value(json!({
        "valid_from": "2026-11-05", "valid_through": "2026-11-05",
        "compare_mode": "previous_period", "metric": "revenue"}));
    assert!(alias.is_ok(), "the plain name is an alias");
    for bad in [
        json!({"valid_from": "2026-11-05", "valid_through": "2026-11-05", "compare_mode": "last_week"}),
        json!({"valid_from": "2026-11-05", "valid_through": "2026-11-05", "compare_mode": "custom", "x": 1}),
        json!({"valid_from": "Nov 5", "valid_through": "2026-11-05", "compare_mode": "custom"}),
    ] {
        assert!(
            serde_json::from_value::<crate::sales::ExplainSalesChangeInput>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn trend_overview_and_customer_refusals() {
    let cx = offline();
    let e = GetSalesTrend
        .read(
            &cx,
            input(
                json!({"valid_from": "2026-01-01", "valid_through": "2026-11-05",
                         "acom:granularity": "day"}),
            ),
        )
        .await
        .unwrap_err();
    assert_code(&e, "invalid_granularity", "get_sales_trend");
    let e = GetSalesTrend
        .read(
            &cx,
            input(
                json!({"valid_from": "2026-11-05", "valid_through": "2026-11-01",
                         "acom:granularity": "week"}),
            ),
        )
        .await
        .unwrap_err();
    assert_code(&e, "invalid_period", "get_sales_trend");

    let e = GetSalesOverview
        .read(
            &cx,
            input(
                json!({"valid_from": "2026-11-05", "valid_through": "2026-11-05",
                         "acom:metrics": ["revenue", "return_rate"],
                         "acom:dimensions": ["category"]}),
            ),
        )
        .await
        .unwrap_err();
    assert_code(&e, "dimension_not_supported", "get_sales_overview");
    let e = GetSalesOverview
        .read(
            &cx,
            input(
                json!({"valid_from": "2026-11-05", "valid_through": "2026-11-05",
                         "compare_from": "2026-10-29"}),
            ),
        )
        .await
        .unwrap_err();
    assert_code(&e, "invalid_period", "get_sales_overview");

    let e = GetCustomerOverview
        .read(
            &cx,
            input(json!({"valid_from": "2026-11-01", "valid_through": "2026-12-01"})),
        )
        .await
        .unwrap_err();
    assert_code(&e, "invalid_period", "get_customer_overview");
}

// ---- database: scene B3 on the seeded demo store ------------------------------------------

async fn demo() -> Cx {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    cx_on(Db::connect(&url, 2).await.expect("connect"))
}

fn dollars(v: &Value) -> String {
    assert_eq!(v["currency"], "USD", "{v}");
    v["value"].to_string()
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn b3_explains_thursdays_drop_from_the_data() {
    let cx = demo().await;
    let out = ExplainSalesChange
        .read(
            &cx,
            input(
                json!({"valid_from": "2026-11-05", "valid_through": "2026-11-05",
                         "compare_mode": "same_weekday_last_4_weeks"}),
            ),
        )
        .await
        .unwrap();
    // Thu Nov 5: $18,240 and 201 orders against a Thursday average of $22,900 and 247.
    assert_eq!(dollars(&out["period"]["revenue"]), "18240.00");
    assert_eq!(out["period"]["order_count"], 201);
    assert_eq!(out["compare_to"]["basis"], "same_weekday_last_4_weeks");
    assert_eq!(
        out["compare_to"]["observation_dates"],
        json!(["2026-10-08", "2026-10-15", "2026-10-22", "2026-10-29"])
    );
    assert_eq!(dollars(&out["compare_to"]["revenue"]), "22900.00");
    assert_eq!(out["compare_to"]["order_count"], 247);
    // −$4,660 (−20%).
    assert_eq!(dollars(&out["gap"]), "-4660.00");
    assert_eq!(out["change_rate"].to_string(), "-0.2035");

    assert_eq!(out["total"], 2, "{out:#}");
    let stock = &out["items"][0];
    assert_eq!(stock["cause"], "out_of_stock");
    assert_eq!(dollars(&stock["amount"]), "-2700.00");
    assert_eq!(dollars(&stock["expected"]), "2700.00");
    assert_eq!(dollars(&stock["actual"]), "0.00");
    assert_eq!(
        stock["detail"],
        "3 bestsellers out of M and L since Wed Nov 4 15:00"
    );
    assert_eq!(stock["since"], "2026-11-04T15:00:00-05:00");
    let evidence: BTreeSet<&str> = stock["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        evidence,
        BTreeSet::from(["prd_chore_coat", "prd_denim_jacket", "prd_rain_parka"])
    );
    assert_eq!(
        stock["evidence"][0], "prd_chore_coat",
        "the biggest loss first"
    );
    let chore = &stock["products"][0];
    assert_eq!(chore["sizes"], json!(["M", "L"]));
    assert_eq!(chore["availability_starts"], "2026-11-12T00:00:00-05:00");

    let email = &out["items"][1];
    assert_eq!(email["cause"], "campaign_not_sent");
    assert_eq!(dollars(&email["amount"]), "-1300.00");
    assert_eq!(
        email["detail"],
        "Weekly email not sent: list import failed Thu 08:00"
    );
    assert_eq!(email["evidence"], json!(["cmp_weekly_1105"]));
    assert_eq!(
        email["campaign"]["compared_with"],
        json!([
            "cmp_weekly_1008",
            "cmp_weekly_1015",
            "cmp_weekly_1022",
            "cmp_weekly_1029"
        ])
    );

    // Unexplained −$660, and the parts add up to the gap exactly.
    assert_eq!(dollars(&out["unexplained"]), "-660.00");
    let cents = |v: &Value| -> i64 {
        let s = v["value"].to_string();
        let (whole, frac) = s.split_once('.').unwrap();
        let sign = if whole.starts_with('-') { -1 } else { 1 };
        whole.parse::<i64>().unwrap() * 100 + sign * frac.parse::<i64>().unwrap()
    };
    let parts: i64 = out["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| cents(&i["amount"]))
        .sum::<i64>()
        + cents(&out["unexplained"]);
    assert_eq!(parts, cents(&out["gap"]));

    // The same split counted in orders: 201 against 247.
    let orders = ExplainSalesChange
        .read(
            &cx,
            input(json!({"valid_from": "2026-11-05", "valid_through": "2026-11-05",
                         "compare_mode": "same_weekday_last_4_weeks", "acom:metric": "order_count"})),
        )
        .await
        .unwrap();
    assert_eq!(orders["gap"], -46);
    let sum: f64 = orders["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["amount"].as_f64().unwrap())
        .sum::<f64>()
        + orders["unexplained"].as_f64().unwrap();
    assert!((sum - -46.0).abs() < 1e-9, "{orders:#}");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn b3_daily_revenue_for_the_fourteen_days() {
    let cx = demo().await;
    let out = GetSalesTrend
        .read(
            &cx,
            input(
                json!({"valid_from": "2026-10-23", "valid_through": "2026-11-05",
                         "acom:granularity": "day", "acom:metrics": ["revenue", "order_count"]}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(out["total"], 28);
    let revenue: Vec<(String, String)> = out["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["acom:metric"] == "revenue")
        .map(|i| {
            (
                i["observation_date"].as_str().unwrap().to_owned(),
                dollars(&i["value"]),
            )
        })
        .collect();
    let expected = [
        ("2026-10-23", "21480.00"),
        ("2026-10-24", "26950.00"),
        ("2026-10-25", "24310.00"),
        ("2026-10-26", "17840.00"),
        ("2026-10-27", "18620.00"),
        ("2026-10-28", "19750.00"),
        ("2026-10-29", "22600.00"),
        ("2026-10-30", "21930.00"),
        ("2026-10-31", "27410.00"),
        ("2026-11-01", "24880.00"),
        ("2026-11-02", "18150.00"),
        ("2026-11-03", "18940.00"),
        ("2026-11-04", "19320.00"),
        ("2026-11-05", "18240.00"),
    ];
    let expected: Vec<(String, String)> = expected
        .iter()
        .map(|(d, v)| ((*d).to_owned(), (*v).to_owned()))
        .collect();
    assert_eq!(revenue, expected);
    let last = &out["items"][27];
    assert_eq!(
        (last["acom:metric"].as_str(), last["value"].as_i64()),
        (Some("order_count"), Some(201))
    );

    let weeks = GetSalesTrend
        .read(
            &cx,
            input(
                json!({"valid_from": "2026-10-26", "valid_through": "2026-11-01",
                         "acom:granularity": "week"}),
            ),
        )
        .await
        .unwrap();
    // Mon Oct 26 – Sun Nov 1: 17,840 + 18,620 + 19,750 + 22,600 + 21,930 + 27,410 + 24,880.
    assert_eq!(weeks["total"], 1);
    assert_eq!(dollars(&weeks["items"][0]["value"]), "153030.00");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn sales_and_customer_overviews_on_the_demo_store() {
    let cx = demo().await;
    let out = GetSalesOverview
        .read(
            &cx,
            input(
                json!({"valid_from": "2026-11-05", "valid_through": "2026-11-05",
                         "compare_from": "2026-10-29", "compare_through": "2026-10-29"}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(out["total"], 5);
    let item = |m: &str| {
        out["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["acom:metric"] == m)
            .cloned()
            .unwrap()
    };
    assert_eq!(dollars(&item("revenue")["value"]), "18240.00");
    // 18,240 against Oct 29's 22,600.
    assert_eq!(item("revenue")["acom:change_rate"].to_string(), "-0.1929");
    assert_eq!(item("order_count")["value"], 201);
    assert_eq!(dollars(&item("average_order_value")["value"]), "90.75");
    assert_eq!(item("cancellation_rate")["value"], 0);

    let tiers = GetSalesOverview
        .read(
            &cx,
            input(
                json!({"valid_from": "2026-11-05", "valid_through": "2026-11-05",
                         "acom:metrics": ["order_count"], "acom:dimensions": ["member_tier"]}),
            ),
        )
        .await
        .unwrap();
    let by_tier: i64 = tiers["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["value"].as_i64().unwrap())
        .sum();
    assert_eq!(by_tier, 201, "every order lands in one tier (or none)");

    let members = GetCustomerOverview
        .read(
            &cx,
            input(json!({"valid_from": "2026-11-05", "valid_through": "2026-11-05"})),
        )
        .await
        .unwrap();
    let vip = members["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| {
            i["acom:metric"] == "tier_distribution" && i["acom:dimension"]["member_tier"] == "vip"
        })
        .cloned()
        .unwrap();
    assert_eq!(vip["value"], 1284);
    let active = members["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["acom:metric"] == "active_members")
        .cloned()
        .unwrap();
    let n = active["value"].as_i64().unwrap();
    assert!(n > 0 && n <= 201, "{active}");
    for i in members["items"].as_array().unwrap() {
        let text = i.to_string();
        assert!(
            !text.contains("cus_") && !text.contains('@'),
            "no personal data: {text}"
        );
    }
}

#[tokio::test]
async fn service_overview_refuses_a_period_in_the_future() {
    let e = GetCsOverview
        .read(
            &offline(),
            input(json!({"valid_from": "2026-11-01", "valid_through": "2026-11-09"})),
        )
        .await
        .unwrap_err();
    assert_code(&e, "invalid_period", "get_cs_overview");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn service_overview_counts_the_seeded_inquiries() {
    let cx = demo().await;
    let out = GetCsOverview
        .read(
            &cx,
            input(json!({"valid_from": "2026-10-07", "valid_through": "2026-11-06"})),
        )
        .await
        .unwrap();
    let value = |m: &str| {
        out["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["acom:metric"] == m)
            .map(|i| i["value"].clone())
            .unwrap()
    };
    // inq_752 (answered in 21 minutes, resolved), inq_768 (35 minutes, in progress), inq_771
    // (no answer yet).
    assert_eq!(value("inquiry_count"), json!(3));
    assert_eq!(value("unresolved_count"), json!(2));
    assert_eq!(value("average_response_time"), json!(28));
    assert_eq!(value("resolution_rate"), json!(0.3333));
    let by_type = GetCsOverview
        .read(
            &cx,
            input(
                json!({"valid_from": "2026-11-05", "valid_through": "2026-11-06",
                         "acom:metrics": ["inquiry_count", "average_response_time"],
                         "acom:dimensions": ["inquiry_type"]}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(by_type["total"], 4);
    let items = by_type["items"].as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|i| i["acom:dimension"]["inquiry_type"] == "callback"
                && i["acom:metric"] == "average_response_time"
                && i["value"] == json!(35))
    );
    assert!(
        items
            .iter()
            .any(|i| i["acom:dimension"]["inquiry_type"] == "return"
                && i["acom:metric"] == "average_response_time"
                && i["value"].is_null())
    );
    let text = by_type.to_string();
    assert!(!text.contains("cus_") && !text.contains("inq_"), "{text}");
}
