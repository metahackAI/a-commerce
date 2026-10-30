//! The commerce service. Design: `docs/backend/commerce.md`, serving: `infra/README.md`.
//!
//! One image, four commands:
//! - `serve` (default): the REST and MCP surfaces and person-only endpoints;
//! - `work`: scheduled jobs and the outbox (deployed separately so it scales on its own);
//! - `migrate`: applies `migrations/` as the owner role, then exits (a Job before rollout);
//! - `healthcheck`: container health probe.
//!
//! `serve` and `work` load the policy files at start and refuse to run if one is invalid, and
//! report ready only when the database answers and every migration is applied.

use std::process::ExitCode;
use std::sync::Arc;

use acom_auth::Verifier;
use acom_policy::{FILES, PolicySet};
use acom_server::Readiness;
use acom_store::{Db, Runtime};
use axum::Router;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "acom-commerce", about = "A-Commerce commerce service", version)]
struct Cli {
    /// Address for `serve`.
    #[arg(long, env = "ACOM_COMMERCE_ADDR", default_value = "127.0.0.1:7001")]
    addr: String,
    /// Address for `work` (health and metrics only).
    #[arg(long, env = "ACOM_WORKER_ADDR", default_value = "127.0.0.1:7003")]
    worker_addr: String,
    /// Service connection (role acom_app; row-level security applies).
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,
    /// Migration connection (role acom_owner), used only by `migrate`.
    #[arg(long, env = "MIGRATION_DATABASE_URL")]
    migration_database_url: Option<String>,
    /// Folder with the policy files.
    #[arg(long, env = "ACOM_POLICY_DIR", default_value = "policies")]
    policy_dir: String,
    /// The gateway's public key (PEM). Every request must carry a token it signed.
    #[arg(long, env = "ACOM_GATEWAY_PUBLIC_KEY_FILE")]
    gateway_public_key_file: Option<String>,
    /// Demo mode: the store's clock is the demo clock (`platform.demo_clock`).
    #[arg(long, env = "ACOM_DEMO_MODE", default_value_t = false)]
    demo_mode: bool,
    /// What `migrate` loads after the migrations: `none`, or `demo` (the demo store, once).
    #[arg(long, env = "ACOM_SEED", value_enum, default_value_t = SeedMode::None)]
    seed: SeedMode,
    /// Folder with the demo store files (`seed/demo`).
    #[arg(long, env = "ACOM_SEED_DIR", default_value = "seed/demo")]
    seed_dir: String,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum SeedMode {
    None,
    Demo,
}

#[derive(Debug, Clone, Copy, Subcommand)]
enum Command {
    /// Serve the API (the default).
    Serve,
    /// Run scheduled jobs and the outbox.
    Work,
    /// Apply migrations as the owner role, then exit. With `ACOM_SEED=demo`, also load the demo
    /// store if it is not there yet.
    Migrate,
    /// Load the demo store (named rows and generated history) as the owner role.
    Seed {
        /// Empty every store table first, then load again.
        #[arg(long)]
        reset: bool,
    },
    /// Exit 0 when this container's /healthz answers.
    Healthcheck {
        /// Probe the worker's port instead of the API's.
        #[arg(long)]
        worker: bool,
    },
}

/// Ready when the database answers and all migrations are applied.
struct DatabaseReady(Option<Db>);

impl Readiness for DatabaseReady {
    fn check(&self) -> std::pin::Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        Box::pin(async move {
            match &self.0 {
                Some(db) => db.migrations_applied().await,
                None => Err("DATABASE_URL is not set".to_owned()),
            }
        })
    }
}

fn load_policies(dir: &str) -> Result<PolicySet, String> {
    let texts = FILES
        .iter()
        .map(|f| {
            std::fs::read_to_string(format!("{dir}/{f}.yml"))
                .map(|t| (*f, t))
                .map_err(|e| format!("{dir}/{f}.yml: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str()))).map_err(|e| e.to_string())
}

fn database(cli: &Cli) -> Option<Db> {
    let url = cli.database_url.as_deref()?;
    match Db::connect_lazy(url, 10) {
        Ok(db) => Some(db),
        Err(e) => {
            tracing::error!(error = %e, "DATABASE_URL is not usable");
            None
        }
    }
}

/// The REST surface: every operation, the change-set endpoints, and the event stream.
fn api(cli: &Cli, db: Db, policy: PolicySet) -> Result<Router, String> {
    let key = cli
        .gateway_public_key_file
        .as_deref()
        .ok_or("ACOM_GATEWAY_PUBLIC_KEY_FILE is not set")?;
    let verifier = Verifier::from_pem_file(key, "commerce").map_err(|e| e.to_string())?;
    let registry = acom_registry::build().map_err(|e| e.join("; "))?;
    tracing::info!(
        operations = registry.len(),
        demo_mode = cli.demo_mode,
        "registry built"
    );
    let api = acom_http::Api {
        runtime: Arc::new(Runtime::new(db, policy, cli.demo_mode)),
        registry: Arc::new(registry),
        verifier: Arc::new(verifier),
    };
    Ok(acom_http::router(Arc::new(api)))
}

/// Loads the demo store: the named rows in `dir`, then the history `dir/generate.yml` describes.
/// A store that is already there is left alone unless `reset`.
async fn seed(db: &Db, dir: &str, reset: bool) -> Result<(), String> {
    use acom_testkit::{generate, load, named};
    let rows = named::load(std::path::Path::new(dir))?;
    let tenant = rows
        .iter()
        .find(|(t, _)| t == "platform.online_store")
        .and_then(|(_, r)| r.get("id").and_then(|v| v.as_str()))
        .ok_or("the seed has no platform.online_store row")?
        .to_owned();
    if reset {
        let n = load::reset(db.pool()).await?;
        tracing::info!(tables = n, "store tables emptied");
    } else if load::is_seeded(db.pool(), &tenant).await? {
        tracing::info!(store = %tenant, "demo store already loaded; nothing to do");
        return Ok(());
    }
    let spec = std::fs::read_to_string(format!("{dir}/generate.yml"))
        .map_err(|e| format!("{dir}/generate.yml: {e}"))?;
    let generated = generate::generate(&rows, &spec)?;
    let mut all = rows;
    all.extend(generated);
    let started = std::time::Instant::now();
    let report = load::load(db.pool(), &all).await?;
    tracing::info!(
        store = %tenant,
        rows = report.total(),
        tables = report.tables.len(),
        seconds = started.elapsed().as_secs_f32(),
        "demo store loaded"
    );
    Ok(())
}

async fn run(cli: Cli, command: Command) -> Result<(), String> {
    match command {
        Command::Healthcheck { .. } => unreachable!("handled before logging starts"),
        Command::Migrate => {
            let url = cli
                .migration_database_url
                .as_deref()
                .ok_or("MIGRATION_DATABASE_URL is not set")?;
            let db = Db::connect(url, 1).await.map_err(|e| e.to_string())?;
            db.migrate().await.map_err(|e| e.to_string())?;
            tracing::info!("migrations applied");
            if cli.seed == SeedMode::Demo {
                seed(&db, &cli.seed_dir, false).await?;
            }
            Ok(())
        }
        Command::Seed { reset } => {
            let url = cli
                .migration_database_url
                .as_deref()
                .ok_or("MIGRATION_DATABASE_URL is not set")?;
            let db = Db::connect(url, 1).await.map_err(|e| e.to_string())?;
            seed(&db, &cli.seed_dir, reset).await
        }
        Command::Serve => {
            let policy = load_policies(&cli.policy_dir)?;
            tracing::info!(policy_version = policy.version(), "policies loaded");
            let db = database(&cli).ok_or("DATABASE_URL is not set or not usable")?;
            let ready = Arc::new(DatabaseReady(Some(db.clone())));
            let app = acom_server::ops_router("commerce", ready).merge(api(&cli, db, policy)?);
            acom_server::serve("commerce", &cli.addr, app)
                .await
                .map_err(|e| e.to_string())
        }
        Command::Work => {
            let policy = load_policies(&cli.policy_dir)?;
            tracing::info!(
                policy_version = policy.version(),
                "worker started; jobs and the outbox run from M2"
            );
            let ready = Arc::new(DatabaseReady(database(&cli)));
            let app = acom_server::ops_router("commerce-worker", ready);
            acom_server::serve("commerce-worker", &cli.worker_addr, app)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Serve);
    if let Command::Healthcheck { worker } = command {
        return acom_server::healthcheck(if worker { &cli.worker_addr } else { &cli.addr }).await;
    }
    acom_server::init_tracing();
    match run(cli, command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, ?command, "commerce stopped");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_repository_policies() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../policies");
        let p = load_policies(dir).unwrap();
        assert!(!p.version().is_empty());
        assert!(load_policies("/nonexistent").is_err());
    }
}
