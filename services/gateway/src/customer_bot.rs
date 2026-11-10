//! Ailism Customer Bot's workers (docs/api/platform.md#customer_bot): the shoppers that keep a
//! demo store busy. Every half minute the gateway asks commerce for the run to work on (a queued
//! one, or a scheduled one whose real minutes have passed). Each of its sessions shops at the same
//! time as one member, through this gateway's own front door like a browser: it signs up or signs
//! in with the demo sign-in, opens the storefront's first page, makes the member's persona when
//! there is none, writes to the shopping assistant, presses the person-only buttons a shopper
//! presses (Place order) and reviews what was delivered. What a person would decide comes from
//! the agent service's shopper model; when it cannot answer, a short plain plan takes over.
//!
//! Every step is recorded to commerce as it happens, with the system subject `sys_customer_bot`.
//! The shopper's own requests carry only the member's session cookie, so they get exactly the
//! member's scopes and leave the member's own trail in the action log. Demo mode only.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use acom_auth::Principal;
use serde_json::{Map, Value, json};

use crate::session::{CSRF_COOKIE, SESSION_COOKIE};
use crate::{Gateway, REQUEST_TOKEN_SECONDS, now_seconds};

/// The subject commerce and the agent see for the workers.
const SUBJECT: &str = "sys_customer_bot";
/// How often the workers ask for a run.
const TICK: Duration = Duration::from_secs(30);
/// The browser the sessions say they are.
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AilismCustomerBot/1.0";
/// A chat turn ends within the agent's limit; this only stops a stream that never ends.
const CHAT_TIMEOUT: Duration = Duration::from_secs(180);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const THINK_TIMEOUT: Duration = Duration::from_secs(150);
/// Characters of a reply the shopper's model reads back in the transcript.
const REPLY_CHARS: usize = 700;

/// Starts the workers. Off unless the gateway runs a demo store.
pub fn spawn(gw: Arc<Gateway>) {
    if !gw.sessions.demo_mode {
        tracing::info!("customer bot: off (not a demo store)");
        return;
    }
    tokio::spawn(async move {
        let http = reqwest::Client::new();
        loop {
            if let Err(e) = tick(&gw, &http).await {
                tracing::warn!(error = %e, "customer bot");
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

fn system_token(gw: &Gateway, request_id: &str) -> Result<String, String> {
    let p = Principal::system(gw.sessions.tenant.clone(), SUBJECT);
    gw.signer
        .sign(&p, request_id, now_seconds(), REQUEST_TOKEN_SECONDS)
        .map_err(|e| e.to_string())
}

/// A call to commerce with the workers' system token.
async fn commerce(
    gw: &Gateway,
    http: &reqwest::Client,
    method: reqwest::Method,
    path: &str,
    body: &Value,
) -> Result<Value, String> {
    let token = system_token(gw, "req_customer_bot")?;
    let r = http
        .request(method, format!("{}{path}", gw.commerce))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .timeout(REQUEST_TIMEOUT)
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| format!("{path}: {e}"))?;
    let status = r.status();
    let text = r.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "{path}: {status} {}",
            text.chars().take(300).collect::<String>()
        ));
    }
    serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
}

/// One look for work: the run commerce gives, its sessions shopping at the same time.
async fn tick(gw: &Arc<Gateway>, http: &reqwest::Client) -> Result<(), String> {
    let next = commerce(
        gw,
        http,
        reqwest::Method::POST,
        "/v1/platform/customer-bot/runs/next",
        &json!({}),
    )
    .await?;
    let Some(run) = next.get("run").filter(|r| r.is_object()) else {
        return Ok(());
    };
    let run_id = run["id"].as_str().unwrap_or_default().to_owned();
    let settings = run["settings"].clone();
    let sessions = run["sessions"].as_array().cloned().unwrap_or_default();
    tracing::info!(run = %run_id, sessions = sessions.len(), "customer bot: run");
    let mut set = tokio::task::JoinSet::new();
    for (i, s) in sessions.into_iter().enumerate() {
        let (gw, http, settings) = (gw.clone(), http.clone(), settings.clone());
        set.spawn(async move {
            // Shoppers do not all walk in at the same second.
            tokio::time::sleep(Duration::from_secs(3 * i as u64)).await;
            shop(&gw, &http, &settings, &s).await;
        });
    }
    while let Some(done) = set.join_next().await {
        if let Err(e) = done {
            tracing::error!(error = %e, run = %run_id, "customer bot: a session panicked");
        }
    }
    Ok(())
}

// ---- the browser ---------------------------------------------------------------------------

/// What came back for one request.
struct Reply {
    status: u16,
    body: Value,
    ms: u64,
}

impl Reply {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The error's code and sentence, or the status.
    fn why(&self) -> String {
        let e = &self.body["error"];
        match (e["code"].as_str(), e["message"].as_str()) {
            (Some(c), Some(m)) => format!("{c}: {m}"),
            _ => format!("HTTP {}", self.status),
        }
    }
}

/// A browser on the storefront: its cookies, and its requests through the gateway's own routes.
struct Browser {
    http: reqwest::Client,
    base: String,
    session: Option<String>,
    csrf: Option<String>,
}

fn millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl Browser {
    fn new(http: &reqwest::Client, base: &str) -> Self {
        Self {
            http: http.clone(),
            base: base.to_owned(),
            session: None,
            csrf: None,
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut cookies = Vec::new();
        if let Some(s) = &self.session {
            cookies.push(format!("{SESSION_COOKIE}={s}"));
        }
        if let Some(c) = &self.csrf {
            cookies.push(format!("{CSRF_COOKIE}={c}"));
        }
        let writes = method != reqwest::Method::GET;
        let mut r = self
            .http
            .request(method, format!("{}{path}", self.base))
            .header("user-agent", USER_AGENT);
        if !cookies.is_empty() {
            r = r.header("cookie", cookies.join("; "));
        }
        if writes && let Some(c) = &self.csrf {
            r = r.header("x-csrf-token", c);
        }
        if path.starts_with("/api/v1/") {
            r = r.header("x-acom-surface", "fo");
        }
        r
    }

    /// Keeps the cookies a response sets (a cleared one is forgotten).
    fn remember(&mut self, headers: &reqwest::header::HeaderMap) {
        for v in headers.get_all(reqwest::header::SET_COOKIE) {
            let Ok(v) = v.to_str() else { continue };
            let Some((name, rest)) = v.split_once('=') else {
                continue;
            };
            let value = rest.split(';').next().unwrap_or_default().trim().to_owned();
            let value = Some(value).filter(|v| !v.is_empty());
            match name.trim() {
                n if n == SESSION_COOKIE => self.session = value,
                n if n == CSRF_COOKIE => self.csrf = value,
                _ => {}
            }
        }
    }

    async fn call(&mut self, method: reqwest::Method, path: &str, body: Option<&Value>) -> Reply {
        let started = Instant::now();
        let mut r = self
            .request(method, path)
            .header("accept", "application/json")
            .timeout(REQUEST_TIMEOUT);
        if let Some(b) = body {
            r = r
                .header("content-type", "application/json")
                .body(b.to_string());
        }
        match r.send().await {
            Ok(res) => {
                self.remember(res.headers());
                let status = res.status().as_u16();
                let text = res.text().await.unwrap_or_default();
                let body = serde_json::from_str(&text).unwrap_or_else(
                    |_| json!({"text": text.chars().take(300).collect::<String>()}),
                );
                Reply {
                    status,
                    body,
                    ms: millis(started),
                }
            }
            Err(e) => Reply {
                status: 0,
                body: json!({"error": {"code": "unreachable", "message": e.to_string()}}),
                ms: millis(started),
            },
        }
    }

    async fn get(&mut self, path: &str) -> Reply {
        self.call(reqwest::Method::GET, path, None).await
    }

    async fn post(&mut self, path: &str, body: &Value) -> Reply {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }

    /// A storefront page as a browser opens it: the HTML, and its title.
    async fn page(&mut self, path: &str) -> (u16, String, usize, u64) {
        let started = Instant::now();
        let r = self
            .request(reqwest::Method::GET, path)
            .header("accept", "text/html")
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await;
        match r {
            Ok(res) => {
                self.remember(res.headers());
                let status = res.status().as_u16();
                let html = res.text().await.unwrap_or_default();
                let title = html
                    .split_once("<title>")
                    .and_then(|(_, t)| t.split_once("</title>"))
                    .map(|(t, _)| t.trim().to_owned())
                    .unwrap_or_default();
                (status, title, html.len(), millis(started))
            }
            Err(_) => (0, String::new(), 0, millis(started)),
        }
    }

    /// One storefront chat turn over the event stream, read to its end.
    async fn chat(&mut self, conversation: Option<&str>, message: &str) -> Turn {
        let started = Instant::now();
        let mut body = json!({"message": message});
        if let Some(c) = conversation {
            body["conversation_id"] = json!(c);
        }
        let mut turn = Turn::default();
        let res = self
            .request(reqwest::Method::POST, "/api/chat/fo/turns")
            .header("accept", "text/event-stream")
            .header("content-type", "application/json")
            .timeout(CHAT_TIMEOUT)
            .body(body.to_string())
            .send()
            .await;
        let text = match res {
            Ok(r) => {
                self.remember(r.headers());
                turn.status = r.status().as_u16();
                r.text().await.unwrap_or_default()
            }
            Err(e) => {
                turn.error = Some(e.to_string());
                String::new()
            }
        };
        if turn.status != 0 && !(200..300).contains(&turn.status) {
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            turn.error = Some(
                v["error"]["message"]
                    .as_str()
                    .map_or_else(|| format!("HTTP {}", turn.status), str::to_owned),
            );
        }
        turn.read(&text);
        turn.ms = millis(started);
        turn
    }
}

/// What one chat turn showed.
#[derive(Default)]
struct Turn {
    status: u16,
    conversation_id: Option<String>,
    reply: String,
    blocks: Vec<Value>,
    tools: Vec<String>,
    memories: Vec<String>,
    usage: Value,
    error: Option<String>,
    ms: u64,
}

impl Turn {
    /// Reads the event stream: the reply, the blocks as they stand at the end, the tools that
    /// ran, what was remembered.
    fn read(&mut self, stream: &str) {
        let mut event = String::new();
        let mut deltas = String::new();
        for line in stream.lines() {
            if let Some(e) = line.strip_prefix("event:") {
                e.trim().clone_into(&mut event);
                continue;
            }
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let v: Value = serde_json::from_str(data.trim()).unwrap_or(Value::Null);
            match event.as_str() {
                "turn.started" => {
                    self.conversation_id = v["conversation_id"].as_str().map(str::to_owned);
                }
                "message.delta" => deltas.push_str(v["text"].as_str().unwrap_or_default()),
                "message.reset" => deltas.clear(),
                "message.done" => {
                    self.reply = v["text"].as_str().unwrap_or_default().to_owned();
                }
                "ui.block" => {
                    let id = v["block_id"].clone();
                    self.blocks.retain(|b| b["block_id"] != id);
                    self.blocks.push(v);
                }
                "ui.block.removed" => {
                    let id = v["block_id"].clone();
                    self.blocks.retain(|b| b["block_id"] != id);
                }
                "agent.step" => {
                    if v["status"] == "done"
                        && let Some(t) = v["tool"].as_str()
                        && !self.tools.iter().any(|x| x == t)
                    {
                        self.tools.push(t.to_owned());
                    }
                }
                "memory.saved" => {
                    self.memories
                        .push(v["text"].as_str().unwrap_or_default().to_owned());
                }
                "error" => {
                    self.error = Some(
                        v["message"]
                            .as_str()
                            .unwrap_or("The turn failed.")
                            .to_owned(),
                    );
                }
                "turn.done" => self.usage = v["usage"].clone(),
                _ => {}
            }
        }
        if self.reply.is_empty() {
            self.reply = deltas;
        }
    }

    fn block(&self, ui: &str) -> Option<&Value> {
        self.blocks.iter().rev().find(|b| b["ui"] == ui)
    }
}

/// A money value as cents, from `{value}` or a bare number or text.
fn cents(v: &Value) -> Option<i64> {
    let raw = match v.get("value").unwrap_or(v) {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.trim().trim_start_matches('$').replace(',', ""),
        _ => return None,
    };
    let (whole, frac) = raw.split_once('.').unwrap_or((&raw, "0"));
    let whole: i64 = whole.parse().ok()?;
    let frac: String = frac.chars().chain("00".chars()).take(2).collect();
    Some(whole * 100 + frac.parse::<i64>().ok()?)
}

fn money_text(c: i64) -> String {
    format!("${}.{:02}", c / 100, c % 100)
}

/// A block as the shopper's model reads it, and as the step keeps it: what it shows, not how.
fn summary(b: &Value) -> Value {
    let d = &b["data"];
    let ui = b["ui"].as_str().unwrap_or_default();
    let price = |v: &Value| cents(v).map(money_text);
    let products = |items: &Value| -> Vec<Value> {
        items
            .as_array()
            .into_iter()
            .flatten()
            .take(8)
            .map(|i| json!({"product_id": i["product_id"], "name": i["name"], "price": price(&i["price"])}))
            .collect()
    };
    let mut out = json!({"ui": ui, "title": b["title"]});
    match ui {
        "product_list" | "look_gallery" => out["products"] = json!(products(&d["items"])),
        "compare_table" => out["products"] = json!(products(&d["products"])),
        "product_detail" => {
            out["product"] = json!({"product_id": d["product_id"], "name": d["name"], "price": price(&d["price"])});
        }
        "cart" => {
            out["lines"] = json!(
                d["lines"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|l| json!({"name": l["name"], "color": l["color"], "size": l["size"], "quantity": l["quantity"]}))
                    .collect::<Vec<_>>()
            );
            out["total"] = json!(price(&d["total"]));
        }
        "checkout_card" => {
            out["total"] = json!(price(&d["total"]));
            out["preview_id"] = d["preview_id"].clone();
            out["failed_checks"] = json!(
                d["checks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["passed"] == false)
                    .map(|c| c["detail"].clone())
                    .collect::<Vec<_>>()
            );
        }
        "fit_check" => {
            out["options"] = json!(
                d["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|o| json!({"size": o["size"], "recommended": o["recommended"]}))
                    .collect::<Vec<_>>()
            );
        }
        "record_list" => {
            out["rows"] = json!(
                d["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(5)
                    .map(|r| r["title"].clone())
                    .collect::<Vec<_>>()
            );
        }
        _ => {}
    }
    out
}

// ---- the shopper's decisions -------------------------------------------------------------

/// One decision of the shopper's model, with what it cost.
async fn think(
    gw: &Gateway,
    http: &reqwest::Client,
    model: &str,
    task: &str,
    input: Value,
) -> Result<(Value, Value), String> {
    let token = system_token(gw, "req_customer_bot_think")?;
    let r = http
        .post(format!("{}/customer-bot/think", gw.agent))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .timeout(THINK_TIMEOUT)
        .body(json!({"task": task, "model": model, "input": input}).to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = r.status();
    let text = r.text().await.unwrap_or_default();
    let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(v["error"]["message"]
            .as_str()
            .map_or_else(|| format!("HTTP {status}"), str::to_owned));
    }
    Ok((v["output"].clone(), v["usage"].clone()))
}

/// A small number from text, the same each time for the same text.
fn dice(seed: &str, n: usize) -> usize {
    let h = seed.bytes().fold(1_469_598_103_934_665_603_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(1_099_511_628_211)
    });
    usize::try_from(h % (n.max(1) as u64)).unwrap_or(0)
}

fn pick<'a>(seed: &str, from: &'a [&'a str]) -> &'a str {
    from[dice(seed, from.len())]
}

/// A new shopper when the model cannot make one up.
fn plain_identity(seed: &str) -> Value {
    let first = pick(
        &format!("{seed}f"),
        &[
            "Maya", "Ethan", "Chloe", "Lucas", "Ava", "Noah", "Priya", "Daniel", "Sofia", "Marcus",
            "Hana", "Leo",
        ],
    );
    let last = pick(
        &format!("{seed}l"),
        &[
            "Carter", "Nguyen", "Patel", "Brooks", "Kim", "Rivera", "Hughes", "Moreno", "Foster",
            "Lee", "Bennett", "Shah",
        ],
    );
    let (city, state, zip) = [
        ("Portland", "OR", "97205"),
        ("Austin", "TX", "78704"),
        ("Denver", "CO", "80205"),
        ("Chicago", "IL", "60614"),
        ("Boston", "MA", "02116"),
        ("Seattle", "WA", "98103"),
        ("Atlanta", "GA", "30308"),
        ("Minneapolis", "MN", "55408"),
    ][dice(&format!("{seed}c"), 8)];
    json!({"name": format!("{first} {last}"),
           "email_local": format!("{}.{}", first.to_lowercase(), last.to_lowercase()),
           "street_address": format!("{} {} St", 10 + dice(&format!("{seed}n"), 880), pick(&format!("{seed}s"), &["Oak", "Maple", "Pine", "Cedar", "Elm", "Lake", "Hill"])),
           "address_locality": city, "address_region": state, "postal_code": zip,
           "telephone": format!("555-01{:02}", dice(&format!("{seed}t"), 100))})
}

/// A persona when the model cannot make one.
fn plain_persona(seed: &str) -> Value {
    let category = pick(
        &format!("{seed}k"),
        &["outerwear", "shirts", "knitwear", "tees", "pants"],
    );
    let color = pick(
        &format!("{seed}o"),
        &["navy", "olive", "black", "sand", "stone"],
    );
    let size = pick(&format!("{seed}z"), &["S", "M", "M", "L", "XL"]);
    let budget = [60, 90, 120, 150, 200][dice(&format!("{seed}b"), 5)];
    json!({"description": format!("Shopper who buys {color} {category}, size {size}, about ${budget} an order."),
           "age_range": "30-39", "location": "United States", "occupation": "office worker",
           "style": ["casual"], "colors": [color], "sizes": {"top": size, "bottom": "32", "shoes": "9"},
           "budget_per_order": budget, "categories": [category], "shopping_style": "decisive",
           "chattiness": "brief", "voice": "short, plain sentences", "interests": []})
}

/// The next step when the model cannot choose: look, add one, check out, buy, leave.
fn plain_next(persona: &Value, said: usize, can_place_order: bool, placed: bool) -> Value {
    let category = persona["categories"][0].as_str().unwrap_or("jackets");
    let color = persona["colors"][0].as_str().unwrap_or("navy");
    let size = persona["sizes"]["top"].as_str().unwrap_or("M");
    let budget = persona["budget_per_order"].as_u64().unwrap_or(100);
    let say =
        |m: String| json!({"action": "say", "message": m, "reason": "plain plan", "goal": ""});
    if placed {
        return json!({"action": "leave", "message": "", "reason": "bought what I came for", "goal": ""});
    }
    if can_place_order {
        return json!({"action": "place_order", "message": "", "reason": "plain plan", "goal": ""});
    }
    match said {
        0 => say(format!(
            "I'm looking for {color} {category} under ${budget}."
        )),
        1 => say(format!("Add the first one in size {size} to my cart.")),
        2 => say("I'd like to check out.".to_owned()),
        _ => json!({"action": "leave", "message": "", "reason": "plain plan ended", "goal": ""}),
    }
}

// ---- one session ---------------------------------------------------------------------------

/// What a session did, as its outcome says it.
#[derive(Default)]
struct Tally {
    chat_turns: u64,
    orders: Vec<Value>,
    spent_cents: i64,
    currency: Option<String>,
    reviews: u64,
    memories: u64,
    cart_adds: u64,
    tools: BTreeSet<String>,
    fallbacks: u64,
}

impl Tally {
    fn outcome(&self) -> Value {
        let spent: Value = serde_json::from_str(&format!(
            "{}.{:02}",
            self.spent_cents / 100,
            self.spent_cents % 100
        ))
        .unwrap_or(Value::Null);
        json!({"chat_turns": self.chat_turns, "orders": self.orders.len(),
               "order_ids": self.orders, "spent": {"value": spent,
               "currency": self.currency.as_deref().unwrap_or("USD")},
               "reviews": self.reviews, "memories": self.memories, "cart_adds": self.cart_adds,
               "tools": self.tools, "fallbacks": self.fallbacks})
    }
}

/// Records a session's steps in commerce, one at a time as they happen.
struct Ledger<'a> {
    gw: &'a Gateway,
    http: &'a reqwest::Client,
    session: String,
}

impl Ledger<'_> {
    async fn note(&self, step: Value, extra: Value) {
        let mut body = json!({"steps": [step]});
        if let (Some(b), Value::Object(e)) = (body.as_object_mut(), extra) {
            b.extend(e);
        }
        let path = format!("/v1/platform/customer-bot/sessions/{}/steps", self.session);
        if let Err(e) = commerce(self.gw, self.http, reqwest::Method::POST, &path, &body).await {
            tracing::warn!(session = %self.session, error = %e, "customer bot: a step was not recorded");
        }
    }

    async fn step(&self, step: Value) {
        self.note(step, json!({})).await;
    }

    async fn finish(&self, state: &str, outcome: Value, error: Option<String>) {
        let path = format!("/v1/platform/customer-bot/sessions/{}/finish", self.session);
        let body = json!({"state": state, "outcome": outcome, "error": error});
        if let Err(e) = commerce(self.gw, self.http, reqwest::Method::POST, &path, &body).await {
            tracing::warn!(session = %self.session, error = %e, "customer bot: the end was not recorded");
        }
    }
}

fn step(kind: &str, ok: bool, title: impl Into<String>, ms: u64) -> Value {
    json!({"kind": kind, "status": if ok { "ok" } else { "failed" }, "title": title.into(), "ms": ms})
}

/// A request step: what was sent and what came back.
fn request_step(
    kind: &str,
    title: impl Into<String>,
    method: &str,
    path: &str,
    body: Option<&Value>,
    r: &Reply,
) -> Value {
    let mut s = step(kind, r.ok(), title, r.ms);
    s["request"] = json!({"method": method, "path": path, "body": body});
    s["response"] = json!({"status": r.status, "body": r.body});
    s
}

/// The gateway's own address, for the sessions' requests.
fn front_door(gw: &Gateway) -> String {
    gw.self_url.to_string()
}

/// One shopping session, start to sign-out. It always ends recorded, done or failed.
async fn shop(gw: &Gateway, http: &reqwest::Client, settings: &Value, s: &Value) {
    let ledger = Ledger {
        gw,
        http,
        session: s["id"].as_str().unwrap_or_default().to_owned(),
    };
    let mut tally = Tally::default();
    let result = visit(gw, http, settings, s, &ledger, &mut tally).await;
    let (state, error) = match result {
        Ok(()) => ("done", None),
        Err(e) => ("failed", Some(e)),
    };
    ledger.finish(state, tally.outcome(), error).await;
}

async fn visit(
    gw: &Gateway,
    http: &reqwest::Client,
    settings: &Value,
    s: &Value,
    ledger: &Ledger<'_>,
    tally: &mut Tally,
) -> Result<(), String> {
    let model = settings["model"].as_str().unwrap_or("glimmer");
    let max_actions = settings["max_actions"].as_u64().unwrap_or(12).clamp(1, 20);
    let reviews_wanted = settings["reviews_per_session"].as_u64().unwrap_or(2).min(5);
    let mut browser = Browser::new(http, &front_door(gw));

    // 1. The account: an existing member, or a new one signed up now.
    let customer = match s["customer_id"].as_str() {
        Some(c) => c.to_owned(),
        None => sign_up(gw, http, model, &ledger.session, ledger, tally).await?,
    };

    // 2. Signing in, as the demo store's sign-in screen does.
    let body = json!({"persona": customer});
    let r = browser.post("/api/auth/demo-login", &body).await;
    ledger
        .step(request_step(
            "sign_in",
            format!("Signed in as {customer}"),
            "POST",
            "/api/auth/demo-login",
            Some(&body),
            &r,
        ))
        .await;
    if !r.ok() {
        return Err(format!("Could not sign in: {}", r.why()));
    }

    // 3. The storefront's first page.
    let (status, title, bytes, ms) = browser.page("/").await;
    let mut v = step(
        "visit",
        (200..400).contains(&status),
        format!(
            "Opened the storefront ({})",
            if title.is_empty() { "/" } else { &title }
        ),
        ms,
    );
    v["request"] = json!({"method": "GET", "path": "/"});
    v["response"] = json!({"status": status, "body": {"title": title, "bytes": bytes}});
    ledger.step(v).await;

    // 4. Who this member is: read their history and make the persona when there is none.
    let (orders, delivered) = history(&mut browser, ledger).await;
    let persona = match s["persona"]["persona"].as_object() {
        Some(p) => Value::Object(p.clone()),
        None => {
            let p = build_persona(
                gw,
                http,
                model,
                &customer,
                s["account"] == "created",
                &mut browser,
                &orders,
                ledger,
                tally,
            )
            .await;
            // A member says once what they wear, as people do when they first shop.
            let msg = format!(
                "Please remember I wear size {} in tops and like {}.",
                p["sizes"]["top"].as_str().unwrap_or("M"),
                p["colors"]
                    .as_array()
                    .map(|c| c
                        .iter()
                        .filter_map(Value::as_str)
                        .take(2)
                        .collect::<Vec<_>>()
                        .join(" and "))
                    .unwrap_or_default()
            );
            chat_step(
                &mut browser,
                None,
                &msg,
                "Told the assistant their sizes and colors",
                ledger,
                tally,
            )
            .await;
            p
        }
    };

    // 5. Reviews of what was delivered and not reviewed yet.
    for item in delivered
        .iter()
        .take(usize::try_from(reviews_wanted).unwrap_or(2) * 2)
    {
        if tally.reviews >= reviews_wanted {
            break;
        }
        review(gw, http, model, &persona, item, &mut browser, ledger, tally).await;
    }

    // 6. Shopping with the assistant: why they came and the first thing they say, then one
    // decision at a time.
    let mut conversation: Option<String> = None;
    let mut transcript: Vec<Value> = Vec::new();
    let started = Instant::now();
    let opening = think(
        gw,
        http,
        model,
        "start",
        json!({"persona": persona, "page": title}),
    )
    .await;
    let (mut goal, first, fallback, usage) = match opening {
        Ok((o, u)) if o["message"].as_str().is_some_and(|m| !m.trim().is_empty()) => (
            o["goal"].as_str().unwrap_or_default().to_owned(),
            Some(o["message"].as_str().unwrap_or_default().trim().to_owned()),
            None,
            u,
        ),
        Ok((_, u)) => (
            String::new(),
            None,
            Some("The answer had no first message.".to_owned()),
            u,
        ),
        Err(e) => (String::new(), None, Some(e), Value::Null),
    };
    if fallback.is_some() {
        tally.fallbacks += 1;
    }
    if goal.trim().is_empty() {
        goal = format!(
            "Find {}",
            persona["categories"][0]
                .as_str()
                .unwrap_or("something to wear")
        );
    }
    let first = first.unwrap_or_else(|| {
        plain_next(&persona, 0, false, false)["message"]
            .as_str()
            .unwrap_or("Show me what's new.")
            .to_owned()
    });
    let mut t = step(
        "think",
        fallback.is_none(),
        format!("Came for: {goal}"),
        millis(started),
    );
    t["detail"] = json!({"action": "say", "message": first, "goal": goal, "usage": usage, "fallback": fallback});
    if fallback.is_some() {
        t["status"] = json!("skipped");
    }
    ledger.note(t, json!({"goal": goal})).await;
    let mut pending: Option<String> = Some(first);
    let mut checkout: Option<Value> = None;
    let mut placed = false;
    let mut said = 0usize;
    let mut failed_in_a_row = 0u32;
    let mut remaining = max_actions;
    let mut bonus = false;
    loop {
        // The checkout card came with the last action: the shopper may still press it.
        let left = match remaining {
            0 if checkout.is_some() && !placed && !bonus => {
                bonus = true;
                1
            }
            0 => break,
            n => n,
        };
        remaining = left - 1;
        // The assistant is not answering: a person gives up after a couple of tries.
        if failed_in_a_row >= 2 {
            ledger
                .step(step(
                    "think",
                    true,
                    "Left: the assistant did not answer twice in a row",
                    0,
                ))
                .await;
            break;
        }
        let can_place_order = checkout.is_some() && !placed;
        if let Some(message) = pending.take() {
            said += 1;
            let turn = chat_step(
                &mut browser,
                conversation.as_deref(),
                &message,
                &format!("Asked: {message}"),
                ledger,
                tally,
            )
            .await;
            failed_in_a_row = u32::from(turn.error.is_some());
            conversation.clone_from(&turn.conversation_id);
            if let Some(c) = turn.block("checkout_card") {
                checkout = Some(summary(c));
            }
            transcript.push(json!({"you": message,
                                   "assistant": turn.reply.chars().take(REPLY_CHARS).collect::<String>(),
                                   "shown": turn.blocks.iter().map(summary).collect::<Vec<_>>(),
                                   "problem": turn.error}));
            continue;
        }
        let input = json!({"persona": persona, "goal": goal, "transcript": transcript,
                           "can_place_order": can_place_order,
                           "checkout": checkout.as_ref().map(|c| json!({"total": c["total"], "failed_checks": c["failed_checks"]})),
                           "actions_left": left});
        let started = Instant::now();
        let decided = think(gw, http, model, "next", input).await;
        let (next, usage, fallback) = match decided {
            Ok((n, u)) if n["action"].is_string() => (n, u, None),
            Ok((_, u)) => (
                plain_next(&persona, said, can_place_order, placed),
                u,
                Some("The answer had no action.".to_owned()),
            ),
            Err(e) => (
                plain_next(&persona, said, can_place_order, placed),
                Value::Null,
                Some(e),
            ),
        };
        let mut next = next;
        // Ready to buy with no checkout card on screen: a person first asks to check out, and
        // the card with Place order comes back.
        // The order is placed: what is left of the visit is leaving, not pressing again.
        if next["action"] == "place_order" && placed {
            next["action"] = json!("leave");
            next["reason"] = json!("The order is placed; the visit is over.");
        }
        if next["action"] == "place_order" && checkout.is_none() && !placed {
            next["action"] = json!("say");
            next["message"] = json!("I'd like to check out now.");
            next["reason"] = json!(format!(
                "Ready to buy, so asked to check out first ({}).",
                next["reason"].as_str().unwrap_or_default()
            ));
        }
        let action = next["action"].as_str().unwrap_or("leave").to_owned();
        let mut t = step(
            "think",
            fallback.is_none(),
            format!("Decided to {}", action.replace('_', " ")),
            millis(started),
        );
        t["detail"] = json!({"action": action, "message": next["message"], "reason": next["reason"],
                             "goal": goal, "usage": usage, "fallback": fallback});
        if fallback.is_some() {
            tally.fallbacks += 1;
            t["status"] = json!("skipped");
            t["title"] = json!(format!(
                "Decided to {} (plain plan: the model did not answer)",
                action.replace('_', " ")
            ));
        }
        ledger.note(t, json!({"goal": goal})).await;
        match action.as_str() {
            "say" => {
                let message = next["message"]
                    .as_str()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if message.is_empty() {
                    break;
                }
                said += 1;
                let turn = chat_step(
                    &mut browser,
                    conversation.as_deref(),
                    &message,
                    &format!("Asked: {message}"),
                    ledger,
                    tally,
                )
                .await;
                if conversation.is_none() {
                    conversation.clone_from(&turn.conversation_id);
                }
                if turn.error.is_some() {
                    failed_in_a_row += 1;
                } else {
                    failed_in_a_row = 0;
                }
                if let Some(c) = turn.block("checkout_card") {
                    checkout = Some(summary(c));
                }
                transcript.push(json!({"you": message,
                                       "assistant": turn.reply.chars().take(REPLY_CHARS).collect::<String>(),
                                       "shown": turn.blocks.iter().map(summary).collect::<Vec<_>>(),
                                       "problem": turn.error}));
                if transcript.len() > 6 {
                    transcript.remove(0);
                }
            }
            "place_order" => {
                let Some(card) = checkout.as_ref().filter(|_| !placed) else {
                    ledger
                        .step(step(
                            "press",
                            false,
                            "Wanted to place an order but no checkout card was shown",
                            0,
                        ))
                        .await;
                    continue;
                };
                let preview = card["preview_id"].as_str().unwrap_or_default().to_owned();
                let body = json!({"preview_id": preview});
                let r = browser.post("/api/v1/sales/orders", &body).await;
                let total = card["total"].as_str().unwrap_or_default().to_owned();
                ledger
                    .step(request_step(
                        "press",
                        if r.ok() {
                            format!("Pressed Place order ({total})")
                        } else {
                            format!("Place order was refused: {}", r.why())
                        },
                        "POST",
                        "/api/v1/sales/orders",
                        Some(&body),
                        &r,
                    ))
                    .await;
                if r.ok() {
                    placed = true;
                    let id = r.body["id"]
                        .as_str()
                        .or_else(|| r.body["order_id"].as_str())
                        .unwrap_or_default();
                    tally.orders.push(json!({"order_id": id, "total": total}));
                    tally.spent_cents += cents(&json!(total)).unwrap_or(0);
                    transcript.push(json!({"you": "(pressed Place order)", "assistant": format!("Order placed: {id}")}));
                } else {
                    transcript.push(json!({"you": "(pressed Place order)", "assistant": format!("Refused: {}", r.why())}));
                }
                checkout = None;
            }
            _ => break,
        }
    }

    // 7. Signing out.
    let r = browser.post("/api/auth/logout", &json!({})).await;
    ledger
        .step(request_step(
            "sign_out",
            "Signed out",
            "POST",
            "/api/auth/logout",
            None,
            &r,
        ))
        .await;
    Ok(())
}

/// A new member: a made-up identity, signed up through commerce (the identity check is the one
/// step a bot cannot do), recorded as the session's account.
async fn sign_up(
    gw: &Gateway,
    http: &reqwest::Client,
    model: &str,
    session: &str,
    ledger: &Ledger<'_>,
    tally: &mut Tally,
) -> Result<String, String> {
    let started = Instant::now();
    let (identity, usage, fallback) =
        match think(gw, http, model, "identity", json!({"hint": session})).await {
            Ok((v, u)) if v["name"].is_string() => (v, u, None),
            Ok(_) => (
                plain_identity(session),
                Value::Null,
                Some("The answer had no name.".to_owned()),
            ),
            Err(e) => (plain_identity(session), Value::Null, Some(e)),
        };
    if fallback.is_some() {
        tally.fallbacks += 1;
    }
    let local: String = identity["email_local"]
        .as_str()
        .unwrap_or("shopper")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.')
        .take(30)
        .collect::<String>()
        .to_lowercase();
    let tag: String = session.chars().rev().take(5).collect();
    let body = json!({
        "name": identity["name"], "email": format!("{}.{tag}@shoppers.example", if local.is_empty() { "shopper" } else { &local }),
        "telephone": identity["telephone"],
        "address": {"street_address": identity["street_address"], "address_locality": identity["address_locality"],
                    "address_region": identity["address_region"], "postal_code": identity["postal_code"]}});
    let made = commerce(
        gw,
        http,
        reqwest::Method::POST,
        "/v1/platform/customer-bot/accounts",
        &body,
    )
    .await;
    let mut s = step(
        "sign_up",
        made.is_ok(),
        "Signed up as a new member",
        millis(started),
    );
    s["request"] =
        json!({"method": "POST", "path": "/v1/platform/customer-bot/accounts", "body": body});
    s["detail"] = json!({"usage": usage, "fallback": fallback});
    match made {
        Ok(v) => {
            let id = v["id"].as_str().unwrap_or_default().to_owned();
            s["title"] = json!(format!("Signed up as a new member {id}"));
            s["response"] = json!({"status": 200, "body": v});
            ledger.note(s, json!({"customer_id": id})).await;
            Ok(id)
        }
        Err(e) => {
            s["response"] = json!({"status": 0, "body": {"error": e}});
            ledger.step(s).await;
            Err(format!("Could not sign up: {e}"))
        }
    }
}

/// The member's orders as they see them, and the delivered items among them.
async fn history(browser: &mut Browser, ledger: &Ledger<'_>) -> (Vec<Value>, Vec<Value>) {
    let path = "/api/v1/sales/orders/mine?limit=20";
    let r = browser.get(path).await;
    let n = r.body["items"].as_array().map_or(0, Vec::len);
    let mut s = request_step(
        "read",
        format!("Read their orders ({n})"),
        "GET",
        path,
        None,
        &r,
    );
    s["response"]["body"] = json!({"total": r.body["total"], "items": n});
    ledger.step(s).await;
    let mut orders = Vec::new();
    let mut delivered = Vec::new();
    for o in r.body["items"].as_array().into_iter().flatten().take(4) {
        let Some(id) = o["id"].as_str() else { continue };
        let path = format!("/api/v1/sales/orders/{id}");
        let d = browser.get(&path).await;
        if !d.ok() {
            continue;
        }
        let items: Vec<Value> = d.body["ordered_item"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|i| {
                json!({"order_id": id, "product_id": i["product_id"], "name": i["name"],
                            "size": i["size"], "color": i["color"], "price": i["price"],
                            "status": i["order_item_status"]})
            })
            .collect();
        delivered.extend(items.iter().filter(|i| i["status"] == "delivered").cloned());
        orders.push(
            json!({"order_number": d.body["order_number"], "status": d.body["order_status"],
                           "total": d.body["totals"]["total"], "items": items}),
        );
    }
    (orders, delivered)
}

#[allow(clippy::too_many_arguments)]
async fn build_persona(
    gw: &Gateway,
    http: &reqwest::Client,
    model: &str,
    customer: &str,
    new_account: bool,
    browser: &mut Browser,
    orders: &[Value],
    ledger: &Ledger<'_>,
    tally: &mut Tally,
) -> Value {
    let path = "/api/v1/customer/people/me";
    let r = browser.get(path).await;
    let profile = json!({"sizes": r.body["sizes"], "tier": r.body["member_of"],
                         "city": r.body["addresses"][0]["address_locality"],
                         "registered": r.body["registered_at"]});
    let mut s = request_step("read", "Read their profile", "GET", path, None, &r);
    s["response"]["body"] = profile.clone();
    ledger.step(s).await;
    let started = Instant::now();
    let input = json!({"profile": profile, "orders": orders, "new_account": new_account});
    let (persona, usage, fallback) = match think(gw, http, model, "persona", input).await {
        Ok((p, u)) if p["description"].is_string() => (p, u, None),
        Ok(_) => (
            plain_persona(customer),
            Value::Null,
            Some("The answer had no description.".to_owned()),
        ),
        Err(e) => (plain_persona(customer), Value::Null, Some(e)),
    };
    if fallback.is_some() {
        tally.fallbacks += 1;
    }
    let description = persona["description"]
        .as_str()
        .unwrap_or("Shopper")
        .to_owned();
    let body =
        json!({"description": description, "persona": persona, "session_id": ledger.session});
    let saved = commerce(
        gw,
        http,
        reqwest::Method::PUT,
        &format!("/v1/platform/customer-bot/personas/{customer}"),
        &body,
    )
    .await;
    let mut p = step(
        "persona",
        saved.is_ok(),
        format!("Made the persona: {description}"),
        millis(started),
    );
    p["detail"] =
        json!({"persona": persona, "usage": usage, "fallback": fallback, "error": saved.err()});
    ledger.step(p).await;
    persona
}

/// One chat turn, recorded with what it showed.
async fn chat_step(
    browser: &mut Browser,
    conversation: Option<&str>,
    message: &str,
    title: &str,
    ledger: &Ledger<'_>,
    tally: &mut Tally,
) -> Turn {
    let turn = browser.chat(conversation, message).await;
    tally.chat_turns += 1;
    tally.memories += turn.memories.len() as u64;
    if turn.tools.iter().any(|t| t == "add_to_cart") {
        tally.cart_adds += 1;
    }
    tally.tools.extend(turn.tools.iter().cloned());
    if let Some(total) = turn
        .block("checkout_card")
        .and_then(|c| c["data"]["total"]["currency"].as_str())
    {
        tally.currency = Some(total.to_owned());
    }
    let mut s = step(
        "chat",
        turn.error.is_none(),
        title.chars().take(280).collect::<String>(),
        turn.ms,
    );
    s["request"] = json!({"method": "POST", "path": "/api/chat/fo/turns", "body": {"message": message, "conversation_id": conversation}});
    s["response"] = json!({"status": turn.status});
    s["detail"] = json!({"reply": turn.reply, "blocks": turn.blocks.iter().map(summary).collect::<Vec<_>>(),
                         "tools": turn.tools, "memories": turn.memories, "usage": turn.usage,
                         "conversation_id": turn.conversation_id, "error": turn.error});
    let extra = turn
        .conversation_id
        .as_ref()
        .map_or_else(|| json!({}), |c| json!({"conversation_id": c}));
    ledger.note(s, extra).await;
    turn
}

/// A review of one delivered item, through the preview a review form makes first.
#[allow(clippy::too_many_arguments)]
async fn review(
    gw: &Gateway,
    http: &reqwest::Client,
    model: &str,
    persona: &Value,
    item: &Value,
    browser: &mut Browser,
    ledger: &Ledger<'_>,
    tally: &mut Tally,
) {
    let name = item["name"].as_str().unwrap_or("an item").to_owned();
    let mut base = Map::new();
    base.insert("product_id".into(), item["product_id"].clone());
    base.insert("order_id".into(), item["order_id"].clone());
    // Already reviewed: the preview's check says so, and nothing is written.
    let probe = json!({"product_id": item["product_id"], "order_id": item["order_id"],
                       "review_rating": 5, "review_body": "Checking whether this item can be reviewed."});
    let p = browser
        .post("/api/v1/catalog/reviews?preview=true", &probe)
        .await;
    let open = p.ok()
        && p.body["checks"]
            .as_array()
            .is_none_or(|c| c.iter().all(|x| x["passed"] != false));
    if !open {
        return;
    }
    let started = Instant::now();
    let (written, usage, fallback) = match think(
        gw,
        http,
        model,
        "review",
        json!({"persona": persona, "item": item}),
    )
    .await
    {
        Ok((v, u)) if v["body"].is_string() => (v, u, None),
        Ok(_) => (
            json!({"rating": 4, "body": format!("The {name} is good. It fits as expected."), "fit": "true_to_size"}),
            Value::Null,
            Some("The answer had no review.".to_owned()),
        ),
        Err(e) => (
            json!({"rating": 4, "body": format!("The {name} is good. It fits as expected."), "fit": "true_to_size"}),
            Value::Null,
            Some(e),
        ),
    };
    if fallback.is_some() {
        tally.fallbacks += 1;
    }
    let mut body = Value::Object(base);
    body["review_rating"] = written["rating"].clone();
    body["review_body"] = written["body"].clone();
    if let Some(fit) = written["fit"].as_str().filter(|f| *f != "none") {
        body["fit"] = json!(fit);
    }
    let path = "/api/v1/catalog/reviews";
    let preview = browser.post(&format!("{path}?preview=true"), &body).await;
    let result = match preview.body["preview_id"].as_str() {
        Some(id) if preview.ok() => browser.post(path, &json!({"preview_id": id})).await,
        _ => preview,
    };
    if result.ok() {
        tally.reviews += 1;
    }
    let stars = written["rating"].as_u64().unwrap_or(0);
    let mut s = request_step(
        "review",
        if result.ok() {
            format!("Reviewed {name}: {stars} of 5")
        } else {
            format!("Could not review {name}: {}", result.why())
        },
        "POST",
        path,
        Some(&body),
        &result,
    );
    s["ms"] = json!(millis(started));
    s["detail"] = json!({"usage": usage, "fallback": fallback});
    ledger.step(s).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_reads_as_cents_however_it_is_written() {
        assert_eq!(
            cents(&json!({"value": 71.2, "currency": "USD"})),
            Some(7120)
        );
        assert_eq!(cents(&json!("$1,204.05")), Some(120_405));
        assert_eq!(cents(&json!(89)), Some(8900));
        assert_eq!(money_text(7120), "$71.20");
    }

    #[test]
    fn a_turn_keeps_the_reply_the_last_blocks_and_the_tools_that_ran() {
        let stream = "event: turn.started\ndata: {\"conversation_id\":\"conv_1\"}\n\n\
event: agent.step\ndata: {\"tool\":\"search_products\",\"status\":\"done\"}\n\n\
event: ui.block\ndata: {\"block_id\":\"b1\",\"ui\":\"product_list\",\"data\":{\"items\":[{\"product_id\":\"prd_a\",\"name\":\"A\",\"price\":{\"value\":89.0}}]}}\n\n\
event: ui.block\ndata: {\"block_id\":\"b2\",\"ui\":\"checkout_card\",\"data\":{\"preview_id\":\"pv_1\",\"total\":{\"value\":\"95.40\",\"currency\":\"USD\"},\"checks\":[]}}\n\n\
event: message.delta\ndata: {\"text\":\"Here\"}\n\n\
event: message.done\ndata: {\"text\":\"Here you go.\"}\n\n";
        let mut t = Turn::default();
        t.read(stream);
        assert_eq!(t.conversation_id.as_deref(), Some("conv_1"));
        assert_eq!(t.reply, "Here you go.");
        assert_eq!(t.tools, vec!["search_products"]);
        let card = summary(t.block("checkout_card").unwrap());
        assert_eq!(card["preview_id"], "pv_1");
        assert_eq!(card["total"], "$95.40");
        assert_eq!(summary(&t.blocks[0])["products"][0]["price"], "$89.00");
    }

    #[test]
    fn the_plain_plan_looks_adds_checks_out_buys_and_leaves() {
        let p = plain_persona("cus_x");
        assert_eq!(plain_next(&p, 0, false, false)["action"], "say");
        assert_eq!(plain_next(&p, 2, true, false)["action"], "place_order");
        assert_eq!(plain_next(&p, 3, false, true)["action"], "leave");
        assert!(
            plain_identity("cbs_1")["name"]
                .as_str()
                .unwrap()
                .contains(' ')
        );
    }
}
