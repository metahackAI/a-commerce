//! The agent service: `/chat/{bo,fo}/turns` over SSE. Design: `docs/backend/agent.md`,
//! wire format: `docs/api/chat.md`, serving: `infra/README.md`.
//!
//! Every request carries the gateway's internal token. The service verifies it (audience
//! `agent`) and hands the same token to commerce for the turn's tool calls; it never makes a
//! token, so it can never widen what the person may do. A chat token never holds `human:*`
//! scopes (the gateway removes them), and one that does is refused here as well.

mod chat;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use acom_agent::store::{PgStore, StoreInfo, SystemStore, TurnLog};
use acom_agent::{Profiles, Runtime};
use acom_auth::Verifier;
use acom_server::Readiness;
use acom_store::Db;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "acom-agent",
    about = "A-Commerce agent service",
    version,
    after_help = "Models come from agents/models.yml, which names their variables: \
                  GLIMMER_BASE_URL and GLIMMER_MODEL (self-hosted vLLM), \
                  META_API_BASE_URL, META_API_KEY and SPARK_MODEL (Meta Model API)."
)]
struct Cli {
    /// Address to listen on.
    #[arg(long, env = "ACOM_AGENT_ADDR", default_value = "127.0.0.1:7002")]
    addr: String,
    /// Commerce service base URL (tool manifest and tool calls).
    #[arg(
        long,
        env = "ACOM_COMMERCE_URL",
        default_value = "http://127.0.0.1:7001"
    )]
    commerce_url: String,
    /// Agent tables (conversations, turns, steps, model calls); role acom_app.
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,
    /// The gateway's public key: chat requests carry tokens it signed (audience `agent`).
    #[arg(long, env = "ACOM_GATEWAY_PUBLIC_KEY_FILE")]
    gateway_public_key_file: Option<String>,
    /// Agent definitions, prompts, and models (`agents/`).
    #[arg(long, env = "ACOM_AGENTS_DIR", default_value = "agents")]
    agents_dir: PathBuf,
    /// Demo mode: the store's clock is the demo clock (`platform.demo_clock`).
    #[arg(long, env = "ACOM_DEMO_MODE", default_value_t = false)]
    demo_mode: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve (the default).
    Serve,
    /// Exit 0 when this container's /healthz answers.
    Healthcheck,
}

/// Ready when the database is migrated, the agent definitions loaded, and tokens can be checked.
struct AgentReady {
    db: Option<Db>,
    problems: Vec<String>,
}

impl Readiness for AgentReady {
    fn check(&self) -> std::pin::Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        Box::pin(async move {
            if !self.problems.is_empty() {
                return Err(self.problems.join("; "));
            }
            match &self.db {
                Some(db) => db.migrations_applied().await,
                None => Err("DATABASE_URL is not set".to_owned()),
            }
        })
    }
}

/// An unset or empty environment variable is `None`.
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Command::Healthcheck)) {
        return acom_server::healthcheck(&cli.addr).await;
    }
    acom_server::init_tracing();
    let db = cli
        .database_url
        .as_deref()
        .and_then(|url| Db::connect_lazy(url, 5).ok());
    let mut problems = Vec::new();

    let verifier = match cli.gateway_public_key_file.as_deref() {
        Some(path) => Verifier::from_pem_file(path, "agent")
            .map(Arc::new)
            .map_err(|e| problems.push(format!("gateway public key: {e}")))
            .ok(),
        None => {
            problems.push("ACOM_GATEWAY_PUBLIC_KEY_FILE is not set".into());
            None
        }
    };

    let runtime = match Profiles::load(&cli.agents_dir) {
        Ok(profiles) => {
            let models = profiles.models.configure(&env);
            if models.is_empty() {
                tracing::warn!(missing = ?models.missing, "no model is configured; turns will say so");
            }
            let (log, store): (Arc<dyn TurnLog>, Arc<dyn StoreInfo>) = match &db {
                // Store time and currency come from commerce (GET /v1/platform/store) each
                // turn; SystemStore is only the fallback when commerce does not answer.
                Some(db) => (Arc::new(PgStore::new(db.clone())), Arc::new(SystemStore)),
                None => (
                    Arc::new(acom_agent::store::MemoryLog::default()),
                    Arc::new(SystemStore),
                ),
            };
            tracing::info!(
                agents = profiles.agents.len(),
                models = ?models.models.keys().collect::<Vec<_>>(),
                "agent definitions loaded"
            );
            Some(Arc::new(Runtime::new(
                Arc::new(profiles),
                models,
                log,
                store,
            )))
        }
        Err(e) => {
            problems.push(format!(
                "agent definitions ({}): {e}",
                cli.agents_dir.display()
            ));
            None
        }
    };
    for p in &problems {
        tracing::error!(problem = %p, "agent is not ready");
    }
    tracing::info!(commerce = %cli.commerce_url, demo_mode = cli.demo_mode, "agent configured");

    let state = Arc::new(chat::App {
        runtime,
        verifier,
        commerce_url: cli.commerce_url.clone(),
        http: reqwest::Client::new(),
    });
    let app = acom_server::ops_router("agent", Arc::new(AgentReady { db, problems }))
        .merge(chat::routes(state));
    match acom_server::serve("agent", &cli.addr, app).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "agent stopped");
            ExitCode::FAILURE
        }
    }
}
