//! `GET` and `PUT /v1/platform/demo-clock` (docs/api/platform.md#데모-시계): the store's time,
//! and moving it in demo mode. Runtime-only endpoints, never tools: no model can move time.
//! Moving the clock forward runs the jobs that come due in between before answering
//! (`acom_changeset::clock`).

use std::sync::Arc;

use acom_changeset::clock;
use acom_contract::{ErrorCode, ToolError};
use acom_core::enums::ActorKind;
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Api, ApiError, Caller};

/// The scope that moves the demo clock (docs/backend/gateway.md §2).
pub const DEMO_SCOPE: &str = "admin:demo";

pub(crate) fn routes() -> Router<Arc<Api>> {
    Router::new().route("/v1/platform/demo-clock", get(read).put(move_clock))
}

/// `PUT` body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveClock {
    /// RFC 3339, or a local time in the store's time zone (`2026-11-07T10:00`).
    now: String,
    /// Whether the demo time then moves with real time. Default `false` (it stays put).
    #[serde(default)]
    is_running: bool,
}

fn done(r: Result<Value, ApiError>) -> Response {
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn read(State(api): State<Arc<Api>>, caller: Caller) -> Response {
    done(
        async {
            caller.require("tools:read:platform")?;
            let cx = api.cx(&caller).await?;
            Ok(clock::read(&cx, api.runtime.demo_mode()).await?)
        }
        .await,
    )
}

async fn move_clock(State(api): State<Arc<Api>>, caller: Caller, body: Bytes) -> Response {
    done(
        async {
            // An operator's own request only: a chat turn never moves time.
            if caller.principal.kind != ActorKind::Operator {
                return Err(crate::forbidden_scope(DEMO_SCOPE));
            }
            caller.require(DEMO_SCOPE)?;
            let m: MoveClock = serde_json::from_value(Value::Object(crate::ops::json_body(&body)?))
                .map_err(|e| ApiError(ToolError::invalid_input(e.to_string())))?;
            let cx = api.cx(&caller).await?;
            let now = cx.time.parse(m.now.trim()).map_err(|e| {
                ApiError(
                    ToolError::new(ErrorCode::INVALID_INPUT, e.to_string())
                        .detail(json!({"field": "now", "reason": "format"})),
                )
            })?;
            Ok(clock::move_to(
                &api.registry,
                &cx,
                api.runtime.demo_mode(),
                now,
                m.is_running,
            )
            .await?)
        }
        .await,
    )
}
