//! `cargo xtask eval-run`: runs the task set against a running store, through the same chat API
//! the web uses, and records what the agent did (`eval/README.md` §4).
//!
//! Each task gets a new conversation; its turns are sent in order. Recorded per turn: the agent
//! the router chose, the plan steps, every tool call and its status, the reply, and the turn's
//! usage. Scored per task, without a model:
//!
//! - `required_called`: every `tools.required` tool was called (and finished).
//! - `forbidden_called`: `expect.must_not_call` tools that were called.
//! - `empty_plans`: turns whose plan had no step.
//! - `fact_numbers`: the numbers in `expect.facts` that appear in the replies or UI blocks. A rough signal
//!   for a person reading the transcript, not a grade: wording is not checked.
//!
//! Tasks that write (`kind` other than `read`) leave drafts in the store, so they run only with
//! `--writes`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::Instant;

use serde_json::{Value, json};

pub struct Options {
    pub base: String,
    pub tasks: String,
    pub only: Option<String>,
    pub writes: bool,
    pub out: Option<String>,
}

pub fn run(root: &Path, o: &Options) -> Result<Value, String> {
    let login = std::env::var("ACOM_EVAL_LOGIN").map_err(|_| "set ACOM_EVAL_LOGIN")?;
    let password = std::env::var("ACOM_EVAL_PASSWORD").map_err(|_| "set ACOM_EVAL_PASSWORD")?;
    let tasks = load_tasks(&root.join(&o.tasks))?;
    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    rt.block_on(async {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .map_err(|e| e.to_string())?;
        let session = sign_in(&http, &o.base, &login, &password).await?;
        let mut rows = Vec::new();
        for t in &tasks {
            let id = t["id"].as_str().unwrap_or_default();
            if o.only.as_deref().is_some_and(|only| !id.contains(only)) {
                continue;
            }
            let kind = t["kind"].as_str().unwrap_or("read");
            if kind != "read" && !o.writes {
                rows.push(json!({"id": id, "skipped": "writes (use --writes)"}));
                continue;
            }
            let row = run_task(&http, &o.base, &session, t).await;
            eprintln!(
                "{id}: required {} · empty plans {} · numbers {}",
                row["score"]["required_called"],
                row["score"]["empty_plans"],
                row["score"]["fact_numbers"]
            );
            rows.push(row);
        }
        Ok(json!({"base": o.base, "tasks": o.tasks, "runs": rows}))
    })
}

fn load_tasks(dir: &Path) -> Result<Vec<Value>, String> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yml"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let text = fs::read_to_string(&f).map_err(|e| e.to_string())?;
        let v: Value =
            serde_norway::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))?;
        out.extend(v.as_array().cloned().unwrap_or_default());
    }
    Ok(out)
}

async fn sign_in(
    http: &reqwest::Client,
    base: &str,
    login: &str,
    password: &str,
) -> Result<(String, String), String> {
    let r = http
        .post(format!("{base}/api/auth/login"))
        .header("content-type", "application/json")
        .body(json!({"login_id": login, "password": password}).to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("sign in failed: {}", r.status()));
    }
    let cookies: Vec<String> = r
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next().map(str::to_owned))
        .collect();
    if cookies.is_empty() {
        return Err("sign in set no cookie".into());
    }
    let body: Value = serde_json::from_str(&r.text().await.map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let csrf = body["csrf_token"]
        .as_str()
        .ok_or("sign in returned no csrf_token")?;
    Ok((cookies.join("; "), csrf.to_owned()))
}

async fn run_task(
    http: &reqwest::Client,
    base: &str,
    session: &(String, String),
    t: &Value,
) -> Value {
    let list = |p: &str| -> Vec<String> {
        t.pointer(p)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut conversation: Option<String> = None;
    let mut turns = Vec::new();
    for message in list("/turns") {
        let started = Instant::now();
        let turn = match send_turn(http, base, session, conversation.as_deref(), &message).await {
            Ok(events) => summarize(&events),
            Err(e) => json!({"error": e}),
        };
        if let Some(c) = turn["conversation_id"].as_str() {
            conversation = Some(c.to_owned());
        }
        let mut turn = turn;
        turn["message"] = json!(message);
        turn["wall_ms"] = json!(started.elapsed().as_millis() as u64);
        turns.push(turn);
    }

    let called: BTreeSet<String> = turns
        .iter()
        .flat_map(|t| t["tool_calls"].as_array().cloned().unwrap_or_default())
        .filter(|c| c["status"] == "done")
        .filter_map(|c| c["tool"].as_str().map(str::to_owned))
        .collect();
    let required = list("/tools/required");
    let forbidden: Vec<String> = list("/expect/must_not_call")
        .into_iter()
        .filter(|f| called.contains(f))
        .collect();
    let replies: String = turns
        .iter()
        .flat_map(|t| [t["reply"].as_str(), t["blocks_text"].as_str()])
        .flatten()
        .collect::<Vec<_>>()
        .join("\n");
    let expected: BTreeSet<String> = list("/expect/facts")
        .iter()
        .flat_map(|f| numbers(f))
        .collect();
    let seen: BTreeSet<String> = numbers(&replies).into_iter().collect();
    let found = expected.intersection(&seen).count();
    let empty_plans = turns
        .iter()
        .filter(|t| {
            t["plan_steps"]
                .as_array()
                .is_some_and(|p| p.iter().any(|n| n == 0))
        })
        .count();
    json!({
        "id": t["id"],
        "turns": turns,
        "score": {
            "required_called": required.iter().all(|r| called.contains(r)),
            "required_missing": required.iter().filter(|r| !called.contains(*r)).collect::<Vec<_>>(),
            "forbidden_called": forbidden,
            "empty_plans": empty_plans,
            "fact_numbers": format!("{found}/{}", expected.len()),
        },
    })
}

async fn send_turn(
    http: &reqwest::Client,
    base: &str,
    (cookie, csrf): &(String, String),
    conversation: Option<&str>,
    message: &str,
) -> Result<Vec<(String, Value)>, String> {
    let mut body = json!({"message": message});
    if let Some(c) = conversation {
        body["conversation_id"] = json!(c);
    }
    let r = http
        .post(format!("{base}/api/chat/bo/turns"))
        .header("cookie", cookie)
        .header("x-csrf-token", csrf)
        .header("accept", "text/event-stream")
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("turn failed: {}", r.status()));
    }
    let text = r.text().await.map_err(|e| e.to_string())?;
    Ok(parse_sse(&text))
}

/// `(event, data)` pairs of an SSE body.
fn parse_sse(text: &str) -> Vec<(String, Value)> {
    text.split("\n\n")
        .filter_map(|frame| {
            let mut event = None;
            let mut data = String::new();
            for line in frame.lines() {
                if let Some(e) = line.strip_prefix("event: ") {
                    event = Some(e.to_owned());
                } else if let Some(d) = line.strip_prefix("data: ") {
                    data.push_str(d);
                }
            }
            Some((event?, serde_json::from_str(&data).unwrap_or(Value::Null)))
        })
        .collect()
}

fn summarize(events: &[(String, Value)]) -> Value {
    let mut out = json!({"tool_calls": [], "plans": [], "plan_steps": [], "blocks": []});
    let mut blocks_text = String::new();
    let mut reply = String::new();
    for (name, d) in events {
        match name.as_str() {
            "turn.started" => {
                out["agent"] = d["agent"].clone();
                out["conversation_id"] = d["conversation_id"].clone();
            }
            "agent.step" if d["status"] != "running" => match d["kind"].as_str() {
                Some("call_tool") => push(
                    &mut out["tool_calls"],
                    json!({"tool": d["tool"], "status": d["status"]}),
                ),
                Some("plan") => {
                    let s = d["summary"].as_str().unwrap_or_default();
                    push(&mut out["plans"], json!(s));
                    if let Some(n) = plan_steps(s) {
                        push(&mut out["plan_steps"], json!(n));
                    }
                }
                Some("search_tools") => out["candidates"] = d["summary"].clone(),
                _ => {}
            },
            "ui.block" => {
                push(&mut out["blocks"], d["ui"].clone());
                blocks_text.push_str(&d.to_string());
                blocks_text.push('\n');
            }
            "message.done" => reply = d["text"].as_str().unwrap_or_default().to_owned(),
            "error" => out["error"] = d.clone(),
            "turn.done" => {
                out["usage"] = d["usage"].clone();
                out["models"] = d["models"].clone();
            }
            _ => {}
        }
    }
    out["reply"] = json!(reply);
    out["blocks_text"] = json!(blocks_text);
    out
}

fn push(arr: &mut Value, v: Value) {
    if let Some(a) = arr.as_array_mut() {
        a.push(v);
    }
}

/// `"… · 3 steps by Glimmer"` or `"3 steps by Glimmer: …"` → 3.
fn plan_steps(summary: &str) -> Option<u64> {
    let i = summary.find(" steps by ")?;
    summary[..i]
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// Numbers in a text, normalized: `$2,463.00` → `2463`, `4.6` → `4.6`.
fn numbers(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        let t = cur.trim_end_matches(['.', ',']).replace(',', "");
        let t = match t.split_once('.') {
            Some((a, b)) if b.bytes().all(|c| c == b'0') => a.to_owned(),
            _ => t,
        };
        if !t.is_empty() && t.bytes().any(|c| c.is_ascii_digit()) {
            out.push(t);
        }
        cur.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() || (!cur.is_empty() && (c == ',' || c == '.')) {
            cur.push(c);
        } else {
            flush(&mut cur, &mut out);
        }
    }
    flush(&mut cur, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_numbers() {
        assert_eq!(
            numbers("refund 9 for $2,463.00, average 4.6. Then 41."),
            vec!["9", "2463", "4.6", "41"]
        );
    }

    #[test]
    fn reads_plan_step_counts() {
        assert_eq!(
            plan_steps("find low stock · 2 steps by Muse Glimmer"),
            Some(2)
        );
        assert_eq!(plan_steps("0 steps by Muse Glimmer: "), Some(0));
        assert_eq!(plan_steps("Planning"), None);
    }

    #[test]
    fn parses_sse_frames() {
        let e = parse_sse(
            "id: 1\nevent: turn.started\ndata: {\"agent\":\"orders\"}\n\nid: 2\nevent: message.done\ndata: {\"text\":\"hi\"}\n\n",
        );
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].1["agent"], "orders");
        let s = summarize(&e);
        assert_eq!(
            (s["agent"].as_str(), s["reply"].as_str()),
            (Some("orders"), Some("hi"))
        );
    }
}
