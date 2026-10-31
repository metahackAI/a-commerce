//! `cargo xtask sim`: plays a demo store's shoppers forward in store time through the same REST
//! API a browser uses, so the store keeps moving after the scene clock: orders take stock, return
//! requests come in, and the watchers that read them make tasks (docs/design.md principle 4).
//!
//! What shoppers do is measured from the store's own history when the run starts (orders per hour
//! of day, who orders, lines per basket, variants and quantities); what history cannot give is in
//! `seed/demo/flow.yml` with its source. Every store minute draws from its own ChaCha8 stream
//! seeded with (seed, minute), so a run from the same store state draws the same events.
//!
//! A stopped demo clock is moved to each event (`PUT /v1/platform/demo-clock`, which runs the jobs
//! due on the way) and left at `--until`; a running clock is followed in real time. Shoppers are
//! generated members only: the scene cast (`never_touch` in the seed spec) is never signed in.
//!
//! ```text
//! cargo xtask sim --base http://127.0.0.1:18280 (--until 2026-11-06T18:00 | --hours 6) \
//!     [--seed 1] [--config seed/demo/flow.yml] [--dry-run]
//! ```
//!
//! The operator session signs in as `admin` with `SIM_ADMIN_PASSWORD` (default: the demo seed's).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDateTime, TimeDelta, Timelike};
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::Deserialize;
use serde_json::{Value, json};

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) acom-sim";
const MINUTE: &str = "%Y-%m-%dT%H:%M";

/// `seed/demo/flow.yml`.
#[derive(Debug, Deserialize)]
struct Flow {
    members: Members,
    orders: Orders,
    returns: Returns,
}

#[derive(Debug, Deserialize)]
struct Members {
    /// A seed spec next to the flow file; its `never_touch.customers` is the scene cast.
    never_touch_from: String,
}

#[derive(Debug, Deserialize)]
struct Orders {
    lookback_days: u32,
    basket_sample: usize,
}

#[derive(Debug, Deserialize)]
struct Returns {
    rate: f64,
    reasons: BTreeMap<String, f64>,
    kind: String,
    send_back: String,
}

/// What the store's history says about its shoppers.
#[derive(Debug, Default)]
struct History {
    /// Orders per hour of day (store time), averaged over the lookback days.
    per_hour: [f64; 24],
    /// The customer of every lookback order: drawing from it weights by how often each orders.
    customers: Vec<String>,
    /// Lines per sampled order.
    basket_sizes: Vec<usize>,
    /// (variant, quantity) of every sampled line.
    lines: Vec<(String, u32)>,
    /// Delivered orders that may still be returned, drawn from without putting back.
    returnable: Vec<String>,
    return_window_days: i64,
}

/// A refused or failed call: the store's error code (or the HTTP status) and its message.
#[derive(Debug)]
struct Refused {
    code: String,
    message: String,
}

impl Refused {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

/// One browser session: its own cookies, so the operator and each shopper stay apart.
struct Session {
    http: reqwest::Client,
    base: String,
    cookies: BTreeMap<String, String>,
}

impl Session {
    fn new(http: &reqwest::Client, base: &str) -> Self {
        Self {
            http: http.clone(),
            base: base.trim_end_matches('/').to_owned(),
            cookies: BTreeMap::new(),
        }
    }

    async fn call(
        &mut self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, Refused> {
        let cookie = self
            .cookies
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        let mut req = self
            .http
            .request(method.clone(), format!("{}{path}", self.base))
            .header("accept", "application/json")
            .header("cookie", cookie);
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
        let r = req
            .send()
            .await
            .map_err(|e| Refused::new("network", format!("{path}: {e}")))?;
        for h in r.headers().get_all(reqwest::header::SET_COOKIE) {
            if let Ok(s) = h.to_str()
                && let Some((k, v)) = s.split(';').next().and_then(|p| p.split_once('='))
            {
                self.cookies
                    .insert(k.trim().to_owned(), v.trim().to_owned());
            }
        }
        let status = r.status();
        let text = r
            .text()
            .await
            .map_err(|e| Refused::new("network", e.to_string()))?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(v);
        }
        let code = v["error"]["code"]
            .as_str()
            .map_or_else(|| status.as_u16().to_string(), str::to_owned);
        let message = v["error"]["message"]
            .as_str()
            .map_or_else(|| text.chars().take(200).collect(), str::to_owned);
        Err(Refused::new(&code, format!("{path}: {message}")))
    }

    async fn get(&mut self, path: &str) -> Result<Value, Refused> {
        self.call(reqwest::Method::GET, path, None).await
    }

    async fn post(&mut self, path: &str, body: Value) -> Result<Value, Refused> {
        self.call(reqwest::Method::POST, path, Some(body)).await
    }

    /// A write through the preview protocol: `?preview=true`, then the `preview_id`.
    async fn write(&mut self, path: &str, body: Value) -> Result<Value, Refused> {
        let preview = self.post(&format!("{path}?preview=true"), body).await?;
        let id = preview["preview_id"]
            .as_str()
            .ok_or_else(|| Refused::new("no_preview", format!("{path}: no preview_id")))?
            .to_owned();
        self.post(path, json!({"preview_id": id})).await
    }
}

/// One store minute's draws. Only `next_u64` is used and the picks are integer arithmetic, like
/// the seed generator's dice, so draws do not depend on `rand`'s samplers.
struct Dice(ChaCha8Rng);

impl Dice {
    fn new(seed: u64, minute: i64) -> Self {
        let mix = seed.rotate_left(32) ^ minute as u64;
        Self(ChaCha8Rng::seed_from_u64(mix))
    }

    fn below(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        let n = n as u64;
        let limit = u64::MAX - u64::MAX % n;
        loop {
            let x = self.0.next_u64();
            if x < limit {
                return usize::try_from(x % n).unwrap_or(0);
            }
        }
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.0.next_u64() >> 11) as f64 / (1_u64 << 53) as f64
    }

    /// A count with the given mean (Knuth; the means here are well under one).
    fn poisson(&mut self, mean: f64) -> usize {
        let limit = (-mean).exp();
        let (mut k, mut p) = (0, self.unit());
        while p > limit {
            k += 1;
            p *= self.unit();
        }
        k
    }

    fn weighted<'a>(&mut self, weights: &'a BTreeMap<String, f64>) -> &'a str {
        let total: f64 = weights.values().sum();
        let mut x = self.unit() * total;
        for (k, w) in weights {
            if x < *w {
                return k;
            }
            x -= w;
        }
        weights.keys().next_back().map_or("", String::as_str)
    }
}

/// What a run did, for the summary.
#[derive(Debug, Default)]
struct Tally {
    orders: usize,
    revenue: f64,
    lines_lost: usize,
    orders_lost: usize,
    returns: usize,
    refused: BTreeMap<String, usize>,
    jobs: BTreeMap<String, usize>,
    tasks: BTreeMap<String, String>,
}

impl Tally {
    fn refuse(&mut self, what: &str, r: &Refused) {
        *self
            .refused
            .entry(format!("{what}: {}", r.code))
            .or_default() += 1;
        eprintln!("  {what} refused: {} {}", r.code, r.message);
    }
}

pub fn sim(root: &Path) -> ExitCode {
    let Some(base) = arg("--base") else {
        eprintln!(
            "usage: cargo xtask sim --base <gateway url> (--until <store time> | --hours N) [--seed N] [--config file] [--dry-run]"
        );
        return ExitCode::FAILURE;
    };
    let config = arg("--config").map_or_else(|| root.join("seed/demo/flow.yml"), Into::into);
    let flow = match load(&config) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let seed: u64 = arg("--seed").and_then(|s| s.parse().ok()).unwrap_or(1);
    let dry_run = std::env::args().any(|a| a == "--dry-run");
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(run(&base, &flow, seed, dry_run)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// The flow file, with the scene cast read from the seed spec it names.
fn load(path: &Path) -> Result<(Flow, BTreeSet<String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let flow: Flow =
        serde_norway::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let spec = path
        .parent()
        .unwrap_or(Path::new("."))
        .join(&flow.members.never_touch_from);
    let text = std::fs::read_to_string(&spec).map_err(|e| format!("{}: {e}", spec.display()))?;
    let v: Value = serde_norway::from_str(&text).map_err(|e| format!("{}: {e}", spec.display()))?;
    let cast = v["never_touch"]["customers"]
        .as_array()
        .ok_or_else(|| format!("{}: no never_touch.customers", spec.display()))?
        .iter()
        .filter_map(|c| c.as_str().map(str::to_owned))
        .collect();
    Ok((flow, cast))
}

async fn run(
    base: &str,
    (flow, cast): &(Flow, BTreeSet<String>),
    seed: u64,
    dry_run: bool,
) -> Result<(), String> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())?;
    let mut op = Session::new(&http, base);
    let password = std::env::var("SIM_ADMIN_PASSWORD").unwrap_or_else(|_| "admin123".into());
    op.post(
        "/api/auth/login",
        json!({"login_id": "admin", "password": password}),
    )
    .await
    .map_err(|e| format!("operator sign-in: {} {}", e.code, e.message))?;
    let clock = op
        .get("/api/v1/platform/demo-clock")
        .await
        .map_err(|e| format!("demo clock: {}", e.message))?;
    if clock["demo_mode"].as_bool() != Some(true) {
        return Err("this store runs on real time; the simulator only plays demo stores".into());
    }
    let now = clock["now"]
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.naive_local())
        .ok_or("demo clock: no now")?;
    let running = clock["is_running"].as_bool().unwrap_or(false);
    let until = match (arg("--until"), arg("--hours")) {
        (Some(u), _) => NaiveDateTime::parse_from_str(&u, MINUTE)
            .map_err(|e| format!("--until {u}: {e} (store time, {MINUTE})"))?,
        (None, Some(h)) => {
            let h: i64 = h
                .parse()
                .map_err(|_| format!("--hours {h}: not a number"))?;
            now + TimeDelta::hours(h)
        }
        (None, None) => return Err("give --until <store time> or --hours N".into()),
    };
    if until <= now {
        return Err(format!(
            "--until {until} is not after the store clock {now}"
        ));
    }

    let started = Instant::now();
    let mut history = read_history(&mut op, flow, cast, now, seed).await?;
    eprintln!(
        "store clock {} ({}), until {}; history: {:.0} orders/day, {} customers, {} sampled lines, {} returnable orders ({:.1}s)",
        now.format(MINUTE),
        if running { "running" } else { "stopped" },
        until.format(MINUTE),
        history.per_hour.iter().sum::<f64>(),
        history.customers.len(),
        history.lines.len(),
        history.returnable.len(),
        started.elapsed().as_secs_f64(),
    );
    if history.customers.is_empty() || history.lines.is_empty() {
        return Err("the store has no order history to play from".into());
    }

    let mut tally = Tally::default();
    let mut planned = (0, 0);
    let mut t = now + TimeDelta::minutes(1);
    while t <= until {
        let mut dice = Dice::new(seed, t.and_utc().timestamp() / 60);
        let mean = history.per_hour[t.hour() as usize] / 60.0;
        let orders = dice.poisson(mean);
        let returns = dice.poisson(mean * flow.returns.rate);
        planned = (planned.0 + orders, planned.1 + returns);
        if !dry_run && orders + returns > 0 {
            reach(&mut op, t, running, now, started, &mut tally).await?;
            for _ in 0..orders {
                let customer = history.customers[dice.below(history.customers.len())].clone();
                let n = history.basket_sizes[dice.below(history.basket_sizes.len())].max(1);
                let lines: Vec<(String, u32)> = (0..n)
                    .map(|_| history.lines[dice.below(history.lines.len())].clone())
                    .collect();
                place_order(&http, base, &customer, &lines, &mut tally).await;
            }
            for _ in 0..returns {
                let reason = dice.weighted(&flow.returns.reasons).to_owned();
                send_return(
                    &http,
                    base,
                    &mut op,
                    &mut history,
                    &mut dice,
                    t,
                    &reason,
                    flow,
                    cast,
                    &mut tally,
                )
                .await;
            }
        }
        t += TimeDelta::minutes(1);
    }
    if !dry_run {
        reach(&mut op, until, running, now, started, &mut tally).await?;
    }

    println!(
        "{} store {} -> {} (seed {seed}): {} orders and {} return requests drawn",
        if dry_run { "dry run," } else { "played" },
        now.format(MINUTE),
        until.format(MINUTE),
        planned.0,
        planned.1,
    );
    if dry_run {
        return Ok(());
    }
    println!(
        "orders placed {} (${:.2}), lines lost to stock {}, orders with nothing in stock {}",
        tally.orders, tally.revenue, tally.lines_lost, tally.orders_lost
    );
    println!("return requests sent {}", tally.returns);
    for (what, n) in &tally.refused {
        println!("refused {what}: {n}");
    }
    for (kind, n) in &tally.jobs {
        println!("job {kind}: ran {n}");
    }
    for (task, state) in &tally.tasks {
        println!("task {task}: {state}");
    }
    println!("took {:.0}s", started.elapsed().as_secs_f64());
    Ok(())
}

/// The lookback days' orders, a sample of them in full, and the delivered orders that may still
/// be returned.
async fn read_history(
    op: &mut Session,
    flow: &Flow,
    cast: &BTreeSet<String>,
    now: NaiveDateTime,
    seed: u64,
) -> Result<History, String> {
    let today = now.date();
    let days = i64::from(flow.orders.lookback_days.max(1));
    let from = today - TimeDelta::days(days);
    let to = today - TimeDelta::days(1);
    let orders = search_orders(op, &format!("order_date_from={from}&order_date_to={to}")).await?;
    let mut h = History::default();
    for o in &orders {
        if let Some(hour) = o["order_date"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.naive_local().hour() as usize)
        {
            h.per_hour[hour] += 1.0 / days as f64;
        }
        if let Some(c) = o["customer_id"].as_str()
            && !cast.contains(c)
        {
            h.customers.push(c.to_owned());
        }
    }
    let mut dice = Dice::new(seed, 0);
    let mut ids: Vec<&str> = orders.iter().filter_map(|o| o["id"].as_str()).collect();
    for i in (1..ids.len()).rev() {
        ids.swap(i, dice.below(i + 1));
    }
    for id in ids.iter().take(flow.orders.basket_sample) {
        let o = op
            .get(&format!("/api/v1/sales/orders/{id}"))
            .await
            .map_err(|e| format!("get order {id}: {}", e.message))?;
        let items = o["ordered_item"].as_array().cloned().unwrap_or_default();
        h.basket_sizes.push(items.len());
        for i in &items {
            if let (Some(v), Some(q)) = (i["variant_id"].as_str(), i["order_quantity"].as_u64()) {
                h.lines.push((v.to_owned(), u32::try_from(q).unwrap_or(1)));
            }
        }
    }

    let window = op
        .get("/api/v1/platform/policies?key=return_window_days")
        .await
        .map_err(|e| format!("return window policy: {}", e.message))?;
    h.return_window_days = window["items"][0]["value"]
        .as_i64()
        .ok_or("return window policy: no value")?;
    // Delivery takes days, so orders placed a little before the window may be delivered inside it;
    // send_return checks the delivery date.
    let from = today - TimeDelta::days(h.return_window_days + 7);
    h.returnable = search_orders(
        op,
        &format!("order_status=delivered&order_date_from={from}&order_date_to={today}"),
    )
    .await?
    .iter()
    .filter(|o| o["customer_id"].as_str().is_some_and(|c| !cast.contains(c)))
    .filter_map(|o| o["id"].as_str().map(str::to_owned))
    .collect();
    Ok(h)
}

async fn search_orders(op: &mut Session, query: &str) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut path = format!("/api/v1/sales/orders?{query}&limit=100");
        if let Some(c) = &cursor {
            path.push_str(&format!("&cursor={c}"));
        }
        let page = op
            .get(&path)
            .await
            .map_err(|e| format!("search orders: {}", e.message))?;
        out.extend(page["items"].as_array().cloned().unwrap_or_default());
        match page["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_owned()),
            None => return Ok(out),
        }
    }
}

/// Brings the store clock to `t`: a stopped clock is moved there (running the jobs due on the
/// way), a running one is waited for.
async fn reach(
    op: &mut Session,
    t: NaiveDateTime,
    running: bool,
    start: NaiveDateTime,
    started: Instant,
    tally: &mut Tally,
) -> Result<(), String> {
    if running {
        let ahead = (t - start).to_std().unwrap_or_default();
        if let Some(wait) = ahead.checked_sub(started.elapsed()) {
            tokio::time::sleep(wait).await;
        }
        return Ok(());
    }
    let moved = op
        .call(
            reqwest::Method::PUT,
            "/api/v1/platform/demo-clock",
            Some(json!({"now": t.format(MINUTE).to_string(), "is_running": false})),
        )
        .await
        .map_err(|e| format!("move clock to {t}: {} {}", e.code, e.message))?;
    for job in moved["ran"].as_array().into_iter().flatten() {
        let kind = job["kind"].as_str().unwrap_or("?");
        *tally.jobs.entry(kind.to_owned()).or_default() += 1;
        for task in job["tasks"].as_array().into_iter().flatten() {
            let id = task["task_id"].as_str().unwrap_or("?").to_owned();
            if task["created"].as_bool() == Some(true) {
                let at = job["run_at"].as_str().unwrap_or("?");
                tally.tasks.insert(id, format!("created by {kind} at {at}"));
            } else {
                tally
                    .tasks
                    .entry(id)
                    .or_insert_with(|| format!("already open, updated by {kind}"));
            }
        }
    }
    Ok(())
}

/// A shopper signs in, puts the lines in the cart (a sold-out line is a lost sale), and checks
/// out to their saved address.
async fn place_order(
    http: &reqwest::Client,
    base: &str,
    customer: &str,
    lines: &[(String, u32)],
    tally: &mut Tally,
) {
    let mut s = Session::new(http, base);
    if let Err(e) = s
        .post("/api/auth/demo-login", json!({"persona": customer}))
        .await
    {
        tally.refuse("sign_in", &e);
        return;
    }
    let mut added = 0;
    for (variant, quantity) in lines {
        let body = json!({"mode": "variant", "variant_id": variant, "order_quantity": quantity});
        match s.write("/api/v1/sales/carts/mine/lines", body).await {
            Ok(_) => added += 1,
            Err(e) if e.code == "out_of_stock" => tally.lines_lost += 1,
            Err(e) => tally.refuse("add_to_cart", &e),
        }
    }
    if added == 0 {
        tally.orders_lost += 1;
    } else if let Err(e) = checkout(&mut s, customer, tally).await {
        tally.refuse("place_order", &e);
    }
    let _ = s.post("/api/auth/logout", json!({})).await;
}

async fn checkout(s: &mut Session, customer: &str, tally: &mut Tally) -> Result<(), Refused> {
    let cart = s.get("/api/v1/sales/carts/mine").await?;
    let addresses = s
        .get(&format!("/api/v1/customer/people/{customer}/addresses"))
        .await?;
    let address = addresses["items"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|x| x["is_primary"].as_bool() == Some(true))
                .or_else(|| a.first())
        })
        .and_then(|a| a["id"].as_str())
        .ok_or_else(|| Refused::new("no_address", "no saved address"))?
        .to_owned();
    let prepared = s
        .post(
            "/api/v1/sales/orders/prepare?preview=true",
            json!({"delivery_mode": "saved", "delivery_address_id": address,
                   "client_total": cart["total"]}),
        )
        .await?;
    // The checkout card shows the failed checks; the shopper does not place the order.
    if let Some(failed) = prepared["checks"]
        .as_array()
        .and_then(|c| c.iter().find(|x| x["passed"].as_bool() == Some(false)))
    {
        return Err(Refused::new(
            failed["rule"].as_str().unwrap_or("check_failed"),
            failed["detail"].as_str().unwrap_or_default(),
        ));
    }
    let id = prepared["preview_id"]
        .as_str()
        .ok_or_else(|| Refused::new("no_preview", "prepare_order: no preview_id"))?;
    s.post("/api/v1/sales/orders", json!({"preview_id": id}))
        .await?;
    tally.orders += 1;
    tally.revenue += cart["total"]["value"].as_f64().unwrap_or(0.0);
    Ok(())
}

/// A shopper with a delivered order inside the return window sends back one line for the drawn
/// reason, confirming the refund and the charge the store offered.
#[allow(clippy::too_many_arguments)]
async fn send_return(
    http: &reqwest::Client,
    base: &str,
    op: &mut Session,
    history: &mut History,
    dice: &mut Dice,
    t: NaiveDateTime,
    reason: &str,
    flow: &Flow,
    cast: &BTreeSet<String>,
    tally: &mut Tally,
) {
    // Draw orders until one can still be returned; each is drawn once.
    let order = loop {
        if history.returnable.is_empty() {
            *tally
                .refused
                .entry("send_return: none returnable".into())
                .or_default() += 1;
            return;
        }
        let id = history
            .returnable
            .swap_remove(dice.below(history.returnable.len()));
        let Ok(o) = op.get(&format!("/api/v1/sales/orders/{id}")).await else {
            continue;
        };
        let delivered = o["delivery"]["delivered_at"]
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.naive_local());
        let inside = delivered
            .is_some_and(|d| d <= t && t - d <= TimeDelta::days(history.return_window_days));
        let claimed = o["claims"].as_array().is_some_and(|c| !c.is_empty());
        let mine = o["customer_id"].as_str().is_some_and(|c| !cast.contains(c));
        if inside && !claimed && mine {
            break o;
        }
    };
    let (Some(order_id), Some(customer)) = (order["id"].as_str(), order["customer_id"].as_str())
    else {
        return;
    };
    let items = order["ordered_item"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        return;
    }
    let item = &items[dice.below(items.len())];
    let (Some(item_id), Some(quantity)) = (item["id"].as_str(), item["order_quantity"].as_u64())
    else {
        return;
    };

    let mut s = Session::new(http, base);
    if let Err(e) = s
        .post("/api/auth/demo-login", json!({"persona": customer}))
        .await
    {
        tally.refuse("sign_in", &e);
        return;
    }
    let sent = async {
        let options = s
            .get(&format!(
                "/api/v1/fulfillment/orders/{order_id}/items/{item_id}/return-options?reason={reason}&quantity={quantity}"
            ))
            .await?;
        let chosen = options["items"]
            .as_array()
            .and_then(|o| o.iter().find(|x| x["kind"] == flow.returns.kind.as_str()))
            .ok_or_else(|| Refused::new("not_offered", format!("no {} offered", flow.returns.kind)))?;
        let offered = chosen["send_back"]["methods"]
            .as_array()
            .is_some_and(|m| m.iter().any(|x| x == flow.returns.send_back.as_str()));
        if !offered {
            return Err(Refused::new(
                "not_offered",
                format!("{} not offered", flow.returns.send_back),
            ));
        }
        s.post(
            "/api/v1/fulfillment/returns",
            json!({
                "order_id": order_id,
                "items": [{"order_item_id": item_id, "quantity": quantity}],
                "reason": reason,
                "kind": flow.returns.kind,
                "send_back_method": flow.returns.send_back,
                "expected_refund": chosen["refund_amount"],
                "expected_charge": chosen["you_pay"],
            }),
        )
        .await
    }
    .await;
    match sent {
        Ok(_) => tally.returns += 1,
        Err(e) => tally.refuse("send_return", &e),
    }
    let _ = s.post("/api/auth/logout", json!({})).await;
}
