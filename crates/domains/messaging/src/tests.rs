//! Contract tests for every messaging operation. The first group needs no database (a pool
//! that never connects, inputs refused before any query, a template preview that reads
//! nothing); the `#[ignore]` group runs scene B2's messages on the demo store with
//! `DATABASE_URL` set: `cargo test -p acom-messaging -- --ignored`.

use std::collections::BTreeSet;
use std::sync::Arc;

use acom_auth::{Principal, Surface};
use acom_contract::{ApprovalNeed, ReadOp, WriteOp};
use acom_core::enums::ActorKind;
use acom_core::{FixedClock, StoreTime, TenantId};
use acom_policy::PolicySet;
use acom_store::{Cx, Db};
use serde_json::{Value, json};

use crate::message::SendCustomerMessage;
use crate::notification::ListNotifications;
use crate::template::{CreateMessageTemplate, ListMessageTemplates};

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

fn cx_on(db: Db, subject: &str) -> Cx {
    let time = StoreTime::new(chrono_tz_ny());
    let (kind, surface) = if subject.starts_with("cus_") {
        (ActorKind::Shopper, Surface::Fo)
    } else {
        (ActorKind::Operator, Surface::Bo)
    };
    Cx {
        tenant: TenantId::parse("store_northwind").unwrap(),
        principal: Arc::new(Principal {
            subject: subject.to_owned(),
            kind,
            tenant: TenantId::parse("store_northwind").unwrap(),
            surface,
            scopes: BTreeSet::new(),
            name: None,
            session_id: None,
        }),
        request_id: "req_test".into(),
        clock: Arc::new(FixedClock::at(time.parse("2026-11-06T10:40").unwrap())),
        time,
        policy: Arc::new(policy()),
        db,
    }
}

fn chrono_tz_ny() -> chrono_tz::Tz {
    chrono_tz::America::New_York
}

fn offline(subject: &str) -> Cx {
    cx_on(
        Db::connect_lazy("postgres://offline.invalid/acom", 1).unwrap(),
        subject,
    )
}

fn input<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
async fn a_message_needs_text() {
    let e = SendCustomerMessage
        .preview(
            &offline("adm_sj"),
            input(json!({"customer_id": "cus_riley", "order_id": "ord_20366",
                         "kind": "ask_for_photos", "text": "   "})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "empty_text");
    assert_eq!(e.next_tool.as_deref(), Some("send_customer_message"));
}

#[tokio::test]
async fn a_template_preview_checks_its_variables_and_needs_no_approval() {
    let cx = offline("adm_sj");
    let p = CreateMessageTemplate
        .preview(
            &cx,
            input(
                json!({"name": "Back in stock", "channel": "email", "purpose": "marketing",
                         "headline": "{{product_name}} is back",
                         "text": "Hi {{customer_name}}, {{product_name}} is back in your size.",
                         "variables": ["customer_name", "product_name"]}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(p.approval, ApprovalNeed::NotNeeded);
    assert_eq!(p.checks[0].rule, "messaging.template.variables_declared");
    assert_eq!(
        p.checks[0].detail.as_deref(),
        Some("2 variables, all declared")
    );
    assert_eq!(p.changes[0].to["status"], "draft");
    assert_eq!(p.plan.in_language, "en-US");
    assert!(p.plan.template_id.starts_with("tpl_"));

    let e = CreateMessageTemplate
        .preview(
            &cx,
            input(
                json!({"name": "Back in stock", "channel": "email", "purpose": "marketing",
                         "text": "Hi {{customer_name}}", "variables": []}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "unknown_variable");
    let e = CreateMessageTemplate
        .preview(
            &cx,
            input(
                json!({"name": "Ping", "channel": "sms", "purpose": "transactional",
                         "headline": "Hello", "text": "Your order shipped."}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "channel_not_supported");
}

#[tokio::test]
async fn a_shopper_only_lists_their_own_notifications() {
    let e = ListNotifications
        .read(
            &offline("cus_alex"),
            input(json!({"customer_id": "cus_riley"})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "customer_not_found");
    let e = ListNotifications
        .read(&offline("adm_sj"), input(json!({"customer_id": "me"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "customer_not_found");
}

#[tokio::test]
async fn template_list_refuses_a_cursor_it_did_not_make() {
    let e = ListMessageTemplates
        .read(&offline("adm_sj"), input(json!({"cursor": "page2"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "invalid_input");
}

// ── Scene B2 on the demo store (integration) ─────────────────────────────────

async fn demo(subject: &str) -> Cx {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    cx_on(Db::connect(&url, 2).await.expect("connect"), subject)
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn b2_photo_request_reads_as_riley_will_and_moves_the_claim() {
    let cx = demo("adm_sj").await;
    let p = SendCustomerMessage
        .preview(
            &cx,
            input(json!({"customer_id": "cus_riley", "order_id": "ord_20366",
                         "kind": "ask_for_photos",
                         "text": "Sorry your rain parkas arrived damaged. Could you reply with a photo or two of the damage? We'll take it from there."})),
        )
        .await
        .unwrap();
    let to = &p.changes[0].to;
    assert_eq!(to["recipient"], "R*** P***");
    assert_eq!(to["subject"], "About your return for order 20366");
    assert_eq!(
        p.checks[1].detail.as_deref(),
        Some("Order 20366 has no photo of the damage")
    );
    assert_eq!(
        p.approval,
        ApprovalNeed::Needed {
            reason: "Sends a message to a customer".into()
        }
    );
    let mut tx = cx.as_system("sys_runner").begin().await.unwrap();
    let done = SendCustomerMessage.commit(&mut tx, &p.plan).await.unwrap();
    let again = SendCustomerMessage.commit(&mut tx, &p.plan).await.unwrap();
    assert_eq!(done, again);
    let status: String = sqlx::query_scalar(
        "SELECT claim_status FROM fulfillment.claim WHERE tenant_id = $1 AND id = 'clm_20366'",
    )
    .bind("store_northwind")
    .fetch_one(&mut *tx.tx)
    .await
    .unwrap();
    assert_eq!(status, "photos_requested");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn b2_decline_rests_on_the_return_window() {
    let cx = demo("adm_sj").await;
    let p = SendCustomerMessage
        .preview(
            &cx,
            input(json!({"customer_id": "cus_dana", "order_id": "ord_20290", "kind": "decline",
                         "text": "We're sorry, but this sweater arrived 39 days ago, past our 30-day return window."})),
        )
        .await
        .unwrap();
    let window = p
        .checks
        .iter()
        .find(|c| c.rule == "return_window_days")
        .unwrap();
    assert!(window.passed);
    assert_eq!(
        window.detail.as_deref(),
        Some("Delivered Sep 28, 39 days ago (limit 30)")
    );
    let e = SendCustomerMessage
        .preview(
            &cx,
            input(
                json!({"customer_id": "cus_riley", "order_id": "ord_20290", "kind": "decline",
                         "text": "Hello"}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "order_not_for_customer");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn templates_and_notifications_read_back() {
    let cx = demo("adm_sj").await;
    let p = CreateMessageTemplate
        .preview(
            &cx,
            input(json!({"name": "Refund on its way", "channel": "email",
                         "purpose": "transactional", "text": "Your refund is on its way."})),
        )
        .await
        .unwrap();
    let mut tx = cx.begin().await.unwrap();
    CreateMessageTemplate
        .commit(&mut tx, &p.plan)
        .await
        .unwrap();
    CreateMessageTemplate
        .commit(&mut tx, &p.plan)
        .await
        .unwrap();
    drop(tx);
    let list = ListMessageTemplates
        .read(&cx, input(json!({"channel": "email"})))
        .await
        .unwrap();
    assert!(list["total"].as_i64().is_some());
    let n = ListNotifications
        .read(&cx, input(json!({"customer_id": "cus_4431"})))
        .await
        .unwrap();
    assert_eq!(n["total"], 3);
    assert_eq!(n["items"][0]["id"], "msg_20311_refund");
    assert_eq!(n["items"][0]["read"], false);
}
