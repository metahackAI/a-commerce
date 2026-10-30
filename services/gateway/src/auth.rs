//! `/api/auth/*`: sign in, sign out, and who is signed in. Design: `docs/backend/gateway.md` §5.
//!
//! - `POST /api/auth/login {login_id, password}`: operators (argon2id against `platform.admin_user`)
//! - `POST /api/auth/demo-login {persona}`: demo mode only; a demo operator or shopper by id
//! - `POST /api/auth/guest`: a guest session (a cart without signing in)
//! - `POST /api/auth/logout`
//! - `GET  /api/auth/session`: the signed-in person, their scopes, and the CSRF token
//! - `GET  /api/auth/personas`: demo mode only; who `demo-login` offers

use std::sync::{Arc, LazyLock};

use acom_core::enums::ActorKind;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::Gateway;
use crate::session::{CSRF_COOKIE, Issued, Login, SESSION_COOKIE, cookie, hash};

pub fn error(status: StatusCode, code: &str, class: &str, message: &str) -> Response {
    let body =
        json!({"error": {"code": code, "class": class, "message": message, "next_tool": null}});
    (status, Json(body)).into_response()
}

fn unavailable() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "upstream_unavailable",
        "upstream_unavailable",
        "Sign-in is unavailable right now. Try again in a moment.",
    )
}

/// `Set-Cookie` values for a new session, or for clearing one (`max_age` 0).
pub fn cookies(gw: &Gateway, token: &str, csrf: &str, max_age: i64) -> [HeaderValue; 2] {
    let secure = if gw.cookie_secure { "; Secure" } else { "" };
    let session = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}"
    );
    let csrf = format!("{CSRF_COOKIE}={csrf}; Path=/; SameSite=Lax; Max-Age={max_age}{secure}");
    [
        HeaderValue::from_str(&session).unwrap_or(HeaderValue::from_static("")),
        HeaderValue::from_str(&csrf).unwrap_or(HeaderValue::from_static("")),
    ]
}

fn user(p: &acom_auth::Principal) -> Value {
    let kind = match (p.kind, p.subject.starts_with("cus_")) {
        (ActorKind::Operator, _) => "operator",
        (ActorKind::Shopper, true) => "shopper",
        _ => "guest",
    };
    json!({"id": if kind == "guest" { Value::Null } else { json!(p.subject) },
           "kind": kind, "name": p.name, "surface": p.surface})
}

/// The response for a new session: the person, the CSRF token, and both cookies.
fn signed_in(gw: &Gateway, issued: &Issued) -> Response {
    let max_age = (issued.session.expires_at - chrono::Utc::now()).num_seconds();
    let body = json!({
        "ok": true,
        "user": user(&issued.session.principal),
        "scopes": issued.session.principal.scopes,
        "csrf_token": issued.csrf,
        "expires_at": issued.session.expires_at.to_rfc3339(),
    });
    let mut res = Json(body).into_response();
    for c in cookies(gw, &issued.token, &issued.csrf, max_age) {
        res.headers_mut().append(header::SET_COOKIE, c);
    }
    res
}

/// Ends the browser's current session, if any, before a new one starts.
async fn end_current(gw: &Gateway, headers: &HeaderMap) {
    if let Some(token) = cookie(headers, SESSION_COOKIE)
        && let Err(e) = gw.sessions.revoke(token).await
    {
        tracing::warn!(error = %e, "could not end the previous session");
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginBody {
    login_id: String,
    password: String,
}

/// Same work for an unknown id as for a wrong password, so timing does not reveal which exist.
static DECOY: LazyLock<String> =
    LazyLock::new(|| acom_auth::password::hash("decoy password").unwrap_or_default());

pub async fn login(
    State(gw): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: Json<LoginBody>,
) -> Response {
    let login_id = body.login_id.trim().to_lowercase();
    if login_id.is_empty() || body.password.is_empty() {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_input",
            "invalid_input",
            "Enter your login id and password.",
        );
    }
    let checked = gw
        .sessions
        .check_password(&login_id, &body.password, DECOY.as_str())
        .await;
    match checked {
        Ok(Login::Operator(id)) => {
            end_current(&gw, &headers).await;
            match gw.sessions.open(ActorKind::Operator, Some(&id)).await {
                Ok(Some(issued)) => {
                    tracing::info!(subject = %id, "operator signed in");
                    signed_in(&gw, &issued)
                }
                Ok(None) => error(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "forbidden",
                    "This account cannot sign in.",
                ),
                Err(e) => {
                    tracing::error!(error = %e, "session store");
                    unavailable()
                }
            }
        }
        Ok(Login::Rejected) => {
            tracing::info!(login_id = %login_id, "sign-in rejected");
            error(
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "unauthenticated",
                "The login id or password is not right.",
            )
        }
        Ok(Login::Blocked(status)) => error(
            StatusCode::LOCKED,
            "account_locked",
            "locked",
            &format!("This account is {status}. Ask a platform admin to reactivate it."),
        ),
        Err(e) => {
            tracing::error!(error = %e, "session store");
            unavailable()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemoBody {
    /// An operator (`adm_…`) or shopper (`cus_…`) id from `GET /api/auth/personas`, or `guest`.
    persona: String,
}

pub async fn demo_login(
    State(gw): State<Arc<Gateway>>,
    headers: HeaderMap,
    body: Json<DemoBody>,
) -> Response {
    if !gw.sessions.demo_mode {
        return error(
            StatusCode::NOT_FOUND,
            "not_found",
            "not_found",
            "Demo sign-in is off.",
        );
    }
    let persona = body.persona.trim();
    let (kind, subject) = match persona {
        "guest" => (ActorKind::Shopper, None),
        p if p.starts_with("adm_") => (ActorKind::Operator, Some(p)),
        p if p.starts_with("cus_") => (ActorKind::Shopper, Some(p)),
        _ => {
            return error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_input",
                "invalid_input",
                "Choose a persona from /api/auth/personas.",
            );
        }
    };
    end_current(&gw, &headers).await;
    match gw.sessions.open(kind, subject).await {
        Ok(Some(issued)) => signed_in(&gw, &issued),
        Ok(None) => error(
            StatusCode::NOT_FOUND,
            "not_found",
            "not_found",
            "There is no active account with that id in this store.",
        ),
        Err(e) => {
            tracing::error!(error = %e, "session store");
            unavailable()
        }
    }
}

pub async fn guest(State(gw): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    // Keep a live session rather than replacing it (a guest's cart hangs on it).
    if let Some(token) = cookie(&headers, SESSION_COOKIE)
        && let Ok(Some(s)) = gw.sessions.find(token).await
    {
        return Json(json!({"ok": true, "user": user(&s.principal),
                           "csrf_token": cookie(&headers, CSRF_COOKIE)}))
        .into_response();
    }
    match gw.sessions.open(ActorKind::Shopper, None).await {
        Ok(Some(issued)) => signed_in(&gw, &issued),
        Ok(None) => unavailable(),
        Err(e) => {
            tracing::error!(error = %e, "session store");
            unavailable()
        }
    }
}

pub async fn logout(State(gw): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    end_current(&gw, &headers).await;
    let mut res = Json(json!({"ok": true})).into_response();
    for c in cookies(&gw, "", "", 0) {
        res.headers_mut().append(header::SET_COOKIE, c);
    }
    res
}

pub async fn session(State(gw): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    let found = match cookie(&headers, SESSION_COOKIE) {
        Some(token) => gw.sessions.find(token).await,
        None => Ok(None),
    };
    match found {
        Ok(Some(s)) => {
            // Hand the page its CSRF token back only when it matches this session.
            let csrf = cookie(&headers, CSRF_COOKIE).filter(|c| hash(c) == s.csrf_hash);
            Json(json!({
                "signed_in": !matches!(user(&s.principal)["kind"].as_str(), Some("guest")),
                "user": user(&s.principal),
                "scopes": s.principal.scopes,
                "csrf_token": csrf,
                "expires_at": s.expires_at.to_rfc3339(),
                "demo_mode": gw.sessions.demo_mode,
            }))
            .into_response()
        }
        Ok(None) => Json(json!({"signed_in": false, "user": null, "scopes": [],
                                "csrf_token": null, "demo_mode": gw.sessions.demo_mode}))
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "session store");
            unavailable()
        }
    }
}

pub async fn personas(State(gw): State<Arc<Gateway>>) -> Response {
    if !gw.sessions.demo_mode {
        return error(
            StatusCode::NOT_FOUND,
            "not_found",
            "not_found",
            "Demo sign-in is off.",
        );
    }
    match gw.sessions.describe(&gw.demo_personas).await {
        Ok(items) => {
            let total = items.len();
            Json(json!({"items": items, "total": total})).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "session store");
            unavailable()
        }
    }
}
