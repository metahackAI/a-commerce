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

use acom_policy::{FILES, PolicySet};
use acom_server::Readiness;
use acom_store::Db;
use axum::Router;
use axum::routing::get;
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
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Clone, Copy, Subcommand)]
enum Command {
    /// Serve the API (the default).
    Serve,
    /// Run scheduled jobs and the outbox.
    Work,
    /// Apply migrations as the owner role, then exit.
    Migrate,
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

/// The tool manifest. Operations are registered from M2; the surface exists from the start so
/// the gateway, the agent, and the web can be wired against it.
async fn tools() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"items": [], "total": 0}))
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
            Ok(())
        }
        Command::Serve => {
            let policy = load_policies(&cli.policy_dir)?;
            tracing::info!(policy_version = policy.version(), "policies loaded");
            let ready = Arc::new(DatabaseReady(database(&cli)));
            let app = acom_server::ops_router("commerce", ready)
                .merge(Router::new().route("/v1/tools", get(tools)));
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
