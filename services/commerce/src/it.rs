//! Database tests for scheduled work and the demo clock, against the demo store at
//! `DATABASE_URL` (migrated and seeded, demo clock Fri Nov 6 2026 11:00). Run on request:
//! `cargo test -p acom-commerce -- --ignored --test-threads=1`.
//!
//! Each test puts back what it changed (tasks, jobs, the demo clock) through
//! `MIGRATION_DATABASE_URL`, even when it fails, so none depends on another or on their order.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use acom_auth::{Principal, Signer, Surface, Verifier, scopes};
use acom_core::TenantId;
use acom_core::enums::ActorKind;
use acom_store::{Db, Runtime};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures::FutureExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const STORE: &str = "store_northwind";

fn tenant() -> TenantId {
    TenantId::parse(STORE).unwrap()
}

async fn runtime(demo_mode: bool) -> Arc<Runtime> {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let db = Db::connect(&url, 4).await.expect("connect");
    let policy = super::load_policies(concat!(env!("CARGO_MANIFEST_DIR"), "/../../policies"))
        .expect("policies");
    Arc::new(Runtime::new(db, policy, demo_mode))
}

async fn owner() -> PgPool {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    PgPool::connect(&url).await.expect("connect as owner")
}

/// The rows these tests may change, as they were.
struct Snapshot {
    tasks: Value,
    jobs: Value,
    clock: Value,
    /// Change sets a watcher's fix may add (with their previews, items, events and log).
    change_sets: Value,
    /// List prices a fix may raise.
    offers: Value,
    /// The store's policy overrides (a test may switch `run_fix`).
    overrides: Value,
}

async fn scoped(pool: &PgPool) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(STORE)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}

async fn snapshot(pool: &PgPool) -> Snapshot {
    let mut tx = scoped(pool).await;
    let all = |table: &'static str| {
        sqlx::query_scalar::<_, Value>(sqlx::AssertSqlSafe(format!(
            "SELECT coalesce(jsonb_agg(to_jsonb(t)), '[]') FROM {table} t WHERE tenant_id = $1"
        )))
        .bind(STORE)
    };
    let tasks = all("platform.task").fetch_one(&mut *tx).await.unwrap();
    let jobs = all("platform.job").fetch_one(&mut *tx).await.unwrap();
    let clock = all("platform.demo_clock")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let change_sets = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(id), '[]') FROM platform.change_set WHERE tenant_id = $1",
    )
    .bind(STORE)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let offers = all("catalog.offer").fetch_one(&mut *tx).await.unwrap();
    let overrides = all("platform.policy_override")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Snapshot {
        tasks,
        jobs,
        clock,
        change_sets,
        offers,
        overrides,
    }
}

async fn restore(pool: &PgPool, s: &Snapshot) {
    let mut tx = scoped(pool).await;
    let statements = [
        (
            "UPDATE catalog.offer o SET price = r.price, availability = r.availability,
               inventory_level = r.inventory_level, updated_at = r.updated_at, version = r.version
             FROM jsonb_populate_recordset(NULL::catalog.offer, $2) r
             WHERE o.tenant_id = $1 AND o.id = r.id
               AND (o.price, o.availability, o.inventory_level, o.version)
                   IS DISTINCT FROM (r.price, r.availability, r.inventory_level, r.version)",
            &s.offers,
        ),
        (
            "UPDATE platform.policy_override p SET partial = r.partial,
               policy_version = r.policy_version, updated_at = r.updated_at, version = r.version
             FROM jsonb_populate_recordset(NULL::platform.policy_override, $2) r
             WHERE p.tenant_id = $1 AND p.file = r.file",
            &s.overrides,
        ),
        (
            "DELETE FROM platform.task WHERE tenant_id = $1 AND id NOT IN
               (SELECT r.id FROM jsonb_populate_recordset(NULL::platform.task, $2) r)",
            &s.tasks,
        ),
        (
            "UPDATE platform.task t SET title = r.title, task_state = r.task_state,
               summary = r.summary, request_text = r.request_text,
               watcher_observation = r.watcher_observation, related_entities = r.related_entities,
               change_set_id = r.change_set_id, conversation_id = r.conversation_id,
               start_time = r.start_time, end_time = r.end_time, updated_at = r.updated_at,
               version = r.version
             FROM jsonb_populate_recordset(NULL::platform.task, $2) r
             WHERE t.tenant_id = $1 AND t.id = r.id",
            &s.tasks,
        ),
        (
            "DELETE FROM platform.job WHERE tenant_id = $1 AND id NOT IN
               (SELECT r.id FROM jsonb_populate_recordset(NULL::platform.job, $2) r)",
            &s.jobs,
        ),
        (
            "UPDATE platform.job j SET run_at = r.run_at, payload = r.payload,
               attempts = r.attempts, locked_at = r.locked_at, done_at = r.done_at,
               last_error = r.last_error, updated_at = r.updated_at, version = r.version
             FROM jsonb_populate_recordset(NULL::platform.job, $2) r
             WHERE j.tenant_id = $1 AND j.id = r.id",
            &s.jobs,
        ),
        (
            "UPDATE platform.demo_clock c SET now = r.now, is_running = r.is_running,
               updated_at = r.updated_at, version = r.version
             FROM jsonb_populate_recordset(NULL::platform.demo_clock, $2) r
             WHERE c.tenant_id = $1",
            &s.clock,
        ),
    ];
    for (sql, rows) in statements {
        sqlx::query(sql)
            .bind(STORE)
            .bind(rows)
            .execute(&mut *tx)
            .await
            .unwrap();
    }
    // Change sets added since the snapshot go, with what hangs off them (the tasks above no
    // longer point at them).
    let new_sets = "(SELECT id FROM platform.change_set WHERE tenant_id = $1
                     AND NOT (to_jsonb(id) <@ $2::jsonb))";
    for table in [
        "platform.action_log",
        "platform.change_set_event",
        "platform.change_set_item",
        "platform.preview",
    ] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE tenant_id = $1 AND change_set_id IN {new_sets}"
        )))
        .bind(STORE)
        .bind(&s.change_sets)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DELETE FROM platform.change_set WHERE tenant_id = $1 AND id IN {new_sets}"
    )))
    .bind(STORE)
    .bind(&s.change_sets)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// Runs `body`, puts the snapshot back whatever happened, then returns what `body` returned
/// (or raises its panic).
async fn isolated<T>(body: impl Future<Output = T>) -> T {
    let pool = owner().await;
    let before = snapshot(&pool).await;
    let out = AssertUnwindSafe(body).catch_unwind().await;
    restore(&pool, &before).await;
    match out {
        Ok(v) => v,
        Err(p) => std::panic::resume_unwind(p),
    }
}

async fn task_row(pool: &PgPool, id: &str) -> (String, Value, i32) {
    let mut tx = scoped(pool).await;
    let row: (String, Value, i32) = sqlx::query_as(
        "SELECT title, watcher_observation, version FROM platform.task
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(STORE)
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    row
}

async fn same_json(pool: &PgPool, a: &Value, b: &Value) -> bool {
    sqlx::query_scalar("SELECT $1::jsonb = $2::jsonb")
        .bind(a)
        .bind(b)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn seeded<'a>(s: &'a Value, id: &str) -> &'a Value {
    s.as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id)
        .unwrap_or_else(|| panic!("seeded task {id}"))
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn watchers_at_the_demo_clock_reproduce_the_seeded_tasks() {
    let rt = runtime(true).await;
    let pool = owner().await;
    let seed = snapshot(&pool).await;
    let registry = acom_registry::build().unwrap();
    let cx = rt
        .context(Principal::system(tenant(), "sys_test"), "req_it".into())
        .await
        .unwrap();
    let at = cx.clock.now();
    assert_eq!(cx.time.format(at), "2026-11-06T11:00:00-05:00");

    let (ran, after) = isolated(async {
        let mut ran = Vec::new();
        // Twice: the second run must refresh the same tasks, not add new ones.
        for round in 0..2 {
            let mut tx = scoped(&pool).await;
            for w in [
                "returns_over_refund_threshold",
                "stock_out_forecast",
                "price_below_cost",
            ] {
                sqlx::query(
                    "INSERT INTO platform.job (tenant_id, id, kind, run_at) VALUES ($1, $2, $3, $4)",
                )
                .bind(STORE)
                .bind(format!("job_it_{w}_{round}"))
                .bind(format!("watcher.{w}"))
                .bind(at)
                .execute(&mut *tx)
                .await
                .unwrap();
            }
            tx.commit().await.unwrap();
            ran.extend(
                acom_changeset::jobs::run_due(&registry, &cx, at)
                    .await
                    .unwrap(),
            );
        }
        (ran, snapshot(&pool).await)
    })
    .await;

    // Each of the six jobs ran once, and only those (the seeded watchers are due later).
    let ids: Vec<&str> = ran.iter().map(|j| j.job_id.as_str()).collect();
    assert_eq!(ids.len(), 6, "{ran:#?}");
    assert!(ids.iter().all(|i| i.starts_with("job_it_")), "{ids:?}");
    assert!(ran.iter().all(|j| j.state.as_str() == "done"), "{ran:#?}");

    // No task was added: the open tasks of the same watcher and cause were refreshed.
    let count = |s: &Value| s.as_array().unwrap().len();
    assert_eq!(count(&after.tasks), count(&seed.tasks));
    let returns = ran
        .iter()
        .find(|j| j.kind == "watcher.returns_over_refund_threshold")
        .unwrap();
    assert_eq!(returns.tasks.len(), 1);
    assert_eq!(returns.tasks[0].task_id, "task_returns_1106");
    assert!(!returns.tasks[0].created);
    let stock = ran
        .iter()
        .find(|j| j.kind == "watcher.stock_out_forecast")
        .unwrap();
    assert_eq!(
        stock
            .tasks
            .iter()
            .map(|t| (t.task_id.as_str(), t.created))
            .collect::<Vec<_>>(),
        [("task_linen_reorder", false)],
        "only the linen shirt sells out within 5 days"
    );
    let price = ran
        .iter()
        .find(|j| j.kind == "watcher.price_below_cost")
        .unwrap();
    assert!(price.tasks.is_empty(), "nothing is under cost after #1040");

    // The observations are the seeded ones, and the tasks were updated (twice).
    for id in ["task_returns_1106", "task_linen_reorder"] {
        let was = seeded(&seed.tasks, id);
        let now = seeded(&after.tasks, id);
        assert!(
            same_json(
                &pool,
                &was["watcher_observation"],
                &now["watcher_observation"]
            )
            .await,
            "{id}: {} vs {}",
            was["watcher_observation"],
            now["watcher_observation"]
        );
        assert_eq!(was["title"], now["title"], "{id}");
        assert_eq!(
            now["version"].as_i64().unwrap(),
            was["version"].as_i64().unwrap() + 2,
            "{id}"
        );
        assert_eq!(was["task_state"], now["task_state"], "{id}");
    }
    let returns = seeded(&after.tasks, "task_returns_1106");
    assert_eq!(returns["watcher_observation"]["observed"]["count"], 12);
    assert_eq!(
        returns["watcher_observation"]["observed"]["oldest_requested_at"],
        "2026-11-04T09:12:00-05:00"
    );
    let linen = &seeded(&after.tasks, "task_linen_reorder")["watcher_observation"]["observed"];
    assert_eq!(linen["inventory_level"], 38);
    assert_eq!(linen["sold_last_7_days"], 61);
    assert_eq!(linen["days_until_stockout"].to_string(), "4.4");

    // Everything is back as it was.
    let (title, observation, version) = task_row(&pool, "task_linen_reorder").await;
    let was = seeded(&seed.tasks, "task_linen_reorder");
    assert_eq!(title, was["title"].as_str().unwrap());
    assert!(same_json(&pool, &observation, &was["watcher_observation"]).await);
    assert_eq!(i64::from(version), was["version"].as_i64().unwrap());
}

async fn set_prices(pool: &PgPool, offers: &[&str], price: &str) {
    let mut tx = scoped(pool).await;
    sqlx::query(
        "UPDATE catalog.offer SET price = $3::numeric, version = version + 1
         WHERE tenant_id = $1 AND id = ANY($2)",
    )
    .bind(STORE)
    .bind(offers)
    .bind(price)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// Runs the price watcher once at the demo clock and reads back its task, the task's change
/// set, and the prices of `offers`.
async fn price_watch(
    pool: &PgPool,
    round: &str,
    offers: &[&str],
) -> (Vec<acom_changeset::jobs::RanJob>, Value, Value, Vec<String>) {
    let rt = runtime(true).await;
    let registry = acom_registry::build().unwrap();
    let cx = rt
        .context(Principal::system(tenant(), "sys_test"), "req_it".into())
        .await
        .unwrap();
    let at = cx.clock.now();
    let mut tx = scoped(pool).await;
    sqlx::query("INSERT INTO platform.job (tenant_id, id, kind, run_at) VALUES ($1, $2, $3, $4)")
        .bind(STORE)
        .bind(format!("job_it_price_{round}"))
        .bind("watcher.price_below_cost")
        .bind(at)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let ran = acom_changeset::jobs::run_due(&registry, &cx, at)
        .await
        .unwrap();
    let task_id = ran
        .iter()
        .flat_map(|j| &j.tasks)
        .map(|t| t.task_id.clone())
        .next()
        .unwrap_or_default();
    let mut tx = scoped(pool).await;
    let task: Value = sqlx::query_scalar(
        "SELECT coalesce((SELECT to_jsonb(t) FROM platform.task t
                          WHERE tenant_id = $1 AND id = $2), 'null')",
    )
    .bind(STORE)
    .bind(&task_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let cs: Value = sqlx::query_scalar(
        "SELECT coalesce((SELECT to_jsonb(c) FROM platform.change_set c
                          WHERE tenant_id = $1 AND id = $2), 'null')",
    )
    .bind(STORE)
    .bind(task["change_set_id"].as_str().unwrap_or_default())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    let prices: Vec<String> = sqlx::query_scalar(
        "SELECT price::text FROM catalog.offer WHERE tenant_id = $1 AND id = ANY($2) ORDER BY id",
    )
    .bind(STORE)
    .bind(offers)
    .fetch_all(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (ran, task, cs, prices)
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_price_under_cost_is_fixed_under_a_standing_approval_and_waits_otherwise() {
    let pool = owner().await;
    let s_m = [
        "off_breton_tee_m_navy_stripe",
        "off_breton_tee_s_navy_stripe",
    ];
    let l = ["off_breton_tee_l_navy_stripe"];
    let (standing, asked) = isolated(async {
        // The demo store gave this fix a standing approval (seed: run_fix: standing_approval).
        set_prices(&pool, &s_m, "19.00").await;
        let standing = price_watch(&pool, "standing", &s_m).await;
        // A store that did not: the same finding waits for a person.
        let mut tx = scoped(&pool).await;
        sqlx::query(
            "UPDATE platform.policy_override SET partial = $2
             WHERE tenant_id = $1 AND file = 'platform'",
        )
        .bind(STORE)
        .bind(json!({"watchers": {"price_below_cost": {"run_fix": "ask"}}}))
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        set_prices(&pool, &l, "19.00").await;
        let asked = price_watch(&pool, "ask", &l).await;
        (standing, asked)
    })
    .await;

    let (ran, task, cs, prices) = standing;
    assert_eq!(ran.len(), 1, "{ran:#?}");
    assert_eq!(ran[0].state.as_str(), "done", "{ran:#?}");
    assert!(ran[0].tasks[0].created);
    assert_eq!(task["task_origin"], "automation");
    assert_eq!(task["task_state"], "done", "{task:#}");
    assert_eq!(task["title"], "1 product is priced below cost");
    assert_eq!(
        task["summary"],
        "Raised to the price floor at 11:00; undo until Nov 7"
    );
    assert_eq!(cs["change_set_state"], "live", "{cs:#}");
    assert_eq!(cs["created_by"], "sys_watcher");
    assert_eq!(
        cs["approved_by"],
        Value::Null,
        "nobody approved it; the policy did"
    );
    assert!(cs["undo_until"].is_string());
    assert_eq!(prices, ["23.00", "23.00"], "raised to cost $20.00 + 15%");

    let (ran, task, cs, prices) = asked;
    assert_eq!(ran[0].tasks.len(), 1, "{ran:#?}");
    assert!(
        ran[0].tasks[0].created,
        "the first task is done; this is new"
    );
    assert_eq!(task["task_state"], "needs_you", "{task:#}");
    assert_eq!(cs["change_set_state"], "waiting_for_approval", "{cs:#}");
    assert_eq!(cs["approved_by"], Value::Null);
    assert_eq!(prices, ["19.00"], "nothing runs before a person approves");
}

// ---------------------------------------------------------------------------------------------
// Over HTTP: the demo-clock endpoints

/// A throwaway Ed25519 key pair from openssl, as the gateway's would be.
fn key_pair() -> (Vec<u8>, Vec<u8>) {
    let dir = std::env::temp_dir().join(format!(
        "acom-it-clock-{}-{}",
        std::process::id(),
        acom_core::id::new_suffix()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let private = dir.join("k.pem");
    let public = dir.join("p.pem");
    let run = |args: &[&std::ffi::OsStr]| {
        assert!(
            std::process::Command::new("openssl")
                .args(args)
                .status()
                .expect("openssl")
                .success()
        );
    };
    run(&[
        "genpkey".as_ref(),
        "-algorithm".as_ref(),
        "ed25519".as_ref(),
        "-out".as_ref(),
        private.as_os_str(),
    ]);
    run(&[
        "pkey".as_ref(),
        "-pubout".as_ref(),
        "-in".as_ref(),
        private.as_os_str(),
        "-out".as_ref(),
        public.as_os_str(),
    ]);
    let pair = (
        std::fs::read(&private).unwrap(),
        std::fs::read(&public).unwrap(),
    );
    std::fs::remove_dir_all(&dir).ok();
    pair
}

struct Http {
    router: axum::Router,
    signer: Signer,
}

async fn http(demo_mode: bool) -> Http {
    let (private, public) = key_pair();
    let api = acom_http::Api {
        runtime: runtime(demo_mode).await,
        registry: Arc::new(acom_registry::build().unwrap()),
        verifier: Arc::new(Verifier::from_pem(&public, "commerce").unwrap()),
        facade: Arc::new(acom_facade::Facade::build().unwrap()),
    };
    Http {
        router: acom_http::router(Arc::new(api)),
        signer: Signer::from_pem(&private).unwrap(),
    }
}

fn operator(scopes: std::collections::BTreeSet<String>) -> Principal {
    Principal {
        subject: "adm_admin".into(),
        kind: ActorKind::Operator,
        tenant: tenant(),
        surface: Surface::Bo,
        scopes,
        name: Some("Admin".into()),
        session_id: Some("ses_it".into()),
    }
}

impl Http {
    async fn call(
        &self,
        who: &Principal,
        method: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let token = self.signer.sign(who, "req_it", now, 300).unwrap();
        let req = Request::builder()
            .method(method)
            .uri("/v1/platform/demo-clock")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn moving_the_clock_from_friday_to_saturday_runs_the_due_jobs_once() {
    let api = http(true).await;
    let admin = operator(scopes::admin());
    let pool = owner().await;

    let (due, first, second, read) = isolated(async {
        let mut tx = scoped(&pool).await;
        let due: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, kind FROM platform.job WHERE tenant_id = $1 AND done_at IS NULL
               AND run_at <= '2026-11-07T10:00:00-05:00' ORDER BY run_at",
        )
        .bind(STORE)
        .fetch_all(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let first = api
            .call(&admin, "PUT", Some(json!({"now": "2026-11-07T10:00"})))
            .await;
        let second = api
            .call(
                &admin,
                "PUT",
                Some(json!({"now": "2026-11-07T10:00:00-05:00", "is_running": false})),
            )
            .await;
        let read = api.call(&admin, "GET", None).await;
        (due, first, second, read)
    })
    .await;

    // Friday's three watchers were due (the promotion jobs come later in November).
    let mut kinds: Vec<&str> = due.iter().map(|(_, k)| k.as_str()).collect();
    kinds.sort_unstable();
    assert_eq!(
        kinds,
        [
            "watcher.price_below_cost",
            "watcher.returns_over_refund_threshold",
            "watcher.stock_out_forecast"
        ]
    );
    let (status, body) = first;
    assert_eq!(status, StatusCode::OK, "{body:#}");
    assert_eq!(body["now"], "2026-11-07T10:00:00-05:00");
    assert_eq!(body["previous"], "2026-11-06T11:00:00-05:00");
    let ran = body["ran"].as_array().unwrap();
    let mut ran_ids: Vec<&str> = ran.iter().map(|j| j["job_id"].as_str().unwrap()).collect();
    ran_ids.sort_unstable();
    let mut due_ids: Vec<&str> = due.iter().map(|(i, _)| i.as_str()).collect();
    due_ids.sort_unstable();
    assert_eq!(ran_ids, due_ids, "each due job ran exactly once: {body:#}");
    assert!(ran.iter().all(|j| j["state"] == "done"), "{body:#}");
    // Missed runs merge into the latest one, in run_at order, and the next run is after 10:00.
    let returns = ran
        .iter()
        .find(|j| j["kind"] == "watcher.returns_over_refund_threshold")
        .unwrap();
    assert_eq!(returns["run_at"], "2026-11-07T09:40:00-05:00");
    assert_eq!(returns["next_run_at"], "2026-11-07T10:10:00-05:00");
    let order: Vec<&str> = ran.iter().map(|j| j["run_at"].as_str().unwrap()).collect();
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(order, sorted);

    // The same time again runs nothing.
    let (status, body) = second;
    assert_eq!(status, StatusCode::OK, "{body:#}");
    assert_eq!(body["ran"], json!([]), "{body:#}");
    let (status, body) = read;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({"now": "2026-11-07T10:00:00-05:00", "is_running": false, "demo_mode": true})
    );

    // And the store is back at Friday 11:00.
    let (_, body) = api.call(&admin, "GET", None).await;
    assert_eq!(body["now"], "2026-11-06T11:00:00-05:00");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn the_clock_refuses_callers_without_admin_demo() {
    let api = http(true).await;
    let mut without = scopes::admin();
    without.remove("admin:demo");
    let operator_without = operator(without);
    let agent = operator(scopes::admin()).on_behalf();
    let mut agent_with = agent.clone();
    agent_with.scopes.insert("admin:demo".into());
    let target = json!({"now": "2026-11-09T09:00"});

    let (refused, chat, chat_with, read, off) = isolated(async {
        let refused = api
            .call(&operator_without, "PUT", Some(target.clone()))
            .await;
        let chat = api.call(&agent, "PUT", Some(target.clone())).await;
        let chat_with = api.call(&agent_with, "PUT", Some(target.clone())).await;
        let read = api.call(&operator_without, "GET", None).await;
        // Out of demo mode the clock does not move, even for the demo administrator.
        let off = http(false)
            .await
            .call(&operator(scopes::admin()), "PUT", Some(target.clone()))
            .await;
        (refused, chat, chat_with, read, off)
    })
    .await;

    for (status, body) in [&refused, &chat, &chat_with] {
        assert_eq!(*status, StatusCode::FORBIDDEN, "{body:#}");
        assert_eq!(body["error"]["code"], "forbidden");
        assert_eq!(body["error"]["detail"]["required_scope"], "admin:demo");
    }
    let (status, body) = off;
    assert_eq!(status, StatusCode::CONFLICT, "{body:#}");
    assert_eq!(body["error"]["code"], "demo_mode_off");
    // Reading needs only the platform read scope, and nothing moved.
    let (status, body) = read;
    assert_eq!(status, StatusCode::OK, "{body:#}");
    assert_eq!(body["now"], "2026-11-06T11:00:00-05:00");
    assert_eq!(body["demo_mode"], true);
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn a_change_set_drafted_for_a_task_is_linked_to_it_and_asks_for_a_person() {
    let rt = runtime(true).await;
    let pool = owner().await;
    let registry = acom_registry::build().unwrap();
    let cx = rt
        .context(Principal::system(tenant(), "sys_test"), "req_it".into())
        .await
        .unwrap();
    let at = cx.clock.now();
    let row = |id: &'static str| {
        let pool = pool.clone();
        async move {
            let mut tx = scoped(&pool).await;
            let r: (String, Option<String>, Option<String>, Option<String>) = sqlx::query_as(
                "SELECT task_state, change_set_id, conversation_id, summary FROM platform.task
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(STORE)
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
            tx.commit().await.unwrap();
            r
        }
    };
    let draft = |task: Option<&'static str>| {
        let cx = cx.clone();
        async move {
            acom_changeset::change_set::create(
                &cx,
                acom_changeset::NewChangeSet {
                    title: "Reorder hiking socks".into(),
                    request_text: vec!["Draft a reorder for the sizes that will run out.".into()],
                    interpretation: Vec::new(),
                    conversation_id: Some("conv_it_task".into()),
                    task_id: task.map(str::to_owned),
                },
            )
            .await
            .unwrap()
        }
    };

    let (first, second, closed, drafting, waiting) = isolated(async {
        let mut tx = scoped(&pool).await;
        for (id, state) in [("task_it_open", "needs_you"), ("task_it_done", "done")] {
            sqlx::query(
                "INSERT INTO platform.task (tenant_id, id, title, task_state, task_origin,
                   summary, request_text, watcher, start_time)
                 VALUES ($1, $2, 'Hiking socks sells out in about 5 days', $3, 'agent_noticed',
                   '78 left', 'Draft a reorder for the sizes that will run out.',
                   'stock_out_forecast', $4)",
            )
            .bind(STORE)
            .bind(id)
            .bind(state)
            .bind(at)
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();

        let first = draft(Some("task_it_open")).await;
        let drafting = row("task_it_open").await;
        // A second set for the same task stands on its own; so does one for a closed task.
        let second = draft(Some("task_it_open")).await;
        let closed = draft(Some("task_it_done")).await;

        let cs = first["id"].as_str().unwrap();
        acom_changeset::preview(
            &registry,
            &cx,
            "create_reorder",
            json!({"product_id": "prd_hiking_socks",
                   "lines": [{"variant_id": "var_hiking_socks_m_grey", "order_quantity": 60}]}),
            Some(cs),
        )
        .await
        .unwrap();
        acom_changeset::change_set::submit(&cx, cs, 1)
            .await
            .unwrap();
        let waiting = row("task_it_open").await;
        (first, second, closed, drafting, waiting)
    })
    .await;

    let cs = first["id"].as_str().unwrap().to_owned();
    assert_eq!(first["task_id"], "task_it_open");
    assert_eq!(
        drafting,
        (
            "working".to_owned(),
            Some(cs.clone()),
            Some("conv_it_task".to_owned()),
            Some(format!("Drafting change set #{}", first["number"]))
        )
    );
    assert!(second["task_id"].is_null(), "{second}");
    assert!(closed["task_id"].is_null(), "{closed}");
    assert_eq!(
        waiting,
        (
            "needs_you".to_owned(),
            Some(cs),
            Some("conv_it_task".to_owned()),
            Some(format!(
                "Change set #{} is waiting for your approval",
                first["number"]
            ))
        )
    );
}
