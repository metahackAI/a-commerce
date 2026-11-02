//! The chat routes (docs/api/chat.md): a turn streamed as server-sent events, the caller's
//! conversations, and a conversation's earlier turns.

use std::convert::Infallible;
use std::sync::Arc;

use acom_agent::{ChatSurface, Emitter, HttpToolClient, Runtime, ToolSet, TurnRequest, encode};
use acom_auth::{Principal, Surface, Verifier};
use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_core::enums::ActorKind;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
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
    /// `ACOM_DEMO_MODE`: only then may a turn use the evaluation's facade (`tool_set: facade`).
    pub demo_mode: bool,
    /// Voice input for the composer, or why it is off (the variables still to set).
    pub voice: Result<crate::voice::Voice, String>,
}

/// The facade outside demo mode (the same code commerce uses for it).
const DEMO_MODE_OFF: ErrorCode = ErrorCode::new("demo_mode_off", ErrorClass::InvalidState);

/// A turn on the screen-shaped facade is for the evaluation, which runs on a demo store.
fn check_tool_set(app: &App, request: &TurnRequest) -> Result<(), ToolError> {
    if request.tool_set == ToolSet::Facade && !app.demo_mode {
        return Err(ToolError::new(
            DEMO_MODE_OFF,
            "The screen-shaped facade exists only for the evaluation, in demo mode.",
        )
        .detail(json!({"field": "tool_set", "reason": "demo_mode_off"})));
    }
    Ok(())
}

/// A recording for the composer: a few minutes of 16 kHz mono WAV at most.
const LARGEST_RECORDING: usize = 10 * 1024 * 1024;

pub fn routes(app: Arc<App>) -> Router {
    Router::new()
        .route("/chat/{surface}/turns", post(turns))
        .route(
            "/chat/{surface}/transcriptions",
            post(crate::voice::transcribe).layer(DefaultBodyLimit::max(LARGEST_RECORDING)),
        )
        .route("/chat/{surface}/conversations", get(conversations))
        .route(
            "/chat/{surface}/conversations/{conversation_id}",
            get(conversation),
        )
        .with_state(app)
}

pub(crate) fn error(e: &ToolError) -> Response {
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
pub(crate) struct Caller {
    principal: Principal,
    token: String,
    pub(crate) request_id: String,
}

pub(crate) fn surface_of(s: &str) -> Result<ChatSurface, ToolError> {
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
pub(crate) fn caller(
    app: &App,
    headers: &HeaderMap,
    surface: ChatSurface,
) -> Result<Caller, ToolError> {
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
        check_tool_set(&app, &request)?;
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

/// `GET /chat/{surface}/conversations`: the caller's conversations on this surface, the most
/// recent first, for the history list. `limit` as every list (docs/api/conventions.md §6):
/// 20 when not given, at most 100.
#[derive(Debug, serde::Deserialize)]
struct ListQuery {
    limit: Option<usize>,
}

async fn conversations(
    State(app): State<Arc<App>>,
    Path(surface): Path<String>,
    Query(q): Query<ListQuery>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let surface = surface_of(&surface)?;
        let c = caller(&app, &headers, surface)?;
        let runtime = app
            .runtime
            .clone()
            .ok_or_else(|| unavailable("the agent definitions did not load (see /readyz)"))?;
        let limit = q.limit.unwrap_or(20).clamp(1, 100);
        // Commerce gives the store's time zone, so the times read as the store's clock.
        let tools =
            HttpToolClient::new(app.http.clone(), &app.commerce_url, &c.token, &c.request_id);
        runtime
            .conversations(&c.principal, surface, limit, &tools)
            .await
    }
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(&e),
    }
}

/// `GET /chat/{surface}/conversations/{id}`: earlier turns and their blocks, for a reload.
/// `?log=true` adds each turn's steps and model calls (the turn log) for the person whose
/// conversation it is.
#[derive(Debug, serde::Deserialize)]
struct ConversationQuery {
    #[serde(default)]
    log: bool,
}

async fn conversation(
    State(app): State<Arc<App>>,
    Path((surface, conversation_id)): Path<(String, String)>,
    Query(q): Query<ConversationQuery>,
    headers: HeaderMap,
) -> Response {
    let result = async {
        let surface = surface_of(&surface)?;
        let c = caller(&app, &headers, surface)?;
        let runtime = app
            .runtime
            .clone()
            .ok_or_else(|| unavailable("the agent definitions did not load (see /readyz)"))?;
        runtime.history(&c.principal, &conversation_id, q.log).await
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
        // One folder per call: tests run in parallel and each removes its folder at the end.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("acom-agent-test-{}-{n}", std::process::id()));
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
        fixture_in(false)
    }

    fn fixture_in(demo_mode: bool) -> Option<Fixture> {
        fixture_with(demo_mode, Err("voice: set META_API_KEY".into()))
    }

    fn fixture_with(
        demo_mode: bool,
        voice: Result<crate::voice::Voice, String>,
    ) -> Option<Fixture> {
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
            demo_mode,
            voice,
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
    async fn the_conversation_list_is_the_callers_own_on_that_surface() {
        let Some(f) = fixture() else {
            eprintln!("openssl not available; skipping");
            return;
        };
        let get = |path: &str, token: Option<&str>| {
            let mut r = Request::get(path);
            if let Some(t) = token {
                r = r.header(header::AUTHORIZATION, format!("Bearer {t}"));
            }
            f.app.clone().oneshot(r.body(Body::empty()).unwrap())
        };
        let res = get("/chat/bo/conversations", None).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        let chat = token(&f, &operator().on_behalf());
        let res = get("/chat/fo/conversations", Some(&chat)).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::FORBIDDEN,
            "a back-office token on the storefront"
        );

        let res = get("/chat/bo/conversations?limit=5", Some(&chat))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&text(res).await).unwrap();
        assert_eq!(body, json!({"items": [], "total": 0}));
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
    async fn the_facade_is_a_demo_mode_turn_only() {
        let body = r#"{"message": "Show the running promotions.", "tool_set": "facade"}"#;
        let Some(f) = fixture_in(false) else {
            eprintln!("openssl not available; skipping");
            return;
        };
        let t = token(&f, &operator().on_behalf());
        let res = f
            .app
            .clone()
            .oneshot(post("/chat/bo/turns", Some(&t), body))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::CONFLICT);
        let text_body = text(res).await;
        assert!(
            text_body.contains("\"code\":\"demo_mode_off\""),
            "{text_body}"
        );
        // A tool set the turn does not know is not a turn request.
        let res = f
            .app
            .clone()
            .oneshot(post(
                "/chat/bo/turns",
                Some(&t),
                r#"{"message": "hi", "tool_set": "screens"}"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // In demo mode the turn starts (and, with no model here, ends saying so).
        let Some(f) = fixture_in(true) else {
            return;
        };
        let t = token(&f, &operator().on_behalf());
        let res = f
            .app
            .clone()
            .oneshot(post("/chat/bo/turns", Some(&t), body))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(text(res).await.contains("event: turn.done"));
    }

    #[tokio::test]
    async fn without_a_key_every_request_is_refused_with_the_reason() {
        let app = routes(Arc::new(App {
            runtime: None,
            verifier: None,
            commerce_url: String::new(),
            http: reqwest::Client::new(),
            demo_mode: false,
            voice: Err("voice: set META_API_KEY".into()),
        }));
        let res = app
            .oneshot(post("/chat/bo/turns", Some("x"), r#"{"message": "hi"}"#))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = text(res).await;
        assert!(body.contains("ACOM_GATEWAY_PUBLIC_KEY_FILE"), "{body}");
    }

    /// A multipart body with a recording and the words on screen, as the browser sends it.
    fn recording(audio: &[u8], keywords: &str) -> (String, Vec<u8>) {
        let b = "acomtestboundary";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{b}\r\nContent-Disposition: form-data; name=\"audio\"; filename=\"s.wav\"\r\n\
                 Content-Type: audio/wav\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(audio);
        body.extend_from_slice(
            format!(
                "\r\n--{b}\r\nContent-Disposition: form-data; name=\"keywords\"\r\n\r\n\
                 {keywords}\r\n--{b}--\r\n"
            )
            .as_bytes(),
        );
        (format!("multipart/form-data; boundary={b}"), body)
    }

    fn wav() -> Vec<u8> {
        let mut w = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        w.resize(128, 0);
        w
    }

    fn post_recording(path: &str, token: &str, audio: &[u8], keywords: &str) -> Request<Body> {
        let (content_type, body) = recording(audio, keywords);
        Request::post(path)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::from(body))
            .unwrap()
    }

    /// A stand-in for the recognizer: checks the key and the settings, then echoes the words
    /// it was told to expect.
    async fn recognizer() -> String {
        async fn transcribe(headers: HeaderMap, mut form: axum::extract::Multipart) -> Response {
            if headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                != Some("Bearer test-key")
            {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            let mut settings = serde_json::Value::Null;
            let mut audio = 0;
            while let Ok(Some(f)) = form.next_field().await {
                let name = f.name().unwrap_or_default().to_owned();
                let bytes = f.bytes().await.unwrap();
                match name.as_str() {
                    "request" => settings = serde_json::from_slice(&bytes).unwrap(),
                    "audio" => audio = bytes.len(),
                    _ => {}
                }
            }
            let words: Vec<&str> = settings["keywords"]
                .as_array()
                .map(|a| a.iter().filter_map(|w| w.as_str()).collect())
                .unwrap_or_default();
            Json(json!({
                "sessionId": "s",
                "transcript": format!(" make {} ten ", words.join(" ")),
                "audioDurationMs": audio,
                "model": settings["model"],
                "languageBias": settings["languageBias"],
                "turns": [],
            }))
            .into_response()
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route("/v1/asr/transcribe", axum::routing::post(transcribe));
        tokio::spawn(async move { axum::serve(listener, app).await.ok() });
        format!("http://{addr}/v1")
    }

    #[tokio::test]
    async fn a_recording_comes_back_as_text_with_the_words_on_screen_as_hints() {
        let base_url = recognizer().await;
        let voice = crate::voice::Voice {
            base_url,
            api_key: "test-key".into(),
            model: "voice-model".into(),
            languages: vec!["English".into()],
        };
        let Some(f) = fixture_with(false, Ok(voice)) else {
            return;
        };
        let t = token(&f, &operator().on_behalf());
        let res = f
            .app
            .clone()
            .oneshot(post_recording(
                "/chat/bo/transcriptions",
                &t,
                &wav(),
                r#"["XL", " XL ", "Cotton utility jacket"]"#,
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&text(res).await).unwrap();
        assert_eq!(body["transcript"], "make XL Cotton utility jacket ten");
        assert_eq!(body["audio_ms"], 128);

        // Not a WAV: refused before anything is sent on.
        let res = f
            .app
            .clone()
            .oneshot(post_recording("/chat/bo/transcriptions", &t, b"OggS", "[]"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
        // The person's own token (not on behalf) is refused like a turn.
        let own = token(&f, &operator());
        let res = f
            .app
            .oneshot(post_recording(
                "/chat/bo/transcriptions",
                &own,
                &wav(),
                "[]",
            ))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn without_voice_settings_the_microphone_says_what_to_set() {
        let Some(f) = fixture() else {
            return;
        };
        let t = token(&f, &operator().on_behalf());
        let res = f
            .app
            .oneshot(post_recording("/chat/bo/transcriptions", &t, &wav(), "[]"))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = text(res).await;
        assert!(body.contains("META_API_KEY"), "{body}");
    }
}
