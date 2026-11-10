//! Ailism Customer Bot's whole record on the demo store (`#[ignore]`: needs the seeded demo
//! database): a person runs it now, the workers take the run, a session records its steps and
//! the persona it made, the run ends with its last session, and the reads show it all.

use acom_auth::scopes;
use acom_contract::{PersonOp, ReadOp};
use acom_core::enums::ActorKind;
use serde_json::{Value, json};

use super::get_customer_bot_run::GetCustomerBotRun;
use super::get_customer_bot_session::GetCustomerBotSession;
use super::get_customer_bot_status::GetCustomerBotStatus;
use super::list_customer_bot_sessions::ListCustomerBotSessions;
use super::list_shopper_personas::ListShopperPersonas;
use super::record;
use super::run_customer_bot::RunCustomerBot;
use super::update_customer_bot_settings::UpdateCustomerBotSettings;
use crate::testdb::{admin, cx_with, owner_exec};

fn input<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

async fn person<O: PersonOp<acom_store::Cx, Output = Value>>(op: O, v: Value) -> Value {
    let cx = admin().await;
    let mut tx = cx.begin().await.unwrap();
    let out = op.run(&mut tx, input(v)).await.unwrap();
    tx.commit().await.unwrap();
    out
}

async fn worker() -> acom_store::Cx {
    cx_with(
        "sys_customer_bot",
        ActorKind::System,
        scopes::all_tool_scopes(),
    )
    .await
}

async fn clean() {
    for sql in [
        "DELETE FROM platform.shopper_persona",
        "DELETE FROM platform.customer_bot_step",
        "DELETE FROM platform.customer_bot_session",
        "DELETE FROM platform.customer_bot_run",
    ] {
        owner_exec(sql, &[]).await;
    }
}

#[tokio::test]
#[ignore = "integration: needs the seeded demo database and MIGRATION_DATABASE_URL"]
async fn a_run_records_every_step_and_ends_with_its_last_session() {
    clean().await;
    person(
        UpdateCustomerBotSettings,
        json!({"mode": "off", "workers": 2, "every_minutes": 10}),
    )
    .await;
    // Off: no scheduled run, but a person can run it now.
    let w = worker().await;
    let mut tx = w.begin().await.unwrap();
    assert!(record::claim_next_run(&mut tx).await.unwrap().is_none());
    tx.commit().await.unwrap();
    let queued = person(RunCustomerBot, json!({})).await;
    assert_eq!(queued["state"], "queued");
    let again = person(RunCustomerBot, json!({})).await;
    assert_eq!(again["id"], queued["id"], "one run at a time");

    let mut tx = w.begin().await.unwrap();
    let run = record::claim_next_run(&mut tx).await.unwrap().unwrap();
    assert_eq!(
        (run.id.as_str(), run.trigger.as_str(), run.workers),
        (queued["id"].as_str().unwrap(), "manual", 2)
    );
    let sessions = record::open_sessions(&mut tx, &run.id, &[Some("cus_m000001".to_owned()), None])
        .await
        .unwrap();
    // While it works, no other run starts.
    assert!(record::claim_next_run(&mut tx).await.unwrap().is_none());
    tx.commit().await.unwrap();
    let existing = sessions[0]["id"].as_str().unwrap().to_owned();
    let created = sessions[1]["id"].as_str().unwrap().to_owned();
    assert_eq!(sessions[1]["account"], "created");

    let mut tx = w.begin().await.unwrap();
    record::append(
        &mut tx,
        &existing,
        &json!({"goal": "A rain jacket under $120",
                "conversation_id": "conv_test_1",
                "steps": [
                    {"kind": "sign_in", "status": "ok", "title": "Signed in as cus_m000001", "ms": 40,
                     "request": {"method": "POST", "path": "/api/auth/demo-login"}, "response": {"status": 200}},
                    {"kind": "chat", "status": "ok", "title": "Asked: any rain jackets?",
                     "detail": {"reply": "Here are three.", "tools": ["search_products"]}}]}),
    )
    .await
    .unwrap();
    let bad = record::append(
        &mut tx,
        &existing,
        &json!({"steps": [{"kind": "approve", "title": "x"}]}),
    )
    .await;
    assert!(bad.is_err(), "a step kind outside the list is refused");
    tx.commit().await.unwrap();
    let mut tx = w.begin().await.unwrap();
    record::save_persona(
        &mut tx,
        "cus_m000001",
        &json!({"description": "Commuter who buys navy outerwear, size M.",
                "persona": {"sizes": {"top": "M"}}, "session_id": existing}),
    )
    .await
    .unwrap();
    record::finish_session(
        &mut tx,
        &existing,
        &json!({"state": "done", "outcome": {"chat_turns": 3, "orders": 1, "reviews": 1,
                                             "spent": {"value": 95.4, "currency": "USD"}}}),
    )
    .await
    .unwrap();
    let mid = record::finish_session(
        &mut tx,
        &created,
        &json!({"state": "failed", "error": "Could not sign up."}),
    )
    .await
    .unwrap();
    assert_eq!(
        mid["run_state"], "done",
        "the run ends with its last session"
    );
    assert!(
        record::append(&mut tx, &existing, &json!({"steps": []}))
            .await
            .is_err(),
        "an ended session takes no more steps"
    );
    tx.commit().await.unwrap();

    let got = GetCustomerBotRun
        .read(
            &admin().await,
            input(json!({"customer_bot_run_id": run.id})),
        )
        .await
        .unwrap();
    assert_eq!(got["state"], "done");
    assert_eq!(got["summary"]["orders"], 1);
    assert_eq!(got["summary"]["failed"], 1);
    assert_eq!(got["summary"]["personas_built"], 1);
    assert_eq!(got["sessions"].as_array().unwrap().len(), 2);
    let one = GetCustomerBotSession
        .read(
            &admin().await,
            input(json!({"customer_bot_session_id": existing})),
        )
        .await
        .unwrap();
    assert_eq!(one["steps"].as_array().unwrap().len(), 2);
    assert_eq!(one["steps"][1]["detail"]["tools"][0], "search_products");
    assert_eq!(one["goal"], "A rain jacket under $120");
    assert_eq!(one["conversation_ids"][0], "conv_test_1");
    assert_eq!(one["persona"]["sessions"], 1);
    let mine = ListCustomerBotSessions
        .read(&admin().await, input(json!({"customer_id": "cus_m000001"})))
        .await
        .unwrap();
    assert_eq!(mine["total"], 1);
    let personas = ListShopperPersonas
        .read(&admin().await, input(json!({})))
        .await
        .unwrap();
    assert_eq!(personas["total"], 1);
    let status = GetCustomerBotStatus
        .read(&admin().await, input(json!({})))
        .await
        .unwrap();
    assert_eq!(status["totals"]["orders"], 1);
    assert_eq!(status["last_run"]["id"], got["id"]);
    assert!(status["next_run_at"].is_null(), "off: no next run");

    // On again: the next scheduled run waits for its minutes.
    person(UpdateCustomerBotSettings, json!({"mode": "on"})).await;
    let mut tx = w.begin().await.unwrap();
    assert!(
        record::claim_next_run(&mut tx).await.unwrap().is_none(),
        "the last run started moments ago"
    );
    tx.commit().await.unwrap();
    clean().await;
}
