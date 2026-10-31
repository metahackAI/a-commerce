//! Contract tests for every messaging operation. The first group needs no database (a pool
//! that never connects, inputs refused before any query, a template preview that reads
//! nothing); the `#[ignore]` group runs scene B2's messages, a message found by its order alone,
//! and campaigns (the seeded VIP reminder, a scheduled send and its job) on the demo store with
//! `DATABASE_URL` set: `cargo test -p acom-messaging -- --ignored --test-threads=1`. Every write
//! runs in a transaction that is rolled back.

use std::collections::BTreeSet;
use std::sync::Arc;

use acom_auth::{Principal, Surface};
use acom_contract::{ApprovalNeed, ErrorClass, ReadOp, WriteOp};
use acom_core::enums::ActorKind;
use acom_core::{FixedClock, StoreTime, TenantId};
use acom_policy::PolicySet;
use acom_store::{Cx, Db};
use serde_json::{Value, json};

use crate::campaign::{CreateCampaign, GetCampaign, SearchCampaigns};
use crate::message::{SendBulk, SendCustomerMessage};
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
        currency: acom_core::Currency::parse("USD").unwrap(),
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

// ── The order's customer, campaigns, the scheduled send ──────────────────────────────────────

#[tokio::test]
async fn a_message_needs_an_order_or_a_customer() {
    let e = SendCustomerMessage
        .preview(
            &offline("adm_sj"),
            input(json!({"kind": "custom", "text": "Thanks for shopping with us."})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "customer_required");
    assert_eq!(e.class, ErrorClass::InvalidInput);
    assert_eq!(e.next_tool.as_deref(), Some("send_customer_message"));
    let e = serde_json::from_value::<crate::message::SendCustomerMessageInput>(
        json!({"order_id": "ord_20366", "kind": "campaign", "text": "Hi"}),
    )
    .unwrap_err();
    assert!(e.to_string().contains("campaign"), "{e}");
}

#[tokio::test]
async fn a_campaign_is_refused_before_anything_is_read() {
    let cx = offline("adm_sj");
    let base = json!({"name": "VIP weekend reminder", "channel": "email",
                      "template_id": "tpl_vip_weekend", "audience_mode": "tier"});
    let mut mixed = base.clone();
    mixed["valid_for_member_tier"] = json!("vip");
    mixed["customer_ids"] = json!(["cus_jamie"]);
    let e = CreateCampaign.preview(&cx, input(mixed)).await.unwrap_err();
    assert_eq!(e.code, "invalid_input");
    let e = CreateCampaign
        .preview(&cx, input(base.clone()))
        .await
        .unwrap_err();
    assert_eq!(e.detail.unwrap()["field"], "valid_for_member_tier");
    let mut past = base.clone();
    past["valid_for_member_tier"] = json!("vip");
    past["scheduled_time"] = json!("2026-11-06T09:00");
    let e = CreateCampaign.preview(&cx, input(past)).await.unwrap_err();
    assert_eq!(e.detail.unwrap()["reason"], "past");
    let mut night = base;
    night["valid_for_member_tier"] = json!("vip");
    night["scheduled_time"] = json!("2026-11-06T22:30");
    let e = CreateCampaign.preview(&cx, input(night)).await.unwrap_err();
    assert_eq!(e.code, "quiet_hours");
    assert_eq!(e.next_tool.as_deref(), Some("create_campaign"));
}

#[test]
fn campaign_operations_match_the_spec() {
    use acom_contract::{Kind, Method, Operation};
    assert_eq!(CreateCampaign::META.kind, Kind::WriteApproval);
    assert_eq!(SendBulk::META.kind, Kind::WriteApproval);
    assert_eq!(
        SendBulk::META.path,
        "/v1/messaging/campaigns/{campaign_id}/send"
    );
    assert_eq!(GetCampaign::META.method, Method::Get);
    assert_eq!(SearchCampaigns::META.produces[0].name, "campaign_id");
    let mut b = acom_contract::RegistryBuilder::default();
    crate::register(&mut b);
    let r = b.build().unwrap_or_else(|e| panic!("{e:#?}"));
    assert_eq!(r.iter().count(), 8);
    assert!(r.job("messaging.send_campaign").is_some());
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_message_about_an_order_goes_to_its_customer() {
    let cx = demo("adm_sj").await;
    let p = SendCustomerMessage
        .preview(
            &cx,
            input(json!({"order_id": "ord_20366", "kind": "ask_for_photos",
                         "text": "Could you reply with a photo or two of the damage?"})),
        )
        .await
        .unwrap();
    assert_eq!(p.plan.customer_id, "cus_riley");
    assert_eq!(p.changes[0].to["recipient"], "R*** P***");
    assert_eq!(p.changes[0].to["customer_id"], "cus_riley");
    let e = SendCustomerMessage
        .preview(
            &cx,
            input(json!({"customer_id": "cus_jamie", "order_id": "ord_20366",
                         "kind": "ask_for_photos", "text": "Hello"})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "order_not_for_customer");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn the_vip_reminder_reaches_only_members_who_opted_in() {
    let cx = demo("adm_sj").await;
    // The seeded draft: only Jamie of the vip tier agreed to marketing email.
    let p = SendBulk
        .preview(&cx, input(json!({"campaign_id": "cmp_vip_weekend"})))
        .await
        .unwrap();
    assert_eq!(p.plan.recipients.len(), 1);
    assert_eq!(p.plan.recipients[0].customer_id, "cus_jamie");
    let consent = &p.checks[0];
    assert_eq!(consent.rule, "consent.marketing_gate");
    assert!(
        consent.detail.as_deref().unwrap().starts_with("1 of "),
        "{consent:?}"
    );
    assert_eq!(
        p.changes[0].to["sample"]["subject"],
        "VIP weekend: early access to the wool overcoat"
    );
    assert!(p.changes[0].note.as_deref().unwrap().contains("recorded"));
    assert_eq!(
        p.approval,
        ApprovalNeed::Needed {
            reason: "Sends email to 1 customer".into()
        }
    );

    let mut tx = cx.as_system("sys_runner").begin().await.unwrap();
    let done = SendBulk.commit(&mut tx, &p.plan).await.unwrap();
    assert_eq!(
        done.produced["message_id"],
        json!(["msg_cmp_vip_weekend_cus_jamie"])
    );
    let again = SendBulk.commit(&mut tx, &p.plan).await.unwrap();
    assert_eq!(done, again);
    let (status, text): (String, String) = sqlx::query_as(
        "SELECT c.campaign_status, m.text FROM messaging.campaign c
         JOIN messaging.message m ON m.tenant_id = c.tenant_id AND m.campaign_id = c.id
         WHERE c.tenant_id = $1 AND c.id = 'cmp_vip_weekend'",
    )
    .bind("store_northwind")
    .fetch_one(&mut *tx.tx)
    .await
    .unwrap();
    assert_eq!(status, "sent");
    assert!(text.starts_with("Hi Jamie, as a VIP you get early access"));
    // A second campaign the same day is still under the daily cap of 2.
    let (keep, capped) =
        crate::campaign::under_cap(&mut tx, p.plan.recipients.clone(), cx.clock.now())
            .await
            .unwrap();
    assert_eq!((keep.len(), capped), (1, 0));
    // Rolled back on drop.
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_scheduled_campaign_is_booked_sent_by_its_job_and_reads_back() {
    let cx = demo("adm_sj").await;
    let p = CreateCampaign
        .preview(
            &cx,
            input(json!({"name": "Overcoat early access", "channel": "email",
                         "template_id": "tpl_vip_weekend", "audience_mode": "customers",
                         "customer_ids": ["cus_jamie", "cus_alex", "cus_riley"],
                         "scheduled_time": "2026-11-07T10:00",
                         "variables": [{"name": "headline_item", "value": "the wool overcoat"}]})),
        )
        .await
        .unwrap();
    assert_eq!(p.changes[0].to["status"], "scheduled");
    assert_eq!(p.changes[0].to["recipients"], 2);
    assert_eq!(p.changes[0].to["left_out"], 1);
    assert_eq!(
        p.checks[1].detail.as_deref(),
        Some("Sends at 10:00 store time")
    );
    let mut tx = cx.begin().await.unwrap();
    let done = CreateCampaign.commit(&mut tx, &p.plan).await.unwrap();
    let job: (String, String) = sqlx::query_as(
        "SELECT kind, payload->>'campaign_id' FROM platform.job
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind("store_northwind")
    .bind(format!("job_send_{}", done.id))
    .fetch_one(&mut *tx.tx)
    .await
    .unwrap();
    assert_eq!(job, ("messaging.send_campaign".to_owned(), done.id.clone()));
    let outcome = <crate::schedule::SendScheduledCampaign as acom_contract::Job<Cx>>::run(
        &crate::schedule::SendScheduledCampaign,
        &mut tx,
        &json!({"campaign_id": done.id}),
    )
    .await
    .unwrap();
    let acom_contract::JobOutcome::Done { result, .. } = outcome else {
        panic!("done")
    };
    assert_eq!(result["status"], "sent");
    assert_eq!(result["sent"], 2);
    let e = CreateCampaign
        .undo(&mut tx, &p.plan, &done)
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_revocable");
    drop(tx);

    let c = GetCampaign
        .read(&cx, input(json!({"campaign_id": "cmp_weekly_1105"})))
        .await
        .unwrap();
    assert_eq!(c["status"], "failed");
    assert_eq!(c["error"]["message"], "List import failed");
    assert_eq!(c["stats"]["sent"], 0);
    let sent = GetCampaign
        .read(
            &cx,
            input(json!({"campaign_id": "cmp_weekly_1029", "fields": ["stats"]})),
        )
        .await
        .unwrap();
    assert_eq!(sent["stats"]["sent"], 11270);
    assert!(sent.get("name").is_none());
    let failed = SearchCampaigns
        .read(&cx, input(json!({"status": "failed"})))
        .await
        .unwrap();
    assert_eq!(failed["total"], 1);
    assert_eq!(failed["items"][0]["error_message"], "List import failed");
    let weekly = SearchCampaigns
        .read(
            &cx,
            input(json!({"template_id": "tpl_weekly_email", "scheduled_from": "2026-10-20T00:00"})),
        )
        .await
        .unwrap();
    assert_eq!(weekly["total"], 3);
    assert_eq!(weekly["items"][0]["id"], "cmp_weekly_1105");
    let e = GetCampaign
        .read(&cx, input(json!({"campaign_id": "cmp_nope"})))
        .await
        .unwrap_err();
    assert_eq!(e.code, "campaign_not_found");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_campaign_nobody_can_get_is_refused_and_a_cancelled_one_is_not_sent() {
    let cx = demo("adm_sj").await;
    let e = CreateCampaign
        .preview(
            &cx,
            input(json!({"name": "Riley only", "channel": "sms",
                         "template_id": "tpl_vip_weekend", "audience_mode": "customers",
                         "customer_ids": ["cus_riley"]})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "template_channel_mismatch");
    let e = CreateCampaign
        .preview(
            &cx,
            input(json!({"name": "Riley only", "channel": "email",
                         "template_id": "tpl_vip_weekend", "audience_mode": "customers",
                         "customer_ids": ["cus_riley"],
                         "variables": [{"name": "headline_item", "value": "a look"}]})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "no_consented_recipients");
    let e = CreateCampaign
        .preview(
            &cx,
            input(json!({"name": "No values", "channel": "email",
                         "template_id": "tpl_vip_weekend", "audience_mode": "tier",
                         "valid_for_member_tier": "vip"})),
        )
        .await
        .unwrap_err();
    assert_eq!(e.detail.unwrap()["items"], json!(["headline_item"]));
    let p = CreateCampaign
        .preview(
            &cx,
            input(json!({"name": "Draft for VIPs", "channel": "email",
                         "template_id": "tpl_vip_weekend", "audience_mode": "tier",
                         "valid_for_member_tier": "vip",
                         "variables": [{"name": "headline_item", "value": "a look"}]})),
        )
        .await
        .unwrap();
    assert_eq!(p.changes[0].to["status"], "draft");
    let mut tx = cx.begin().await.unwrap();
    let done = CreateCampaign.commit(&mut tx, &p.plan).await.unwrap();
    CreateCampaign.undo(&mut tx, &p.plan, &done).await.unwrap();
    let c = crate::repo::campaign(&mut tx, &done.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.campaign_status, "cancelled");
    let e = crate::message::ready(&mut tx, &c).await.unwrap_err();
    assert_eq!(e.code, "campaign_not_ready");
    // Rolled back on drop.
}
