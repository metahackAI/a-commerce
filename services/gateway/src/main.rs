//! The gateway: the only way in for browsers and outside agents. Design: `docs/backend/gateway.md`,
//! serving: `infra/README.md`.
//!
//! Routes (paths seen by the browser → upstream):
//! - `/api/v1/*`  → commerce `/v1/*` (tool REST, person-only endpoints)
//! - `/mcp`, `/mcp/*` → commerce (outside agents)
//! - `/api/chat/*` → agent `/chat/*` (SSE, streamed without buffering)
//! - `/api/auth/*` → the gateway itself (sessions and internal tokens, M3)
//! - everything else → web (Next.js)
//!
//! Bodies stream both ways. Sessions, scopes, and signed internal tokens arrive in M3; until then
//! the gateway forwards requests as they are, with a request id and `x-forwarded-*` headers.

use std::process::ExitCode;
use std::sync::Arc;

use acom_server::{AlwaysReady, REQUEST_ID};
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use clap::{Parser, Subcommand};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

const SERVICE: &str = "gateway";

#[derive(Debug, Parser)]
#[command(name = "acom-gateway", about = "A-Commerce gateway", version)]
struct Cli {
    /// Address to listen on.
    #[arg(long, env = "ACOM_GATEWAY_ADDR", default_value = "127.0.0.1:7000")]
    addr: String,
    /// Commerce service base URL.
    #[arg(
        long,
        env = "ACOM_COMMERCE_URL",
        default_value = "http://127.0.0.1:7001"
    )]
    commerce_url: String,
    /// Agent service base URL.
    #[arg(long, env = "ACOM_AGENT_URL", default_value = "http://127.0.0.1:7002")]
    agent_url: String,
    /// Web (Next.js) base URL.
    #[arg(long, env = "ACOM_WEB_URL", default_value = "http://127.0.0.1:3000")]
    web_url: String,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve (the default).
    Serve,
    /// Exit 0 when this container's /healthz answers (for container health checks).
    Healthcheck,
}

#[derive(Clone)]
struct Upstreams {
    client: Client<HttpConnector, Body>,
    commerce: Arc<str>,
    agent: Arc<str>,
    web: Arc<str>,
}

/// Hop-by-hop headers are about one connection and must not be forwarded.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    for h in HOP_BY_HOP {
        headers.remove(*h);
    }
}

/// Where a path goes, and the path the upstream should see.
fn route(path: &str) -> Option<(Target, String)> {
    if let Some(rest) = path.strip_prefix("/api/v1/") {
        Some((Target::Commerce, format!("/v1/{rest}")))
    } else if path == "/mcp" || path.starts_with("/mcp/") {
        Some((Target::Commerce, path.to_owned()))
    } else if let Some(rest) = path.strip_prefix("/api/chat/") {
        Some((Target::Agent, format!("/chat/{rest}")))
    } else if path.starts_with("/api/auth/") || path == "/api/auth" {
        None
    } else {
        Some((Target::Web, path.to_owned()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Commerce,
    Agent,
    Web,
}

fn error(status: StatusCode, code: &str, class: &str, message: &str) -> Response {
    let body = serde_json::json!({"error": {"code": code, "class": class, "message": message, "next_tool": null}});
    (status, axum::Json(body)).into_response()
}

async fn proxy(State(up): State<Upstreams>, mut req: Request) -> Response {
    let path = req.uri().path().to_owned();
    let Some((target, upstream_path)) = route(&path) else {
        return error(
            StatusCode::NOT_IMPLEMENTED,
            "not_implemented",
            "internal",
            "Sign-in arrives with the gateway's session support (milestone M3).",
        );
    };
    let base = match target {
        Target::Commerce => &up.commerce,
        Target::Agent => &up.agent,
        Target::Web => &up.web,
    };
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let Ok(uri) = format!("{base}{upstream_path}{query}").parse::<Uri>() else {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "invalid_input",
            "The request path is not valid.",
        );
    };

    let original_host = req.headers().get(header::HOST).cloned();
    let headers = req.headers_mut();
    strip_hop_by_hop(headers);
    headers.remove(header::HOST);
    if let Some(host) = original_host {
        headers.insert(HeaderName::from_static("x-forwarded-host"), host);
    }
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("http"),
    );
    *req.uri_mut() = uri;

    let request_id = req.headers().get(REQUEST_ID).cloned();
    match up.client.request(req).await {
        Ok(res) => {
            let (mut parts, body) = res.into_parts();
            strip_hop_by_hop(&mut parts.headers);
            if let Some(id) = request_id {
                parts.headers.insert(REQUEST_ID, id);
            }
            Response::from_parts(parts, Body::new(body))
        }
        Err(e) => {
            tracing::warn!(upstream = ?target, error = %e, "upstream unavailable");
            error(
                StatusCode::BAD_GATEWAY,
                "upstream_unavailable",
                "upstream_unavailable",
                "A service behind the gateway did not answer. Try again in a moment.",
            )
        }
    }
}

fn app(up: Upstreams) -> Router {
    acom_server::ops_router(SERVICE, Arc::new(AlwaysReady)).merge(
        Router::new()
            .route("/", any(proxy))
            .route("/{*path}", any(proxy))
            .with_state(up),
    )
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Command::Healthcheck)) {
        return acom_server::healthcheck(&cli.addr).await;
    }
    acom_server::init_tracing();
    let up = Upstreams {
        client: Client::builder(TokioExecutor::new()).build_http(),
        commerce: cli.commerce_url.trim_end_matches('/').into(),
        agent: cli.agent_url.trim_end_matches('/').into(),
        web: cli.web_url.trim_end_matches('/').into(),
    };
    match acom_server::serve(SERVICE, &cli.addr, app(up)).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "gateway stopped");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_paths_to_the_right_service() {
        assert_eq!(
            route("/api/v1/search/products"),
            Some((Target::Commerce, "/v1/search/products".into()))
        );
        assert_eq!(route("/mcp"), Some((Target::Commerce, "/mcp".into())));
        assert_eq!(
            route("/api/chat/fo/turns"),
            Some((Target::Agent, "/chat/fo/turns".into()))
        );
        assert_eq!(
            route("/bo/change-sets/cs_1042"),
            Some((Target::Web, "/bo/change-sets/cs_1042".into()))
        );
        assert_eq!(route("/"), Some((Target::Web, "/".into())));
        assert_eq!(route("/api/auth/demo-login"), None);
    }
}
