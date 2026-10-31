//! `cargo xtask eval`: runs the agent task set (`eval/tasks/{design,held_out}/*.yml`, format in
//! eval/README.md §3) through a running A-Commerce and scores it (docs/design.md §7,
//! docs/backend/testing.md §3). `cargo xtask check-eval` checks the same files without running them.
//!
//! Each task signs in as its persona, sets the store clock to the tasks' clock, says its turns in
//! one conversation through the chat API (`POST /api/chat/{surface}/turns`, server-sent events),
//! then reads the conversation's turn log (`GET …/conversations/{id}?log=true`) for the tool and
//! model calls. A task is done when:
//! - every `tools.required` tool ran without an error and no `must_not_call` tool was called;
//! - the expected change set is there: its state (`drafting` also matches a set waiting for
//!   approval: neither is decided by a person yet), an item per expected item (tool, and every
//!   expected argument equal), and every expected failed check among its failed checks;
//! - every fact whose numbers can be checked shows them in the last turn's reply or blocks
//!   (`1,284` and `1284.00` match `1284`). Facts without numbers are listed for a person to judge.
//!
//! Tool precision is the share of calls to `required` or `allowed` tools; recall the share of
//! `required` tools reached. With `--repeat N`, pass^N is the share of tasks done in every run.
//! The facade has other tool names, so for it only facts decide, and tools are not scored.
//!
//! ```text
//! cargo xtask eval --base https://a-commerce.ailism.team --set design \
//!     [--tool-set acom|facade|both] [--repeat 3] [--reset] [--only id,id] [--out file]
//! ```
//!
//! `--reset` loads the demo store again (`POST /v1/platform/demo/reset`) before each task that
//! writes, so every run starts from the same store; it ends every session on that stack.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

/// The tasks' store clock (eval/README.md §6).
const CLOCK: &str = "2026-11-06T11:00";

/// One task (eval/README.md §3); fields the runner does not use are ignored.
#[derive(Debug, Clone, Deserialize)]
struct Task {
    id: String,
    persona: String,
    surface: String,
    kind: String,
    turns: Vec<String>,
    #[serde(default)]
    tools: Tools,
    #[serde(default)]
    expect: Expect,
    #[serde(default)]
    gaps: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Tools {
    #[serde(default)]
    required: Vec<String>,
    #[serde(default)]
    allowed: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Expect {
    #[serde(default)]
    change_set: Option<ExpectedSet>,
    #[serde(default)]
    facts: Vec<String>,
    #[serde(default)]
    must_not_call: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ExpectedSet {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    items: Vec<ExpectedItem>,
    #[serde(default)]
    checks_failed: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ExpectedItem {
    tool: String,
    #[serde(default)]
    args: serde_json::Map<String, Value>,
}

/// One run of one task with one tool set.
#[derive(Debug, Clone, Default)]
struct Run {
    task: String,
    tool_set: String,
    completed: bool,
    missing: Vec<String>,
    unchecked: usize,
    tools: Vec<(String, bool)>,
    model_calls: u64,
    input_tokens: u64,
    output_tokens: u64,
    ms: u128,
    precision: Option<f64>,
    recall: Option<f64>,
    /// The agent said its model is not answering: the run measures the outage, not the agent.
    model_down: bool,
}

/// How the agent runtime words a turn on which no model answered (crates/agent MODEL_DOWN).
const MODEL_DOWN: &str = "The assistant's model is not answering right now";
/// Runs in a row with the model down before the evaluation stops.
const STOP_AFTER_DOWN: usize = 2;

struct Client {
    http: reqwest::Client,
    base: String,
    cookies: BTreeMap<String, String>,
}

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) acom-eval";

impl Client {
    fn new(base: &str) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .user_agent(UA)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            cookies: BTreeMap::new(),
        })
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn keep_cookies(&mut self, r: &reqwest::Response) {
        for h in r.headers().get_all(reqwest::header::SET_COOKIE) {
            if let Ok(s) = h.to_str()
                && let Some((pair, _)) = s.split_once(';').or(Some((s, "")))
                && let Some((k, v)) = pair.split_once('=')
            {
                self.cookies
                    .insert(k.trim().to_owned(), v.trim().to_owned());
            }
        }
    }

    async fn send(
        &mut self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        accept: &str,
    ) -> Result<reqwest::Response, String> {
        let mut req = self
            .http
            .request(method.clone(), format!("{}{path}", self.base))
            .header("accept", accept)
            .header("cookie", self.cookie_header());
        if method != reqwest::Method::GET
            && let Some(csrf) = self.cookies.get("acom_csrf")
        {
            req = req.header("x-csrf-token", csrf);
        }
        if let Some(b) = body {
            req = req
                .header("content-type", "application/json")
                .body(b.to_string());
        }
        let r = req.send().await.map_err(|e| format!("{path}: {e}"))?;
        self.keep_cookies(&r);
        Ok(r)
    }

    async fn json(
        &mut self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let r = self.send(method, path, body, "application/json").await?;
        let status = r.status();
        let text = r.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "{path}: {status} {}",
                text.chars().take(300).collect::<String>()
            ));
        }
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
    }

    async fn sign_in(&mut self, persona: &str) -> Result<(), String> {
        self.cookies.clear();
        let (path, body) = match persona {
            "admin" => (
                "/api/auth/login",
                json!({"login_id": "admin", "password": "admin123"}),
            ),
            "guest" => ("/api/auth/guest", json!({})),
            p => ("/api/auth/demo-login", json!({"persona": p})),
        };
        self.json(reqwest::Method::POST, path, Some(body))
            .await
            .map(|_| ())
    }

    /// One turn: the conversation id, the reply text, and the blocks shown.
    async fn turn(
        &mut self,
        surface: &str,
        conversation: Option<&str>,
        text: &str,
        tool_set: &str,
    ) -> Result<(String, String, Vec<Value>), String> {
        let mut body = json!({"message": text});
        if let Some(c) = conversation {
            body["conversation_id"] = json!(c);
        }
        if tool_set != "acom" {
            body["tool_set"] = json!(tool_set);
        }
        let mut r = self
            .send(
                reqwest::Method::POST,
                &format!("/api/chat/{surface}/turns"),
                Some(body),
                "text/event-stream",
            )
            .await?;
        if !r.status().is_success() {
            let status = r.status();
            let t = r.text().await.unwrap_or_default();
            return Err(format!(
                "turn: {status} {}",
                t.chars().take(300).collect::<String>()
            ));
        }
        let (mut conv, mut reply, mut blocks) = (
            conversation.unwrap_or_default().to_owned(),
            String::new(),
            Vec::new(),
        );
        let (mut pending, mut event) = (String::new(), String::new());
        'read: while let Some(chunk) = r.chunk().await.map_err(|e| e.to_string())? {
            pending.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = pending.find('\n') {
                let line: String = pending.drain(..=end).collect();
                let line = line.trim_end();
                if let Some(e) = line.strip_prefix("event:") {
                    e.trim().clone_into(&mut event);
                } else if let Some(d) = line.strip_prefix("data:") {
                    let Ok(v) = serde_json::from_str::<Value>(d.trim()) else {
                        continue;
                    };
                    match event.as_str() {
                        "turn.started" => {
                            if let Some(c) = v["conversation_id"].as_str() {
                                c.clone_into(&mut conv);
                            }
                        }
                        "message.done" => reply = v["text"].as_str().unwrap_or_default().to_owned(),
                        "ui.block" => blocks.push(v),
                        "error" => return Err(format!("turn error: {v}")),
                        "turn.done" => break 'read,
                        _ => {}
                    }
                }
            }
        }
        Ok((conv, reply, blocks))
    }
}

/// `1,284.00` → `1284`; words lowercased.
fn normalized(s: &str) -> String {
    let lower = s.to_lowercase().replace(',', "");
    let mut out = String::new();
    let mut rest = lower.as_str();
    while !rest.is_empty() {
        let digits: String = rest
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if digits.is_empty() {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            rest = &rest[c.len_utf8()..];
        } else {
            let n = digits.trim_end_matches('.');
            let n = if n.contains('.') {
                n.trim_end_matches('0').trim_end_matches('.')
            } else {
                n
            };
            out.push_str(n);
            rest = &rest[digits.len()..];
        }
    }
    out
}

async fn run_task(c: &mut Client, task: &Task, tool_set: &str, reset: bool) -> Run {
    let mut run = Run {
        task: task.id.clone(),
        tool_set: tool_set.to_owned(),
        ..Run::default()
    };
    let started = Instant::now();
    let result: Result<(), String> = async {
        c.sign_in("adm_admin").await?;
        if reset && task.kind != "read" {
            c.json(reqwest::Method::POST, "/api/v1/platform/demo/reset", None)
                .await?;
            c.sign_in("adm_admin").await?;
        }
        c.json(
            reqwest::Method::PUT,
            "/api/v1/platform/demo-clock",
            Some(json!({"now": CLOCK, "is_running": false})),
        )
        .await?;
        c.sign_in(&task.persona).await?;
        let mut conv: Option<String> = None;
        let (mut reply, mut blocks) = (String::new(), Vec::new());
        for line in &task.turns {
            let (id, r, b) = c
                .turn(&task.surface, conv.as_deref(), line, tool_set)
                .await?;
            conv = Some(id);
            reply = r;
            blocks = b;
        }
        run.model_down = reply.starts_with(MODEL_DOWN);
        let conv = conv.ok_or("no conversation")?;
        let log = c
            .json(
                reqwest::Method::GET,
                &format!("/api/chat/{}/conversations/{conv}?log=true", task.surface),
                None,
            )
            .await?;
        if let Some(dir) = arg("--logs") {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(
                format!("{dir}/{}-{tool_set}-{conv}.json", task.id),
                serde_json::to_string_pretty(
                    &json!({"task": task.id, "reply": reply, "blocks": blocks, "log": log}),
                )
                .unwrap_or_default(),
            );
        }
        for t in log["turns"].as_array().into_iter().flatten() {
            for s in t["log"]["steps"].as_array().into_iter().flatten() {
                if s["kind"] == "call_tool"
                    && let Some(tool) = s["tool"].as_str()
                {
                    run.tools
                        .push((tool.to_owned(), s["status"] == "completed"));
                }
            }
            for m in t["log"]["model_calls"].as_array().into_iter().flatten() {
                run.model_calls += 1;
                run.input_tokens += m["input_tokens"].as_u64().unwrap_or(0);
                run.output_tokens += m["output_tokens"].as_u64().unwrap_or(0);
            }
        }
        let change_set = match &task.expect.change_set {
            Some(_) if tool_set == "acom" => {
                let id = blocks
                    .iter()
                    .find_map(|b| b["data"]["change_set_id"].as_str().map(str::to_owned));
                match id {
                    Some(id) => Some(
                        c.json(
                            reqwest::Method::GET,
                            &format!("/api/v1/platform/change-sets/{id}"),
                            None,
                        )
                        .await?,
                    ),
                    None => None,
                }
            }
            _ => None,
        };
        score(
            task,
            tool_set,
            &reply,
            &blocks,
            change_set.as_ref(),
            &mut run,
        );
        Ok(())
    }
    .await;
    run.ms = started.elapsed().as_millis();
    if let Err(e) = result {
        run.missing.push(e);
        run.completed = false;
    }
    run
}

/// Numbers in a sentence, normalized (`$2,463.00` → `2463`, `4.6` stays).
fn numbers(text: &str) -> Vec<String> {
    let t = text.replace(',', "");
    let mut out = Vec::new();
    let chars: Vec<char> = t.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len()
            && (chars[i].is_ascii_digit()
                || (chars[i] == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)))
        {
            i += 1;
        }
        out.push(normalized(&chars[start..i].iter().collect::<String>()));
    }
    out
}

fn score(
    task: &Task,
    tool_set: &str,
    reply: &str,
    blocks: &[Value],
    change_set: Option<&Value>,
    run: &mut Run,
) {
    let ok = |t: &str| run.tools.iter().any(|(name, done)| name == t && *done);
    if tool_set == "acom" {
        for t in &task.tools.required {
            if !ok(t) {
                run.missing.push(format!("tool {t}"));
            }
        }
        let fitting: Vec<&String> = task
            .tools
            .required
            .iter()
            .chain(&task.tools.allowed)
            .collect();
        if !run.tools.is_empty() {
            let hits = run
                .tools
                .iter()
                .filter(|(t, _)| fitting.contains(&t))
                .count();
            run.precision = Some(hits as f64 / run.tools.len() as f64);
        }
        if !task.tools.required.is_empty() {
            let reached = task.tools.required.iter().filter(|t| ok(t)).count();
            run.recall = Some(reached as f64 / task.tools.required.len() as f64);
        }
        if let Some(want) = &task.expect.change_set {
            match change_set {
                None => run.missing.push("change set".into()),
                Some(cs) => {
                    // "drafting" means not yet decided by a person; a set the agent submitted
                    // for approval is that too.
                    let is = cs["state"].as_str().unwrap_or_default();
                    let same = |want: &str| {
                        is == want || (want == "drafting" && is == "waiting_for_approval")
                    };
                    if let Some(state) = &want.state
                        && !same(state)
                    {
                        run.missing
                            .push(format!("change set state {state} (is {})", cs["state"]));
                    }
                    let items = cs["items"].as_array().cloned().unwrap_or_default();
                    for w in &want.items {
                        let found = items.iter().any(|i| {
                            i["tool"] == w.tool.as_str()
                                && w.args.iter().all(|(k, v)| i["input"].get(k) == Some(v))
                        });
                        if !found {
                            run.missing.push(format!(
                                "change set item {} {}",
                                w.tool,
                                Value::Object(w.args.clone())
                            ));
                        }
                    }
                    let failed: Vec<String> = items
                        .iter()
                        .flat_map(|i| i["checks"].as_array().cloned().unwrap_or_default())
                        .filter(|c| c["passed"] == false)
                        .filter_map(|c| c["rule"].as_str().map(str::to_owned))
                        .collect();
                    for r in &want.checks_failed {
                        if !failed.contains(r) {
                            run.missing.push(format!("failed check {r}"));
                        }
                    }
                }
            }
        }
    }
    for t in &task.expect.must_not_call {
        if run.tools.iter().any(|(name, _)| name == t) {
            run.missing.push(format!("called {t}"));
        }
    }
    let shown = normalized(&format!("{reply} {}", Value::Array(blocks.to_vec())));
    for fact in &task.expect.facts {
        let ns = numbers(fact);
        if ns.is_empty() {
            run.unchecked += 1;
            continue;
        }
        let absent: Vec<&String> = ns.iter().filter(|n| !shown.contains(n.as_str())).collect();
        if !absent.is_empty() {
            run.missing.push(format!(
                "fact \"{}\" ({} not shown)",
                fact.chars().take(60).collect::<String>(),
                absent
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    run.completed = run.missing.is_empty();
}

fn mean(xs: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = xs.collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

fn pct(x: Option<f64>) -> String {
    x.map_or_else(|| "—".to_owned(), |x| format!("{:.0}%", x * 100.0))
}

fn report(set: &str, base: &str, runs: &[Run], repeat: usize) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Evaluation: {set} set\n");
    let _ = writeln!(
        out,
        "Against {base}, {repeat} run(s) per task and tool set, store clock {CLOCK}. Generated by `cargo xtask eval`; method in eval/README.md.\n"
    );
    let mut sets: Vec<String> = runs.iter().map(|r| r.tool_set.clone()).collect();
    sets.sort();
    sets.dedup();
    let _ = writeln!(
        out,
        "| Tool set | Done | pass^{repeat} | Tool precision | Tool recall | Tool calls / task | Model calls / task | Tokens / task | Seconds / task |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|");
    for set in &sets {
        let rs: Vec<&Run> = runs.iter().filter(|r| &r.tool_set == set).collect();
        let n = rs.len().max(1) as f64;
        let mut by: BTreeMap<&str, Vec<&Run>> = BTreeMap::new();
        for r in &rs {
            by.entry(r.task.as_str()).or_default().push(r);
        }
        let pass_k = by
            .values()
            .filter(|v| v.iter().all(|r| r.completed))
            .count() as f64
            / by.len().max(1) as f64;
        let _ = writeln!(
            out,
            "| {set} | {} | {} | {} | {} | {:.1} | {:.1} | {:.0} | {:.1} |",
            pct(Some(rs.iter().filter(|r| r.completed).count() as f64 / n)),
            pct(Some(pass_k)),
            pct(mean(rs.iter().filter_map(|r| r.precision))),
            pct(mean(rs.iter().filter_map(|r| r.recall))),
            rs.iter().map(|r| r.tools.len()).sum::<usize>() as f64 / n,
            rs.iter().map(|r| r.model_calls).sum::<u64>() as f64 / n,
            rs.iter()
                .map(|r| r.input_tokens + r.output_tokens)
                .sum::<u64>() as f64
                / n,
            rs.iter().map(|r| r.ms).sum::<u128>() as f64 / n / 1000.0,
        );
    }
    let _ = writeln!(out, "\n## Runs\n");
    let _ = writeln!(
        out,
        "| Task | Tool set | Done | Tools called | Model calls | Tokens | Seconds | Facts to judge | Missing |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|---|---|");
    for r in runs {
        let tools = r
            .tools
            .iter()
            .map(|(t, ok)| if *ok { t.clone() } else { format!("{t} ✗") })
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} | {:.1} | {} | {} |",
            r.task,
            r.tool_set,
            if r.completed { "yes" } else { "no" },
            if tools.is_empty() {
                "—".into()
            } else {
                tools
            },
            r.model_calls,
            r.input_tokens + r.output_tokens,
            r.ms as f64 / 1000.0,
            r.unchecked,
            r.missing.join("; ").replace('|', "/"),
        );
    }
    out
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn load(root: &Path, set: &str) -> Result<Vec<Task>, String> {
    let dir = root.join("eval/tasks").join(set);
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yml"))
        .collect();
    files.sort();
    let mut tasks = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        let mut list: Vec<Task> =
            serde_norway::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))?;
        tasks.append(&mut list);
    }
    Ok(tasks)
}

pub fn eval(root: &Path) -> ExitCode {
    let Some(base) = arg("--base") else {
        eprintln!(
            "usage: cargo xtask eval --base <gateway url> [--set design|held_out] [--tool-set acom|facade|both] [--repeat N] [--reset] [--only id,id] [--ready] [--out file]"
        );
        return ExitCode::FAILURE;
    };
    let set = arg("--set").unwrap_or_else(|| "design".into());
    let sets: Vec<String> = match arg("--tool-set").as_deref() {
        None | Some("acom") => vec!["acom".into()],
        Some("facade") => vec!["facade".into()],
        Some("both") => vec!["acom".into(), "facade".into()],
        Some(other) => {
            eprintln!("unknown tool set {other}");
            return ExitCode::FAILURE;
        }
    };
    let repeat: usize = arg("--repeat")
        .and_then(|n| n.parse().ok())
        .unwrap_or(1)
        .max(1);
    let reset = std::env::args().any(|a| a == "--reset");
    // Tasks that record a data gap expect "there is no data" answers; --ready leaves them out.
    let ready_only = std::env::args().any(|a| a == "--ready");
    let only: Option<Vec<String>> =
        arg("--only").map(|s| s.split(',').map(str::to_owned).collect());
    let tasks = match load(root, &set) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let chosen: Vec<&Task> = tasks
        .iter()
        .filter(|t| only.as_ref().is_none_or(|o| o.contains(&t.id)))
        .filter(|t| !ready_only || t.gaps.is_empty())
        .collect();
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let runs = rt.block_on(async {
        let mut c = match Client::new(&base) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{e}");
                return Vec::new();
            }
        };
        let mut runs = Vec::new();
        let mut down = 0;
        'all: for ts in &sets {
            for task in &chosen {
                for n in 0..repeat {
                    let r = run_task(&mut c, task, ts, reset).await;
                    down = if r.model_down { down + 1 } else { 0 };
                    eprintln!(
                        "{} [{ts}] run {}: {} — {} tool call(s), {} model call(s), {:.1}s{}",
                        task.id,
                        n + 1,
                        if r.completed { "done" } else { "not done" },
                        r.tools.len(),
                        r.model_calls,
                        r.ms as f64 / 1000.0,
                        if r.missing.is_empty() {
                            String::new()
                        } else {
                            format!(" — missing {}", r.missing.join("; "))
                        }
                    );
                    runs.push(r);
                    if down >= STOP_AFTER_DOWN {
                        break 'all;
                    }
                }
            }
        }
        runs
    });
    if runs
        .iter()
        .rev()
        .take(STOP_AFTER_DOWN)
        .filter(|r| r.model_down)
        .count()
        == STOP_AFTER_DOWN
    {
        eprintln!(
            "The model is not answering ({STOP_AFTER_DOWN} runs in a row); stopped after {} run(s). No report written: these runs measure the outage, not the agent.",
            runs.len()
        );
        return ExitCode::FAILURE;
    }
    let md = report(&set, &base, &runs, repeat);
    match arg("--out") {
        Some(path) => {
            if let Some(dir) = Path::new(&path).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, &md) {
                eprintln!("{path}: {e}");
                // A run against the stack takes minutes; print the report rather than lose it.
                println!("{md}");
                return ExitCode::FAILURE;
            }
            println!("wrote {path}");
        }
        None => println!("{md}"),
    }
    if runs.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(yaml: &str) -> Task {
        serde_norway::from_str(yaml).unwrap()
    }

    #[test]
    fn numbers_in_facts_are_found_as_people_write_them() {
        assert_eq!(numbers("41 reviews, average 4.6"), ["41", "4.6"]);
        assert_eq!(numbers("refund $2,463.00 for 9"), ["2463", "9"]);
        let shown = normalized("Here are 41 reviews (4.6 stars) and $2,463.00 for 9 refunds");
        assert!(
            numbers("41 reviews, average 4.6")
                .iter()
                .all(|n| shown.contains(n.as_str()))
        );
    }

    #[test]
    fn a_run_is_done_only_when_tools_change_set_and_facts_hold() {
        let t = task(
            r#"
id: t
persona: adm_sj
surface: bo
kind: write_approval
turns: ["x"]
tools: {required: [set_sale_prices], allowed: [search_products]}
expect:
  change_set:
    state: drafting
    items: [{tool: set_sale_prices, args: {category_id: cat_tees}}]
    checks_failed: [max_discount]
  facts: ["25% is over the 20% limit", "the agent says which rule stopped it"]
  must_not_call: [set_price]
"#,
        );
        let cs = json!({"state": "drafting", "items": [{"tool": "set_sale_prices",
            "input": {"category_id": "cat_tees", "percent_off": 25},
            "checks": [{"rule": "max_discount", "passed": false}]}]});
        let mut run = Run {
            tools: vec![
                ("search_products".into(), true),
                ("set_sale_prices".into(), true),
            ],
            ..Run::default()
        };
        score(
            &t,
            "acom",
            "25% is over the 20% limit for everyone.",
            &[],
            Some(&cs),
            &mut run,
        );
        assert!(run.completed, "{:?}", run.missing);
        assert_eq!(run.unchecked, 1);
        assert_eq!((run.precision, run.recall), (Some(1.0), Some(1.0)));

        let mut run = Run {
            tools: vec![("set_price".into(), true)],
            ..Run::default()
        };
        score(&t, "acom", "Done.", &[], None, &mut run);
        assert!(!run.completed);
        assert_eq!(run.missing.len(), 4, "{:?}", run.missing);
    }
}
