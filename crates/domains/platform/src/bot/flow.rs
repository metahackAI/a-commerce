//! The bot's whole loop on the demo store (`#[ignore]`: needs the seeded demo database): run
//! now, the cycle's report filtered into decisions and tasks, a decision asked again, the same
//! cause not proposed twice, feedback, and settings read back at once.

use acom_auth::scopes;
use acom_contract::{PersonOp, ReadOp};
use acom_core::enums::ActorKind;
use serde_json::{Value, json};

use super::get_bot_cycle::GetBotCycle;
use super::get_bot_status::GetBotStatus;
use super::give_bot_feedback::GiveBotFeedback;
use super::list_bot_decisions::ListBotDecisions;
use super::list_bot_feedback::ListBotFeedback;
use super::rerequest_bot_decision::RerequestBotDecision;
use super::run_bot_cycle::RunBotCycle;
use super::update_bot_settings::UpdateBotSettings;
use crate::testdb::{admin, cx_with, owner_exec};

fn input<T: serde::de::DeserializeOwned>(v: Value) -> T {
    serde_json::from_value(v).unwrap()
}

async fn person<O: PersonOp<acom_store::Cx, Output = Value>>(
    op: O,
    v: Value,
) -> Result<Value, acom_contract::ToolError> {
    let cx = admin().await;
    let mut tx = cx.begin().await.unwrap();
    let out = op.run(&mut tx, input(v)).await;
    if out.is_ok() {
        tx.commit().await.unwrap();
    }
    out
}

async fn report(cycle: &str, body: Value) -> Value {
    let cx = cx_with(
        "sys_ailism_bot",
        ActorKind::System,
        scopes::all_tool_scopes(),
    )
    .await;
    let mut tx = cx.begin().await.unwrap();
    let out = super::report::record(&mut tx, cycle, &body).await.unwrap();
    tx.commit().await.unwrap();
    out
}

fn proposal(area: &str, title: &str, about: &str) -> Value {
    json!({"area": area, "title": title, "request": format!("{title}."), "reason": "From the snapshot.",
           "evidence": ["4 products"], "expected_effect": {"metric": "margin", "direction": "up", "horizon_days": 7},
           "risk": "low", "about": [{"entity": "product", "id": about}]})
}

async fn clean() {
    for sql in [
        "DELETE FROM platform.bot_feedback",
        "UPDATE platform.bot_decision SET revises_id = NULL",
        "DELETE FROM platform.bot_decision",
        "DELETE FROM platform.task WHERE watcher = 'ailism_bot'",
        "DELETE FROM platform.bot_cycle",
    ] {
        owner_exec(sql, &[]).await;
    }
}

#[tokio::test]
#[ignore = "integration: needs the seeded demo database and MIGRATION_DATABASE_URL"]
async fn a_cycle_proposes_once_per_cause_and_a_person_can_ask_again() {
    clean().await;
    // From the store's defaults, whatever an earlier run left.
    person(
        UpdateBotSettings,
        json!({"mode": "propose", "every_minutes": 240, "areas": {"community": true}}),
    )
    .await
    .unwrap();
    let first = person(RunBotCycle, json!({})).await.unwrap();
    let cycle = first["id"].as_str().unwrap().to_owned();
    let again = person(RunBotCycle, json!({})).await.unwrap();
    assert_eq!(
        again["id"], first["id"],
        "the cycle still observing is returned"
    );

    let out = report(
        &cycle,
        json!({"state": "done", "snapshot": {"reads": []}, "anomalies": [],
        "usage": {"model": "glimmer", "model_calls": 1},
        "report": {"summary": "Two things to fix.", "findings": [], "proposals": [
            proposal("pricing", "Raise the Breton tee to the floor", "prd_breton_tee"),
            proposal("pricing", "Raise the Breton tee again", "prd_breton_tee"),
            proposal("weather", "Pray for rain", "prd_rain_parka"),
            proposal("questions_reviews", "Answer the windbreaker question", "prd_windbreaker"),
        ]}}),
    )
    .await;
    assert_eq!(out["state"], "done");
    assert_eq!(out["opened"].as_array().unwrap().len(), 2, "{out}");
    let whys: Vec<&str> = out["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["why"].as_str().unwrap())
        .collect();
    assert_eq!(
        whys,
        [
            "This cycle already proposed the same thing.",
            "It named no known area."
        ]
    );

    let cx = admin().await;
    let list = ListBotDecisions.read(&cx, input(json!({}))).await.unwrap();
    assert_eq!(list["total"], 2);
    assert!(
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["state"] == "drafting"),
        "{list}"
    );
    let c = GetBotCycle
        .read(&cx, input(json!({"bot_cycle_id": cycle})))
        .await
        .unwrap();
    assert_eq!(c["state"], "done");
    assert_eq!(
        c["report"]["proposals"][0]["outcome"]["opened"],
        out["opened"][0]["decision_id"]
    );
    assert_eq!(c["decisions"].as_array().unwrap().len(), 2);

    // The agent's turn for the second one ended without a draft: it waits for a person, with
    // what the agent answered; the request carries the ids the proposal named.
    let second = out["opened"][1]["task_id"].as_str().unwrap().to_owned();
    {
        let sys = cx_with(
            "sys_task_pickup",
            ActorKind::System,
            scopes::all_tool_scopes(),
        )
        .await;
        let mut tx = sys.begin().await.unwrap();
        crate::task::note_undrafted(
            &mut tx,
            &second,
            Some("conv_01testpickup"),
            "I could not find the question.",
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    let list = ListBotDecisions.read(&cx, input(json!({}))).await.unwrap();
    let d = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["task_id"] == json!(second))
        .cloned()
        .unwrap();
    assert_eq!(d["state"], "needs_you", "{d}");
    assert_eq!(
        d["summary"],
        "The agent did not draft a change: I could not find the question."
    );
    assert!(
        d["request"]
            .as_str()
            .unwrap()
            .ends_with("(prd_windbreaker)"),
        "{d}"
    );

    // Asked again: the old one is put away, a new one revises it with the correction.
    let old = out["opened"][0]["decision_id"].as_str().unwrap().to_owned();
    let re = person(
        RerequestBotDecision,
        json!({"bot_decision_id": old, "text": "Only up to the floor, no more."}),
    )
    .await
    .unwrap();
    assert_eq!(re["revises"], json!(old));
    let list = ListBotDecisions.read(&cx, input(json!({}))).await.unwrap();
    let by_id = |id: &str| {
        list["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(by_id(&old)["state"], "dismissed");
    let new = by_id(re["id"].as_str().unwrap());
    assert_eq!(new["state"], "drafting");
    assert!(
        new["request"]
            .as_str()
            .unwrap()
            .ends_with("The operator asked for a change: Only up to the floor, no more.")
    );

    // The next cycle does not propose the same thing while the revision is open.
    let next = person(RunBotCycle, json!({})).await.unwrap();
    assert_ne!(next["id"], json!(cycle));
    let out = report(
        next["id"].as_str().unwrap(),
        json!({"state": "done",
        "report": {"summary": "Again.", "findings": [], "proposals": [
            proposal("pricing", "Raise the Breton tee to the floor", "prd_breton_tee")]}}),
    )
    .await;
    assert_eq!(out["opened"], json!([]));
    assert!(
        out["skipped"][0]["why"]
            .as_str()
            .unwrap()
            .starts_with("A decision about the same thing is still open"),
        "{out}"
    );

    // Feedback is open until closed; settings read back at once.
    let f = person(
        GiveBotFeedback,
        json!({"kind": "directive", "text": "Never discount new arrivals."}),
    )
    .await
    .unwrap();
    let open = ListBotFeedback
        .read(&cx, input(json!({"state": "open"})))
        .await
        .unwrap();
    assert!(
        open["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["id"] == f["id"])
    );
    let s = person(
        UpdateBotSettings,
        json!({"mode": "observe", "every_minutes": 120, "areas": {"community": false}}),
    )
    .await
    .unwrap();
    assert_eq!(s["settings"]["mode"], "observe");
    let status = GetBotStatus
        .read(&admin().await, input(json!({})))
        .await
        .unwrap();
    assert_eq!(status["settings"]["every_minutes"], 120);
    assert_eq!(status["settings"]["areas"]["community"], false);
    assert_eq!(status["decisions"]["drafting"], 1, "the revision");
    assert_eq!(
        status["decisions"]["needs_you"], 1,
        "the one the agent could not draft"
    );
    assert!(
        person(UpdateBotSettings, json!({"acts_as": "adm_nobody"}))
            .await
            .is_err()
    );
    person(
        UpdateBotSettings,
        json!({"mode": "propose", "every_minutes": 240, "areas": {"community": true}}),
    )
    .await
    .unwrap();
    clean().await;
}
