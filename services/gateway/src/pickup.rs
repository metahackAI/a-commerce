//! Task pickup (docs/api/platform.md §task): a task a watcher opens with origin `agent_noticed`
//! goes to the agent before anyone opens it. The gateway follows the store's progress events
//! (`GET /v1/platform/events`) and, for each task a watcher creates, sends the task's request as a
//! back-office turn with its `task_id`, for the operator `platform.yml task_pickup.acts_as` names.
//! The turn carries that operator's on-behalf token, never a `human:*` scope, so what the agent
//! drafts waits for a person's approval. Off unless the store sets `task_pickup.enabled`.
//!
//! Only new tasks are picked up: a refreshed or seeded task stays for a person to hand over.
//! Turns run one at a time, in the order the tasks appeared.

use std::sync::Arc;
use std::time::Duration;

use acom_auth::Principal;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{Gateway, REQUEST_TOKEN_SECONDS, TURN_TOKEN_SECONDS, now_seconds};

/// The subject commerce sees for the gateway's own reads.
const SUBJECT: &str = "sys_task_pickup";
/// How long to wait before following the events again after the stream ends or fails.
const RECONNECT: Duration = Duration::from_secs(5);
/// The line a back-office turn stream sends when the turn finished.
const TURN_DONE: &[u8] = b"event: turn.done";

/// Starts following the store's events, and the worker that hands new tasks to the agent.
pub fn spawn(gw: Arc<Gateway>) {
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(work(gw.clone(), rx));
    tokio::spawn(async move {
        let http = reqwest::Client::new();
        loop {
            if let Err(e) = follow(&gw, &http, &tx).await {
                tracing::warn!(error = %e, "task pickup: events stream");
            }
            tokio::time::sleep(RECONNECT).await;
        }
    });
}

/// A token for the gateway's own reads in commerce.
fn system_token(gw: &Gateway, request_id: &str) -> Result<String, String> {
    let p = Principal::system(gw.sessions.tenant.clone(), SUBJECT);
    gw.signer
        .sign(&p, request_id, now_seconds(), REQUEST_TOKEN_SECONDS)
        .map_err(|e| e.to_string())
}

/// Reads the events stream until it ends, queueing every task a watcher created for the agent.
async fn follow(
    gw: &Gateway,
    http: &reqwest::Client,
    queue: &mpsc::UnboundedSender<String>,
) -> Result<(), String> {
    let token = system_token(gw, "req_task_pickup_events")?;
    let mut r = http
        .get(format!("{}/v1/platform/events", gw.commerce))
        .bearer_auth(token)
        .header("accept", "text/event-stream")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("events: {}", r.status()));
    }
    // Bytes until a whole line is in: a chunk may end inside a character, never inside a `\n`.
    let (mut pending, mut event) = (Vec::<u8>::new(), String::new());
    while let Some(chunk) = r.chunk().await.map_err(|e| e.to_string())? {
        pending.extend_from_slice(&chunk);
        while let Some(end) = pending.iter().position(|b| *b == b'\n') {
            let raw: Vec<u8> = pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end();
            if let Some(e) = line.strip_prefix("event:") {
                e.trim().clone_into(&mut event);
            } else if let Some(data) = line.strip_prefix("data:")
                && let Some(task) = new_agent_task(&event, data)
            {
                let _ = queue.send(task);
            }
        }
    }
    Ok(())
}

/// The task id of a `task.updated` event that announces a new `agent_noticed` task.
fn new_agent_task(event: &str, data: &str) -> Option<String> {
    let v: Value = serde_json::from_str(data.trim()).ok()?;
    (event == "task.updated" && v["created"] == true && v["origin"] == "agent_noticed")
        .then(|| v["task_id"].as_str().map(str::to_owned))
        .flatten()
}

async fn work(gw: Arc<Gateway>, mut rx: mpsc::UnboundedReceiver<String>) {
    let http = reqwest::Client::new();
    while let Some(task) = rx.recv().await {
        // Each pickup runs as its own task, so a panic in one leaves the queue running.
        let (gw, http, id) = (gw.clone(), http.clone(), task.clone());
        match tokio::spawn(async move { pick_up(&gw, &http, &id).await }).await {
            Ok(Ok(outcome)) => tracing::info!(task = %task, outcome = %outcome, "task pickup"),
            Ok(Err(e)) => tracing::warn!(task = %task, error = %e, "task pickup"),
            Err(e) => tracing::error!(task = %task, error = %e, "task pickup panicked"),
        }
    }
}

/// Hands one task to the agent, if the store wants that and the task is still open and undrafted.
async fn pick_up(gw: &Gateway, http: &reqwest::Client, task: &str) -> Result<String, String> {
    let request_id = format!("req_pickup_{task}");
    let token = system_token(gw, &request_id)?;
    let read = |path: String| {
        let token = token.clone();
        async move {
            let r = http
                .get(format!("{}{path}", gw.commerce))
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            if !r.status().is_success() {
                return Err(format!("{path}: {}", r.status()));
            }
            let text = r.text().await.map_err(|e| e.to_string())?;
            serde_json::from_str::<Value>(&text).map_err(|e| format!("{path}: {e}"))
        }
    };

    let policy = read("/v1/platform/policies?key=task_pickup".into()).await?;
    let value = |key: &str| {
        policy["items"]
            .as_array()
            .and_then(|items| items.iter().find(|i| i["key"] == key))
            .map(|i| i["value"].clone())
            .unwrap_or(Value::Null)
    };
    if value("task_pickup.enabled") != true {
        return Ok("off".into());
    }
    let Some(acts_as) = value("task_pickup.acts_as").as_str().map(String::from) else {
        return Err("task_pickup.acts_as names no operator".into());
    };

    let t = read(format!("/v1/platform/tasks/{task}")).await?;
    let open = matches!(t["state"].as_str(), Some("needs_you" | "working"));
    let drafted = !t["related"]["change_set_id"].is_null();
    let request = t["request"].as_str().map(str::trim).unwrap_or_default();
    if !open || drafted || request.is_empty() {
        return Ok("nothing to do".into());
    }

    let operator = gw
        .sessions
        .operator(&acts_as)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("operator {acts_as} is not active"))?;
    let token = gw
        .signer
        .sign(
            &operator.on_behalf(),
            &request_id,
            now_seconds(),
            TURN_TOKEN_SECONDS,
        )
        .map_err(|e| e.to_string())?;
    let mut r = http
        .post(format!("{}/chat/bo/turns", gw.agent))
        .bearer_auth(token)
        .header("accept", "text/event-stream")
        .header("content-type", "application/json")
        .body(json!({"message": request, "task_id": task}).to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        return Err(format!(
            "turn: {status} {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    // Read the turn to its end; the change set and the task carry the result.
    let (mut tail, mut done) = (Vec::new(), false);
    while let Some(chunk) = r.chunk().await.map_err(|e| e.to_string())? {
        done |= saw_turn_done(&mut tail, &chunk);
    }
    Ok(if done {
        format!("turn done as {acts_as}")
    } else {
        format!("turn ended without turn.done as {acts_as}")
    })
}

/// Feeds one chunk of a turn stream: true when [`TURN_DONE`] is in it, also when chunks split
/// it. Works on bytes, since a chunk may end inside a character; `tail` keeps only the bytes a
/// split `TURN_DONE` could start with.
fn saw_turn_done(tail: &mut Vec<u8>, chunk: &[u8]) -> bool {
    tail.extend_from_slice(chunk);
    let seen = tail.windows(TURN_DONE.len()).any(|w| w == TURN_DONE);
    tail.drain(..tail.len().saturating_sub(TURN_DONE.len() - 1));
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_new_agent_noticed_tasks_are_picked_up() {
        let new = r#"{"task_id": "task_1", "state": "needs_you", "created": true, "origin": "agent_noticed"}"#;
        assert_eq!(
            new_agent_task("task.updated", new).as_deref(),
            Some("task_1")
        );
        let refreshed = r#"{"task_id": "task_1", "state": "needs_you"}"#;
        assert_eq!(new_agent_task("task.updated", refreshed), None);
        let automation =
            r#"{"task_id": "task_2", "state": "working", "created": true, "origin": "automation"}"#;
        assert_eq!(new_agent_task("task.updated", automation), None);
        assert_eq!(new_agent_task("change_set.updated", new), None);
    }

    #[test]
    fn a_turn_stream_split_inside_characters_and_the_done_line_still_ends() {
        let stream = "event: message.delta\ndata: {\"text\": \"재고 — 주문\"}\n\nevent: turn.done\ndata: {}\n\n";
        let bytes = stream.as_bytes();
        // Every chunk size, so some chunk ends inside `—`, `재` and `turn.done`.
        for size in 1..=bytes.len() {
            let (mut tail, mut done) = (Vec::new(), false);
            for chunk in bytes.chunks(size) {
                done |= saw_turn_done(&mut tail, chunk);
            }
            assert!(done, "chunks of {size}");
            assert!(tail.len() < TURN_DONE.len());
        }
        let (mut tail, mut done) = (Vec::new(), false);
        for chunk in "event: message.delta\ndata: {\"text\": \"주문\"}\n"
            .as_bytes()
            .chunks(3)
        {
            done |= saw_turn_done(&mut tail, chunk);
        }
        assert!(!done);
    }
}
