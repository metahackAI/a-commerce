//! `GET /api/chat/ws`: the chat over one WebSocket per page (docs/api/chat.md §2.6).
//!
//! The browser keeps one connection open and sends each turn over it; the events of the turn
//! come back over it as they happen. Behind the socket every turn is still one call to the
//! agent's event stream, signed for that turn alone: a connection that stays open for an hour
//! never carries a token older than its turn, and a session that ends stops the next turn.
//!
//! The handshake, in order:
//! 1. The upgrade request must come from the store's own pages (`Origin` names this host) and
//!    carry a live session cookie; otherwise it is refused before the upgrade (403, 401).
//! 2. The first frame is `hello` with the session's CSRF token and the surface (`fo`, `bo`);
//!    anything else, or nothing within [`HELLO_SECONDS`], closes the socket.
//! 3. The gateway answers `ready`; from then on the browser sends `turn`, `cancel` and `ping`.
//!
//! The gateway pings every [`HEARTBEAT_SECONDS`] and closes a connection silent for
//! [`SILENT_SECONDS`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade, close_code};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, interval, timeout};

use crate::auth::error;
use crate::session::{SESSION_COOKIE, cookie, hash};
use crate::{Gateway, TURN_TOKEN_SECONDS, now_seconds, on_storefront};

/// The wire protocol's version, in `hello` and `ready`.
pub const PROTOCOL: u32 = 1;
/// How long the gateway waits for `hello` after the upgrade.
const HELLO_SECONDS: u64 = 10;
/// How often the gateway pings an open connection.
const HEARTBEAT_SECONDS: u64 = 20;
/// A connection that sent nothing (not even a pong) for this long is closed.
const SILENT_SECONDS: u64 = 75;
/// Turns one connection may run at once.
const MAX_TURNS: usize = 4;
/// The largest frame the browser may send (a turn with a few picked elements).
const MAX_FRAME_BYTES: usize = 256 * 1024;

/// Close codes (4000–4999 are the application's own).
const CLOSE_UNAUTHENTICATED: u16 = 4001;
const CLOSE_FORBIDDEN: u16 = 4003;
const CLOSE_HELLO_TIMEOUT: u16 = 4008;
const CLOSE_BAD_FRAME: u16 = 4400;

/// What the browser sends.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Incoming {
    Hello {
        surface: String,
        csrf_token: String,
        #[serde(default)]
        protocol: Option<u32>,
    },
    Turn {
        request_id: String,
        turn: Value,
    },
    Cancel {
        request_id: String,
    },
    Ping,
}

/// Whether an upgrade comes from the store's own pages. Browsers always send `Origin` on a
/// WebSocket upgrade, so a cookie riding on another site's socket is refused; a client that is
/// not a browser sends none and is let through (it cannot borrow anyone's cookie).
pub fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    let origin_host = origin
        .split_once("://")
        .map_or(origin, |(_, rest)| rest)
        .trim_end_matches('/');
    let named = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(',').next().unwrap_or(v).trim().to_owned())
    };
    [named("x-forwarded-host"), named(header::HOST.as_str())]
        .into_iter()
        .flatten()
        .any(|h| h.eq_ignore_ascii_case(origin_host))
}

/// The upgrade: refuses other sites' pages and callers without a session before switching.
pub async fn upgrade(
    State(gw): State<Arc<Gateway>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !same_origin(&headers) {
        return error(
            StatusCode::FORBIDDEN,
            "forbidden",
            "forbidden",
            "This connection did not come from the store's own pages.",
        );
    }
    let Some(token) = cookie(&headers, SESSION_COOKIE).map(str::to_owned) else {
        return error(
            StatusCode::UNAUTHORIZED,
            "not_signed_in",
            "unauthenticated",
            "Open a session first: sign in, or continue as a guest.",
        );
    };
    match gw.sessions.find(&token).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            return error(
                StatusCode::UNAUTHORIZED,
                "not_signed_in",
                "unauthenticated",
                "The session has ended. Sign in again, or continue as a guest.",
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "session store");
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "upstream_unavailable",
                "upstream_unavailable",
                "Sessions are unavailable right now. Try again in a moment.",
            );
        }
    }
    ws.max_message_size(MAX_FRAME_BYTES)
        .on_upgrade(move |socket| serve(gw, socket, token))
}

/// What turn tasks hand the connection: a frame to send, or that a turn ended.
enum Out {
    Frame(String),
    Finished(String),
}

fn frame(v: &Value) -> Message {
    Message::Text(Utf8Bytes::from(v.to_string()))
}

async fn close(mut socket: WebSocket, code: u16, reason: &str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: Utf8Bytes::from(reason.to_owned()),
        })))
        .await;
}

/// The next text frame, skipping pings and pongs; `None` when the socket closed.
async fn next_text(socket: &mut WebSocket) -> Option<Result<String, &'static str>> {
    loop {
        match socket.recv().await? {
            Ok(Message::Text(t)) => return Some(Ok(t.to_string())),
            Ok(Message::Binary(_)) => return Some(Err("binary frames are not used")),
            Ok(Message::Ping(_) | Message::Pong(_)) => {}
            Ok(Message::Close(_)) | Err(_) => return None,
        }
    }
}

async fn serve(gw: Arc<Gateway>, mut socket: WebSocket, token: String) {
    // 2. hello
    let hello = match timeout(Duration::from_secs(HELLO_SECONDS), next_text(&mut socket)).await {
        Err(_) => return close(socket, CLOSE_HELLO_TIMEOUT, "hello did not come").await,
        Ok(None) => return,
        Ok(Some(Err(why))) => return close(socket, CLOSE_BAD_FRAME, why).await,
        Ok(Some(Ok(text))) => text,
    };
    let (surface, csrf) = match serde_json::from_str::<Incoming>(&hello) {
        Ok(Incoming::Hello {
            surface,
            csrf_token,
            protocol,
        }) if protocol.unwrap_or(PROTOCOL) == PROTOCOL => (surface, csrf_token),
        Ok(Incoming::Hello { .. }) => {
            return close(socket, CLOSE_BAD_FRAME, "protocol version not served").await;
        }
        _ => return close(socket, CLOSE_BAD_FRAME, "the first frame must be hello").await,
    };
    if surface != "fo" && surface != "bo" {
        return close(socket, CLOSE_BAD_FRAME, "surface must be fo or bo").await;
    }
    let session = match gw.sessions.find(&token).await {
        Ok(Some(s)) => s,
        _ => return close(socket, CLOSE_UNAUTHENTICATED, "the session has ended").await,
    };
    if hash(&csrf) != session.csrf_hash {
        return close(
            socket,
            CLOSE_FORBIDDEN,
            "the CSRF token does not match the session",
        )
        .await;
    }
    let p = if surface == "fo" {
        on_storefront(session.principal.clone())
    } else {
        session.principal.clone()
    };
    let user = crate::auth::user_json(&session.principal);
    let ready = json!({"type": "ready", "protocol": PROTOCOL, "surface": surface, "user": user,
                       "acts_as": p.kind.as_str(), "heartbeat_seconds": HEARTBEAT_SECONDS});
    if socket.send(frame(&ready)).await.is_err() {
        return;
    }

    // 3. turns
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
    let mut running: BTreeMap<String, JoinHandle<()>> = BTreeMap::new();
    let mut heartbeat = interval(Duration::from_secs(HEARTBEAT_SECONDS));
    heartbeat.tick().await;
    let mut last_heard = Instant::now();
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let text = match incoming {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {
                        last_heard = Instant::now();
                        continue;
                    }
                    Some(Ok(Message::Binary(_))) => {
                        close(socket, CLOSE_BAD_FRAME, "binary frames are not used").await;
                        abort_all(&mut running);
                        return;
                    }
                    Some(Ok(Message::Text(t))) => t.to_string(),
                };
                last_heard = Instant::now();
                let reply = match serde_json::from_str::<Incoming>(&text) {
                    Ok(Incoming::Ping) => Some(json!({"type": "pong"})),
                    Ok(Incoming::Hello { .. }) => Some(json!({"type": "error", "code": "invalid_input",
                        "message": "hello is sent once, first."})),
                    Ok(Incoming::Cancel { request_id }) => {
                        if let Some(task) = running.remove(&request_id) {
                            task.abort();
                            Some(json!({"type": "end", "request_id": request_id, "reason": "cancelled"}))
                        } else {
                            None
                        }
                    }
                    Ok(Incoming::Turn { request_id, turn }) => {
                        running.retain(|_, t| !t.is_finished());
                        if request_id.is_empty() || request_id.len() > 64 {
                            Some(json!({"type": "error", "request_id": request_id, "code": "invalid_input",
                                "message": "request_id is 1 to 64 characters."}))
                        } else if running.contains_key(&request_id) {
                            Some(json!({"type": "error", "request_id": request_id, "code": "invalid_input",
                                "message": "A turn with this request_id is still running."}))
                        } else if running.len() >= MAX_TURNS {
                            Some(json!({"type": "error", "request_id": request_id, "code": "rate_limited",
                                "message": "Too many turns at once on this connection; wait for one to end."}))
                        } else {
                            let task = tokio::spawn(run_turn(
                                gw.clone(),
                                token.clone(),
                                surface.clone(),
                                request_id.clone(),
                                turn,
                                tx.clone(),
                            ));
                            running.insert(request_id, task);
                            None
                        }
                    }
                    Err(e) => Some(json!({"type": "error", "code": "invalid_input",
                        "message": format!("The frame is not understood: {e}")})),
                };
                if let Some(r) = reply
                    && socket.send(frame(&r)).await.is_err()
                {
                    break;
                }
            }
            Some(out) = rx.recv() => match out {
                Out::Frame(text) => {
                    if socket.send(Message::Text(Utf8Bytes::from(text))).await.is_err() {
                        break;
                    }
                }
                Out::Finished(request_id) => {
                    running.remove(&request_id);
                }
            },
            _ = heartbeat.tick() => {
                if last_heard.elapsed() > Duration::from_secs(SILENT_SECONDS) {
                    close(socket, close_code::AWAY, "no answer to pings").await;
                    abort_all(&mut running);
                    return;
                }
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
        }
    }
    abort_all(&mut running);
}

fn abort_all(running: &mut BTreeMap<String, JoinHandle<()>>) {
    for (_, task) in std::mem::take(running) {
        task.abort();
    }
}

/// One turn: a fresh token for it, the agent's event stream, each event passed on as it comes.
async fn run_turn(
    gw: Arc<Gateway>,
    token: String,
    surface: String,
    request_id: String,
    turn: Value,
    tx: mpsc::UnboundedSender<Out>,
) {
    let send = |v: Value| {
        let _ = tx.send(Out::Frame(v.to_string()));
    };
    let fail = |code: &str, message: &str| {
        send(json!({"type": "error", "request_id": request_id, "code": code, "message": message}));
        send(json!({"type": "end", "request_id": request_id, "reason": "error"}));
    };
    let outcome = stream_turn(&gw, &token, &surface, &request_id, &turn, &send).await;
    match outcome {
        Ok(()) => send(json!({"type": "end", "request_id": request_id, "reason": "done"})),
        Err((code, message)) => fail(&code, &message),
    }
    let _ = tx.send(Out::Finished(request_id.clone()));
}

async fn stream_turn(
    gw: &Gateway,
    token: &str,
    surface: &str,
    request_id: &str,
    turn: &Value,
    send: &impl Fn(Value),
) -> Result<(), (String, String)> {
    let session = match gw.sessions.find(token).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return Err((
                "not_signed_in".into(),
                "The session has ended. Sign in again, or continue as a guest.".into(),
            ));
        }
        Err(_) => {
            return Err((
                "upstream_unavailable".into(),
                "Sessions are unavailable right now. Try again in a moment.".into(),
            ));
        }
    };
    let principal = if surface == "fo" {
        on_storefront(session.principal)
    } else {
        session.principal
    };
    // A chat turn acts for the person but can never act as the person.
    let trace = format!("req_{}", acom_core::id::new_suffix());
    let signed = gw
        .signer
        .sign(
            &principal.on_behalf(),
            &trace,
            now_seconds(),
            TURN_TOKEN_SECONDS,
        )
        .map_err(|e| {
            tracing::error!(error = %e, "cannot sign an internal token");
            (
                "internal".to_owned(),
                "The gateway could not sign the request.".to_owned(),
            )
        })?;
    let unreachable = |e: reqwest::Error| {
        tracing::warn!(error = %e, "agent unreachable");
        (
            "upstream_unavailable".to_owned(),
            "The assistant could not be reached. Try again in a moment.".to_owned(),
        )
    };
    let mut res = gw
        .http
        .post(format!("{}/chat/{surface}/turns", gw.agent))
        .bearer_auth(signed)
        .header(acom_server::REQUEST_ID, &trace)
        .header(header::ACCEPT, "text/event-stream")
        .header(header::CONTENT_TYPE, "application/json")
        .body(turn.to_string())
        .send()
        .await
        .map_err(unreachable)?;
    if !res.status().is_success() {
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let err = &body["error"];
        return Err((
            err["code"]
                .as_str()
                .unwrap_or("upstream_unavailable")
                .to_owned(),
            err["message"].as_str().map_or_else(
                || format!("The assistant is not available ({status})."),
                str::to_owned,
            ),
        ));
    }
    let mut events = SseReader::default();
    while let Some(chunk) = res.chunk().await.map_err(unreachable)? {
        for e in events.feed(&chunk) {
            send(
                json!({"type": "event", "request_id": request_id, "id": e.id,
                        "event": e.event, "data": e.data}),
            );
        }
    }
    Ok(())
}

/// One event of the agent's stream.
#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    pub id: Option<u64>,
    pub event: String,
    pub data: Value,
}

/// Splits an event stream into events, across chunks cut anywhere (also inside a character).
#[derive(Debug, Default)]
pub struct SseReader {
    pending: Vec<u8>,
}

impl SseReader {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        self.pending.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(end) = self.pending.windows(2).position(|w| w == b"\n\n") {
            let raw: Vec<u8> = self.pending.drain(..end + 2).collect();
            let raw = String::from_utf8_lossy(&raw);
            let mut e = SseEvent {
                id: None,
                event: "message".into(),
                data: Value::Null,
            };
            let mut data = String::new();
            for line in raw.lines() {
                if let Some(v) = line.strip_prefix("id:") {
                    e.id = v.trim().parse().ok();
                } else if let Some(v) = line.strip_prefix("event:") {
                    v.trim().clone_into(&mut e.event);
                } else if let Some(v) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(v.trim_start());
                }
            }
            if data.is_empty() {
                continue;
            }
            e.data = serde_json::from_str(&data).unwrap_or(Value::String(data));
            out.push(e);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn only_the_stores_own_pages_may_open_the_socket() {
        let host = "a-commerce.ailism.team";
        assert!(same_origin(&headers(&[
            ("origin", "https://a-commerce.ailism.team"),
            ("host", host)
        ])));
        assert!(same_origin(&headers(&[
            ("origin", "https://a-commerce.ailism.team"),
            ("host", "172.17.0.1:18180"),
            ("x-forwarded-host", host),
        ])));
        assert!(!same_origin(&headers(&[
            ("origin", "https://evil.example"),
            ("host", host)
        ])));
        assert!(same_origin(&headers(&[
            ("origin", "http://localhost:8080"),
            ("host", "localhost:8080")
        ])));
        // Not a browser: no Origin, no borrowed cookie.
        assert!(same_origin(&headers(&[("host", host)])));
    }

    #[test]
    fn reads_events_split_anywhere() {
        let stream = "id: 1\nevent: turn.started\ndata: {\"turn_id\":\"turn_1\"}\n\nid: 2\nevent: message.delta\ndata: {\"text\":\"caf\u{e9}\"}\n\n";
        let bytes = stream.as_bytes();
        for cut in 1..bytes.len() {
            let mut r = SseReader::default();
            let mut got = r.feed(&bytes[..cut]);
            got.extend(r.feed(&bytes[cut..]));
            assert_eq!(got.len(), 2, "cut at {cut}");
            assert_eq!(got[0].event, "turn.started");
            assert_eq!(got[0].id, Some(1));
            assert_eq!(got[1].data["text"], "caf\u{e9}");
        }
    }

    #[test]
    fn frames_are_read_by_type() {
        let hello: Incoming = serde_json::from_str(
            r#"{"type":"hello","surface":"fo","csrf_token":"x","protocol":1}"#,
        )
        .unwrap();
        assert!(matches!(hello, Incoming::Hello { ref surface, .. } if surface == "fo"));
        let turn: Incoming =
            serde_json::from_str(r#"{"type":"turn","request_id":"r1","turn":{"message":"hi"}}"#)
                .unwrap();
        assert!(matches!(turn, Incoming::Turn { ref request_id, .. } if request_id == "r1"));
        assert!(serde_json::from_str::<Incoming>(r#"{"type":"approve"}"#).is_err());
    }
}
