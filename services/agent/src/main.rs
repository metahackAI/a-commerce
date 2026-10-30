//! The agent service: `/chat/{bo,fo}/turns` over SSE. Design: `docs/backend/agent.md`,
//! serving: `infra/README.md`.
//!
//! The runtime (router, tool search, planner, executor, blocks) arrives in M3. The service, its
//! configuration, health, and readiness exist now so every environment runs the same topology.

use std::process::ExitCode;
use std::sync::Arc;

use acom_server::Readiness;
use acom_store::Db;
use axum::Router;
use axum::http::StatusCode;
use axum::routing::post;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "acom-agent", about = "A-Commerce agent service", version)]
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
    /// Agent tables (conversations, turns, model calls); role acom_app.
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,
    /// OpenAI-compatible endpoint serving the worker model (self-hosted vLLM).
    #[arg(long, env = "GLIMMER_BASE_URL")]
    glimmer_base_url: Option<String>,
    /// Meta Model API base URL (planner model).
    #[arg(long, env = "META_API_BASE_URL")]
    meta_api_base_url: Option<String>,
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

struct AgentReady(Option<Db>);

impl Readiness for AgentReady {
    fn check(&self) -> std::pin::Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        Box::pin(async move {
            match &self.0 {
                Some(db) => db.migrations_applied().await,
                None => Err("DATABASE_URL is not set".to_owned()),
            }
        })
    }
}

async fn turns_not_ready() -> (StatusCode, axum::Json<serde_json::Value>) {
    (
        StatusCode::NOT_IMPLEMENTED,
        axum::Json(serde_json::json!({"error": {
            "code": "not_implemented", "class": "internal",
            "message": "Chat turns arrive with the agent runtime (milestone M3).", "next_tool": null}})),
    )
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
    tracing::info!(
        commerce = %cli.commerce_url,
        worker_model = cli.glimmer_base_url.is_some(),
        planner_model = cli.meta_api_base_url.is_some(),
        "agent configured"
    );
    let app = acom_server::ops_router("agent", Arc::new(AgentReady(db)))
        .merge(Router::new().route("/chat/{surface}/turns", post(turns_not_ready)));
    match acom_server::serve("agent", &cli.addr, app).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "agent stopped");
            ExitCode::FAILURE
        }
    }
}
