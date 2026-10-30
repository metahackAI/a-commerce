//! The chat routes (docs/api/chat.md): a turn streamed as server-sent events, and a
//! conversation's earlier turns.

use std::convert::Infallible;
use std::sync::Arc;

use acom_agent::{ChatSurface, Emitter, HttpToolClient, Runtime, TurnRequest, encode};
use acom_auth::{Principal, Surface, Verifier};
use acom_contract::{ErrorCode, ToolError};
use acom_core::enums::ActorKind;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

/// What the chat handlers share.
pub struct App {
    /// `None` when `agents/` did not load; turns then answer 503 with the reason.
    pub runtime: Option<Arc<Runtime>>,
    /// `None` without the gateway's public key; every request is then refused.
    pub verifier: Option<Arc<Verifier>>,
    pub commerce_url: String,
    pub http: reqwest::Client,
}

pub fn routes(app: Arc<App>) -> Router {
    Router::new()
        .route("/chat/{surface}/turns", post(turns))
        .route(
            "/chat/{surface}/conversations/{conversation_id}",
            get(conversation),
        )
        .with_state(app)
}

fn error(e: &ToolError) -> Response {
    let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(json!({"error": e}))).into_response()
}

fn unavailable(why: &str) -> ToolError {
    ToolError::new(
        ErrorCode::UPSTREAM_UNAVAILABLE,
        format!("The assistant is not ready: {why}."),
    )
    .detail(json!({"upstream": "agent", "retryable": false}))
}

/// The verified caller: who, with which token, under which request id.
struct Caller {
    principal: Principal,
    token: String,
    request_id: String,
}

fn surface_of(s: &str) -> Result<ChatSurface, ToolError> {
    ChatSurface::parse(s).ok_or_else(|| {
        ToolError::new(
            ErrorCode::NOT_FOUND,
            "Chats are `bo` (back office) or `fo` (storefront).",
        )
        .detail(json!({"entity": "surface", "id": s}))
    })
}

/// Checks the gateway's token. Only on-behalf tokens for this surface pass, and never one with
/// a person-only scope: the agent acts for a person and never as one.
fn caller(app: &App, headers: &HeaderMap, surface: ChatSurface) -> Result<Caller, ToolError> {
    let verifier = app
        .verifier
        .as_ref()
        .ok_or_else(|| unavailable("ACOM_GATEWAY_PUBLIC_KEY_FILE is not set"))?;
    let unauthenticated = |why: &str| {
        ToolError::new(ErrorCode::UNAUTHENTICATED, "Sign in to continue.")
            .detail(json!({"reason": why}))
    };
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| unauthenticated("not_signed_in"))?;
    let claims = verifier.verify(token).map_err(|e| {
        tracing::warn!(error = %e, "internal token rejected");
        unauthenticated("invalid_signature")
    })?;
    let principal = claims.principal();
    let forbidden = |message: &str, reason: &str| {
        ToolError::new(ErrorCode::FORBIDDEN, message).detail(json!({"reason": reason}))
    };
    if principal.kind != ActorKind::AgentOnBehalf {
        return Err(forbidden(
            "Chat turns come through the gateway's chat route.",
            "kind",
        ));
    }
    if principal.scopes.iter().any(|s| s.starts_with("human:")) {
        return Err(forbidden(
            "A chat token may not carry a person-only permission.",
            "scope",
        ));
    }
    let same = matches!(
        (principal.surface, surface),
        (Surface::Bo, ChatSurface::Bo) | (Surface::Fo, ChatSurface::Fo)
    );
    if !same {
        return Err(forbidden(
            "This chat belongs to the other side of the store.",
            "surface",
        ));
    }
    let request_id = headers
        .get(acom_server::REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .map_or_else(|| claims.request_id.clone(), str::to_owned);
    Ok(Caller {
        principal,
        token: token.to_owned(),
        request_id,
    })
}

/// `POST /chat/{surface}/turns`: one turn, answered as an event stream. Problems found before
/// the turn starts (sign-in, the request body) are plain JSON errors; once it streams, every
/// problem is an `error` event and the stream still ends with `turn.done`.
async fn turns(
    State(app): State<Arc<App>>,
    Path(surface): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = async {
        let surface = surface_of(&surface)?;
        let c = caller(&app, &headers, surface)?;
        let runtime = app
            .runtime
            .clone()
            .ok_or_else(|| unavailable("the agent definitions did not load (see /readyz)"))?;
        let request: TurnRequest = serde_json::from_slice(&body)
            .map_err(|e| ToolError::invalid_input(format!("The turn request is not valid: {e}")))?;
        let prepared = runtime
            .prepare(c.principal, c.request_id.clone(), surface, request)
            .await?;
        let tools = Arc::new(HttpToolClient::new(
            app.http.clone(),
            &app.commerce_url,
            &c.token,
            &c.request_id,
        ));
        let (emit, rx) = Emitter::channel();
        tokio::spawn(async move { runtime.run(prepared, tools, emit).await });
        Ok::<_, ToolError>(rx)
    }
    .await;
    match started {
        Ok(rx) => event_stream(rx),
        Err(e) => error(&e),
    }
}

/// Writes events as they come, numbered from 1, until the turn drops its emitter.
fn event_stream(rx: tokio::sync::mpsc::UnboundedReceiver<acom_agent::Event>) -> Response {
    let frames = futures::stream::unfold((rx, 0u64), |(mut rx, n)| async move {
        let e = rx.recv().await?;
        Some((Ok::<_, Infallible>(encode(n + 1, &e)), (rx, n + 1)))
    });
    let mut res = Response::new(Body::from_stream(frames));
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    // Proxies must pass events on as they come.
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    res
}

/// `GET /chat/{surface}/conversations/{id}`: earlier turns and their blocks, for a reload.
async fn conversation(
    State(app): State<Arc<App>>,
    Path((surface, conversation_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let surface = surface_of(&surface)?;
        let c = caller(&app, &headers, surface)?;
        let runtime = app
            .runtime
            .clone()
            .ok_or_else(|| unavailable("the agent definitions did not load (see /readyz)"))?;
        runtime.history(&c.principal, &conversation_id).await
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(&e),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use acom_agent::store::{FixedStore, MemoryLog, StoreSettings};
    use acom_auth::{Signer, scopes};
    use acom_core::{StoreTime, TenantId};
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    /// A throwaway Ed25519 key pair from openssl; `None` (test skipped) without openssl.
    fn keys() -> Option<(Vec<u8>, Vec<u8>)> {
        let dir = std::env::temp_dir().join(format!("acom-agent-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let (private, public) = (dir.join("k.pem"), dir.join("p.pem"));
        let ok = std::process::Command::new("openssl")
            .args(["genpkey", "-algorithm", "ed25519", "-out"])
            .arg(&private)
            .status()
            .ok()?
            .success()
            && std::process::Command::new("openssl")
                .args(["pkey", "-pubout", "-in"])
                .arg(&private)
                .arg("-out")
                .arg(&public)
                .status()
                .ok()?
                .success();
        let pair = ok.then(|| (std::fs::read(&private).ok(), std::fs::read(&public).ok()));
        std::fs::remove_dir_all(&dir).ok();
        match pair {
            Some((Some(a), Some(b))) => Some((a, b)),
            _ => None,
        }
    }

    fn operator() -> Principal {
        Principal {
            subject: "adm_sj".into(),
            kind: ActorKind::Operator,
            tenant: TenantId::parse("store_northwind").unwrap(),
            surface: Surface::Bo,
            scopes: scopes::admin(),
            name: Some("Son SeongJun".into()),
            session_id: Some("ses_1".into()),
        }
    }

    struct Fixture {
        app: Router,
        signer: Signer,
    }

    fn fixture() -> Option<Fixture> {
        let (private, public) = keys()?;
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../agents");
        let profiles = acom_agent::Profiles::load(&dir).unwrap();
        let models = profiles.models.configure(&|_| None);
        let time = StoreTime::new(chrono_tz::America::New_York);
        let store = FixedStore(StoreSettings {
            now: time.parse("2026-11-06T11:02").unwrap(),
            time,
            currency: "USD".into(),
        });
        let runtime = Runtime::new(
            Arc::new(profiles),
            models,
            Arc::new(MemoryLog::default()),
            Arc::new(store),
        );
        let app = Arc::new(App {
            runtime: Some(Arc::new(runtime)),
            verifier: Some(Arc::new(Verifier::from_pem(&public, "agent").unwrap())),
            // Never reached: without a model the turn ends before any tool call.
            commerce_url: "http://127.0.0.1:9".into(),
            http: reqwest::Client::new(),
        });
        Some(Fixture {
            app: routes(app),
            signer: Signer::from_pem(&private).unwrap(),
        })
    }

    fn token(f: &Fixture, p: &Principal) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        f.signer.sign(p, "req_chat_test", now, 300).unwrap()
    }

    fn post(path: &str, token: Option<&str>, body: &str) -> Request<Body> {
        let mut r = Request::post(path).header(header::CONTENT_TYPE, "application/json");
        if let Some(t) = token {
            r = r.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        r.body(Body::from(body.to_owned())).unwrap()
    }

    async fn text(res: Response) -> String {
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn without_a_model_the_turn_streams_an_error_and_ends() {
        let Some(f) = fixture() else {
            eprintln!("openssl not available; skipping");
            return;
        };
        let t = token(&f, &operator().on_behalf());
        let res = f
            .app
            .clone()
            .oneshot(post(
                "/chat/bo/turns",
                Some(&t),
                r#"{"message": "Run a 20% weekend sale."}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "text/event-stream");
        let body = text(res).await;
        assert!(
            body.starts_with("id: 1\nevent: turn.started\ndata: {"),
            "{body}"
        );
        assert!(body.contains("event: error\ndata: {\"code\":\"upstream_unavailable\",\"message\":\"No model is configured: "), "{body}");
        assert!(body.contains("META_API_KEY"), "{body}");
        assert!(
            body.trim_end()
                .rsplit("\n\n")
                .next()
                .unwrap()
                .contains("event: turn.done"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn only_on_behalf_tokens_for_this_chat_without_person_scopes_pass() {
        let Some(f) = fixture() else {
            eprintln!("openssl not available; skipping");
            return;
        };
        let call = |req| f.app.clone().oneshot(req);

        let res = call(post("/chat/bo/turns", None, r#"{"message": "hi"}"#))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // The person's own token (with human:approve) is not a chat token.
        let own = token(&f, &operator());
        let res = call(post("/chat/bo/turns", Some(&own), r#"{"message": "hi"}"#))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // An on-behalf token that somehow kept a person-only scope is refused too.
        let mut leaky = operator().on_behalf();
        leaky.scopes.insert("human:approve".into());
        let res = call(post(
            "/chat/bo/turns",
            Some(&token(&f, &leaky)),
            r#"{"message": "hi"}"#,
        ))
        .await
        .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        let chat = token(&f, &operator().on_behalf());
        let res = call(post("/chat/fo/turns", Some(&chat), r#"{"message": "hi"}"#))
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::FORBIDDEN,
            "a back-office token on the storefront chat"
        );

        let res = call(post("/chat/xx/turns", Some(&chat), r#"{"message": "hi"}"#))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);

        let res = call(post(
            "/chat/bo/turns",
            Some(&chat),
            r#"{"message": "hi", "approve": true}"#,
        ))
        .await
        .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = text(res).await;
        assert!(body.contains("\"code\":\"invalid_input\""), "{body}");

        let res = call(
            Request::get("/chat/bo/conversations/conv_unknown")
                .header(header::AUTHORIZATION, format!("Bearer {chat}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn without_a_key_every_request_is_refused_with_the_reason() {
        let app = routes(Arc::new(App {
            runtime: None,
            verifier: None,
            commerce_url: String::new(),
            http: reqwest::Client::new(),
        }));
        let res = app
            .oneshot(post("/chat/bo/turns", Some("x"), r#"{"message": "hi"}"#))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = text(res).await;
        assert!(body.contains("ACOM_GATEWAY_PUBLIC_KEY_FILE"), "{body}");
    }
}
