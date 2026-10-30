//! The gateway: the only way in for browsers and outside agents. Design: `docs/backend/gateway.md`,
//! serving: `infra/README.md`.
//!
//! Routes (paths seen by the browser → upstream):
//! - `/api/auth/*` → the gateway itself (sessions; see `auth`)
//! - `/api/v1/*`  → commerce `/v1/*` with the person's internal token (tool REST, person-only endpoints)
//! - `/api/chat/*` → agent `/chat/*` with an on-behalf token: the person's scopes minus every
//!   `human:*` scope (SSE, streamed without buffering)
//! - `/mcp`, `/mcp/*` → commerce (outside agents)
//! - everything else → web (Next.js)
//!
//! Every request to commerce or the agent gets a fresh token signed here (Ed25519); a token the
//! browser sends is dropped. State-changing requests from a session must carry the session's
//! `X-CSRF-Token`. A browser without a session that changes something (a guest's first
//! add-to-cart) gets a guest session on the spot.

mod auth;
mod session;

use std::process::ExitCode;
use std::sync::Arc;

use acom_auth::{Principal, Signer, Surface, scopes};
use acom_core::TenantId;
use acom_core::enums::ActorKind;
use acom_server::{REQUEST_ID, Readiness};
use acom_store::Db;
use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::Response;
use axum::routing::{any, get, post};
use clap::{Parser, Subcommand};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

use crate::auth::error;
use crate::session::{SESSION_COOKIE, Session, Sessions, cookie, hash};

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
    /// Database connection (role acom_app) for sessions and accounts.
    #[arg(long, env = "DATABASE_URL")]
    database_url: Option<String>,
    /// The Ed25519 private key that signs internal tokens (PEM).
    #[arg(long, env = "ACOM_GATEWAY_SIGNING_KEY_FILE")]
    signing_key_file: Option<String>,
    /// The store this gateway serves.
    #[arg(long, env = "ACOM_STORE_ID", default_value = "store_northwind")]
    store_id: String,
    /// Demo mode: demo sign-in with personas is on.
    #[arg(long, env = "ACOM_DEMO_MODE", default_value_t = false)]
    demo_mode: bool,
    /// Who demo sign-in offers, comma-separated ids (`adm_…`, `cus_…`, `guest`).
    #[arg(
        long,
        env = "ACOM_DEMO_PERSONAS",
        default_value = "adm_admin,adm_sj,cus_jamie,cus_alex,guest",
        value_delimiter = ','
    )]
    demo_personas: Vec<String>,
    /// Mark cookies `Secure` (on whenever the site is served over HTTPS).
    #[arg(long, env = "ACOM_COOKIE_SECURE", default_value_t = false)]
    cookie_secure: bool,
    /// How long a session lasts, in hours.
    #[arg(long, env = "ACOM_SESSION_HOURS", default_value_t = 12)]
    session_hours: i64,
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

/// Lifetime of a token for one API request, and for one chat turn (the turn's model calls and
/// tool calls all use it).
const REQUEST_TOKEN_SECONDS: u64 = 300;
const TURN_TOKEN_SECONDS: u64 = 900;

/// Everything the handlers share.
#[allow(missing_debug_implementations)]
pub struct Gateway {
    client: Client<HttpConnector, Body>,
    commerce: Arc<str>,
    agent: Arc<str>,
    web: Arc<str>,
    signer: Signer,
    pub sessions: Sessions,
    pub demo_personas: Vec<String>,
    pub cookie_secure: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    Commerce,
    Agent,
    Mcp,
    Web,
}

/// Where a path goes, and the path the upstream should see.
fn route(path: &str) -> (Target, String) {
    if let Some(rest) = path.strip_prefix("/api/v1/") {
        (Target::Commerce, format!("/v1/{rest}"))
    } else if path == "/mcp" || path.starts_with("/mcp/") {
        (Target::Mcp, path.to_owned())
    } else if let Some(rest) = path.strip_prefix("/api/chat/") {
        (Target::Agent, format!("/chat/{rest}"))
    } else {
        (Target::Web, path.to_owned())
    }
}

fn changes_state(m: &Method) -> bool {
    !matches!(*m, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// The caller when there is no session: read-only browsing.
fn anonymous(tenant: &TenantId) -> Principal {
    let mut scopes = scopes::guest();
    scopes.retain(|s| s.starts_with("tools:read:"));
    Principal {
        subject: "anonymous".into(),
        kind: ActorKind::Shopper,
        tenant: tenant.clone(),
        surface: Surface::Fo,
        scopes,
        name: None,
        session_id: None,
    }
}

/// Finds the caller's session, checks CSRF for state changes, and opens a guest session for a
/// first state change. Returns the principal and any cookies to set.
async fn caller(
    gw: &Gateway,
    method: &Method,
    headers: &HeaderMap,
) -> Result<(Principal, Vec<HeaderValue>), Box<Response>> {
    let mutating = changes_state(method);
    let session: Option<Session> = match cookie(headers, SESSION_COOKIE) {
        Some(token) => gw.sessions.find(token).await.map_err(|e| {
            tracing::error!(error = %e, "session store");
            Box::new(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "upstream_unavailable",
                "upstream_unavailable",
                "Sessions are unavailable right now. Try again in a moment.",
            ))
        })?,
        None => None,
    };
    match session {
        Some(s) => {
            if mutating {
                let sent = headers.get("x-csrf-token").and_then(|v| v.to_str().ok());
                if sent.is_none_or(|t| hash(t) != s.csrf_hash) {
                    return Err(Box::new(error(
                        StatusCode::FORBIDDEN,
                        "csrf_mismatch",
                        "forbidden",
                        "This request did not come from the page you signed in on. Reload and try again.",
                    )));
                }
            }
            Ok((s.principal, Vec::new()))
        }
        None if mutating => match gw.sessions.open(ActorKind::Shopper, None).await {
            Ok(Some(issued)) => {
                let max_age = (issued.session.expires_at - chrono::Utc::now()).num_seconds();
                let set = auth::cookies(gw, &issued.token, &issued.csrf, max_age).to_vec();
                Ok((issued.session.principal, set))
            }
            _ => Err(Box::new(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "upstream_unavailable",
                "upstream_unavailable",
                "Sessions are unavailable right now. Try again in a moment.",
            ))),
        },
        None => Ok((anonymous(&gw.sessions.tenant), Vec::new())),
    }
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

async fn proxy(State(gw): State<Arc<Gateway>>, mut req: Request) -> Response {
    let path = req.uri().path().to_owned();
    let (target, upstream_path) = route(&path);
    let base = match target {
        Target::Commerce | Target::Mcp => &gw.commerce,
        Target::Agent => &gw.agent,
        Target::Web => &gw.web,
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

    let request_id = req
        .headers()
        .get(REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("req_unknown")
        .to_owned();
    let mut set_cookies = Vec::new();
    let mut token = None;
    if matches!(target, Target::Commerce | Target::Agent) {
        let method = req.method().clone();
        let headers = req.headers().clone();
        let (principal, cookies) = match caller(&gw, &method, &headers).await {
            Ok(c) => c,
            Err(res) => return *res,
        };
        set_cookies = cookies;
        // A chat turn acts for the person but can never act as the person.
        let (principal, ttl) = if target == Target::Agent {
            (principal.on_behalf(), TURN_TOKEN_SECONDS)
        } else {
            (principal, REQUEST_TOKEN_SECONDS)
        };
        match gw.signer.sign(&principal, &request_id, now_seconds(), ttl) {
            Ok(t) => token = Some(t),
            Err(e) => {
                tracing::error!(error = %e, "cannot sign an internal token");
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal",
                    "The gateway could not sign the request.",
                );
            }
        }
    }

    let original_host = req.headers().get(header::HOST).cloned();
    let headers = req.headers_mut();
    strip_hop_by_hop(headers);
    headers.remove(header::HOST);
    // Only tokens this gateway signs reach the services.
    headers.remove(header::AUTHORIZATION);
    if target != Target::Web {
        headers.remove(header::COOKIE);
    }
    if let Some(t) = token
        && let Ok(v) = HeaderValue::from_str(&format!("Bearer {t}"))
    {
        headers.insert(header::AUTHORIZATION, v);
    }
    if let Some(host) = original_host {
        headers.insert(HeaderName::from_static("x-forwarded-host"), host);
    }
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static(if gw.cookie_secure { "https" } else { "http" }),
    );
    *req.uri_mut() = uri;

    match gw.client.request(req).await {
        Ok(res) => {
            let (mut parts, body) = res.into_parts();
            strip_hop_by_hop(&mut parts.headers);
            if let Ok(id) = HeaderValue::from_str(&request_id) {
                parts.headers.insert(REQUEST_ID, id);
            }
            for c in set_cookies {
                parts.headers.append(header::SET_COOKIE, c);
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

/// Ready when the session store answers.
struct DatabaseReady(Db);

impl Readiness for DatabaseReady {
    fn check(&self) -> std::pin::Pin<Box<dyn Future<Output = Result<(), String>> + Send + '_>> {
        Box::pin(async move {
            sqlx::query("SELECT 1")
                .execute(self.0.pool())
                .await
                .map(|_| ())
                .map_err(|e| format!("database unreachable: {e}"))
        })
    }
}

fn app(gw: Arc<Gateway>) -> Router {
    let ready = Arc::new(DatabaseReady(gw.sessions.db.clone()));
    acom_server::ops_router(SERVICE, ready).merge(
        Router::new()
            .route("/api/auth/login", post(auth::login))
            .route("/api/auth/demo-login", post(auth::demo_login))
            .route("/api/auth/guest", post(auth::guest))
            .route("/api/auth/logout", post(auth::logout))
            .route("/api/auth/session", get(auth::session))
            .route("/api/auth/personas", get(auth::personas))
            .route("/", any(proxy))
            .route("/{*path}", any(proxy))
            .with_state(gw),
    )
}

fn gateway(cli: &Cli) -> Result<Gateway, String> {
    let url = cli
        .database_url
        .as_deref()
        .ok_or("DATABASE_URL is not set")?;
    let db = Db::connect_lazy(url, 10).map_err(|e| e.to_string())?;
    let key = cli
        .signing_key_file
        .as_deref()
        .ok_or("ACOM_GATEWAY_SIGNING_KEY_FILE is not set")?;
    let signer = Signer::from_pem_file(key).map_err(|e| e.to_string())?;
    let tenant = TenantId::parse(cli.store_id.as_str()).map_err(|e| e.to_string())?;
    Ok(Gateway {
        client: Client::builder(TokioExecutor::new()).build_http(),
        commerce: cli.commerce_url.trim_end_matches('/').into(),
        agent: cli.agent_url.trim_end_matches('/').into(),
        web: cli.web_url.trim_end_matches('/').into(),
        signer,
        sessions: Sessions::new(
            db,
            tenant,
            chrono::Duration::hours(cli.session_hours.clamp(1, 24 * 30)),
            cli.demo_mode,
        ),
        demo_personas: cli
            .demo_personas
            .iter()
            .map(|p| p.trim().to_owned())
            .filter(|p| !p.is_empty())
            .collect(),
        cookie_secure: cli.cookie_secure,
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    if matches!(cli.command, Some(Command::Healthcheck)) {
        return acom_server::healthcheck(&cli.addr).await;
    }
    acom_server::init_tracing();
    let gw = match gateway(&cli) {
        Ok(gw) => Arc::new(gw),
        Err(e) => {
            tracing::error!(error = %e, "gateway cannot start");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(store = %cli.store_id, demo_mode = cli.demo_mode, "gateway configured");
    match acom_server::serve(SERVICE, &cli.addr, app(gw)).await {
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
            (Target::Commerce, "/v1/search/products".into())
        );
        assert_eq!(route("/mcp"), (Target::Mcp, "/mcp".into()));
        assert_eq!(
            route("/api/chat/fo/turns"),
            (Target::Agent, "/chat/fo/turns".into())
        );
        assert_eq!(
            route("/bo/change-sets/cs_1042"),
            (Target::Web, "/bo/change-sets/cs_1042".into())
        );
        assert_eq!(route("/"), (Target::Web, "/".into()));
    }

    #[test]
    fn anonymous_callers_only_read() {
        let p = anonymous(&TenantId::parse("store_northwind").unwrap());
        assert!(p.scopes.iter().all(|s| s.starts_with("tools:read:")));
        assert!(!p.scopes.is_empty());
        assert!(p.customer_id().is_none());
        assert!(changes_state(&Method::POST) && !changes_state(&Method::GET));
    }

    #[test]
    fn chat_tokens_never_carry_person_only_scopes() {
        let mut p = anonymous(&TenantId::parse("store_northwind").unwrap());
        p.kind = ActorKind::Operator;
        p.scopes = scopes::admin();
        let agent = p.on_behalf();
        assert!(agent.scopes.iter().all(|s| !s.starts_with("human:")));
        assert!(agent.has_scope("changeset:draft"));
    }
}
