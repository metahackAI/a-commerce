//! The REST view of the registry.
//!
//! Builds axum routes from operation metadata (reads as GET, writes with `?preview=true` or a
//! `preview_id`, person-only endpoints with their `human:*` scope), serves the tool manifest at
//! `GET /v1/tools`, the change-set endpoints, the store event stream, and turns `ToolError` into
//! `{"error": {...}}` with the status from the error catalog (`docs/api/errors.md`).
//!
//! Every request must carry the gateway's internal token (`Authorization: Bearer …`); the
//! token's `tenant` claim alone decides the store.

mod change_sets;
pub mod coerce;
mod events;
mod idempotency;
mod ops;
mod store;

use std::sync::Arc;

use acom_auth::{Principal, Verifier};
use acom_contract::{ErrorCode, Registry, ToolError};
use acom_store::{Cx, Runtime};
use axum::Router;
use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// What every handler shares.
pub struct Api {
    pub runtime: Arc<Runtime>,
    pub registry: Arc<Registry<Cx>>,
    pub verifier: Arc<Verifier>,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api")
            .field("operations", &self.registry.len())
            .finish()
    }
}

impl Api {
    pub(crate) async fn cx(&self, caller: &Caller) -> Result<Cx, ApiError> {
        Ok(self
            .runtime
            .context(caller.principal.clone(), caller.request_id.clone())
            .await?)
    }
}

/// The REST surface: every registered operation plus the fixed endpoints.
pub fn router(api: Arc<Api>) -> Router {
    let r = ops::routes(&api.registry);
    r.merge(change_sets::routes())
        .merge(events::routes())
        .merge(store::routes())
        .with_state(api)
}

/// A `ToolError` as an HTTP response.
#[derive(Debug)]
pub struct ApiError(pub ToolError);

impl From<ToolError> for ApiError {
    fn from(e: ToolError) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, axum::Json(json!({"error": self.0}))).into_response()
    }
}

/// The verified caller of one request.
#[derive(Debug, Clone)]
pub struct Caller {
    pub principal: Principal,
    pub request_id: String,
}

impl Caller {
    pub(crate) fn require(&self, scope: &str) -> Result<(), ApiError> {
        if self.principal.has_scope(scope) {
            Ok(())
        } else {
            Err(forbidden_scope(scope))
        }
    }
}

pub(crate) fn forbidden_scope(scope: &str) -> ApiError {
    ApiError(
        ToolError::new(
            ErrorCode::FORBIDDEN,
            format!("This needs the `{scope}` permission."),
        )
        .detail(json!({"reason": "scope", "required_scope": scope})),
    )
}

impl FromRequestParts<Arc<Api>> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, api: &Arc<Api>) -> Result<Self, ApiError> {
        let unauthenticated = |why: &str| {
            ApiError(
                ToolError::new(ErrorCode::UNAUTHENTICATED, "Sign in to continue.")
                    .detail(json!({"reason": why})),
            )
        };
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| unauthenticated("not_signed_in"))?;
        let claims = api.verifier.verify(token).map_err(|e| {
            tracing::warn!(error = %e, "internal token rejected");
            unauthenticated("invalid_signature")
        })?;
        let request_id = parts
            .headers
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map_or_else(|| claims.request_id.clone(), str::to_owned);
        Ok(Self {
            principal: claims.principal(),
            request_id,
        })
    }
}
