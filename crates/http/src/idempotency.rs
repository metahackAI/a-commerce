//! `Idempotency-Key` for person-only endpoints (docs/api/conventions.md §10): the same key and
//! body within a day returns the first response; the same key with another body is refused.

use acom_contract::{ErrorCode, ToolError};
use acom_store::Cx;
use axum::Json;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::ApiError;

fn db(e: sqlx::Error) -> ApiError {
    ApiError(ToolError::from(acom_store::StoreError::from(e)))
}

fn respond(status: u16, body: Value) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    (status, Json(body)).into_response()
}

/// Runs `f` once per `Idempotency-Key` (or every time when there is no key).
pub(crate) async fn run<F, Fut>(
    cx: &Cx,
    headers: &HeaderMap,
    endpoint: &str,
    request: &Value,
    f: F,
) -> Result<Response, ApiError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Value, ToolError>>,
{
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|k| !k.is_empty())
    else {
        return Ok(Json(f().await?).into_response());
    };
    if key.len() > 200 {
        return Err(ToolError::invalid_input(
            "Idempotency-Key is too long (200 characters at most).",
        )
        .into());
    }
    let digest = Sha256::digest(format!("{endpoint}\n{request}").as_bytes());
    let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();

    let mut tx = cx.begin().await.map_err(|e| ApiError(e.into()))?;
    let existing: Option<(String, Option<i32>, Option<Value>)> = sqlx::query_as(
        "SELECT request_hash, response_status, response FROM platform.idempotency_key
         WHERE tenant_id = $1 AND key = $2 AND expires_at > now() FOR UPDATE",
    )
    .bind(cx.tenant.as_str())
    .bind(key)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    match existing {
        Some((h, _, _)) if h != hash => {
            return Err(ToolError::new(
                ErrorCode::IDEMPOTENCY_CONFLICT,
                "This Idempotency-Key was already used for a different request.",
            )
            .into());
        }
        Some((_, Some(status), Some(body))) => {
            return Ok(respond(u16::try_from(status).unwrap_or(200), body));
        }
        Some(_) => {
            return Err(ToolError::new(
                ErrorCode::IDEMPOTENCY_CONFLICT,
                "The first request with this Idempotency-Key is still running.",
            )
            .into());
        }
        None => {}
    }
    sqlx::query(
        "INSERT INTO platform.idempotency_key (tenant_id, key, endpoint, subject_id, request_hash,
           expires_at) VALUES ($1, $2, $3, $4, $5, now() + interval '24 hours')
         ON CONFLICT (tenant_id, key) DO UPDATE SET endpoint = EXCLUDED.endpoint,
           subject_id = EXCLUDED.subject_id, request_hash = EXCLUDED.request_hash,
           response_status = NULL, response = NULL, expires_at = EXCLUDED.expires_at,
           updated_at = now(), version = platform.idempotency_key.version + 1",
    )
    .bind(cx.tenant.as_str())
    .bind(key)
    .bind(endpoint)
    .bind(&cx.principal.subject)
    .bind(&hash)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    tx.commit().await.map_err(|e| ApiError(e.into()))?;

    let result = f().await;
    let (status, body) = match &result {
        Ok(v) => (200u16, v.clone()),
        Err(e) => (e.http_status(), json!({"error": e})),
    };
    let mut tx = cx.begin().await.map_err(|e| ApiError(e.into()))?;
    if status >= 500 {
        // Let a retry run again.
        sqlx::query("DELETE FROM platform.idempotency_key WHERE tenant_id = $1 AND key = $2")
            .bind(cx.tenant.as_str())
            .bind(key)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
    } else {
        sqlx::query(
            "UPDATE platform.idempotency_key SET response_status = $3, response = $4,
               updated_at = now() WHERE tenant_id = $1 AND key = $2",
        )
        .bind(cx.tenant.as_str())
        .bind(key)
        .bind(i32::from(status))
        .bind(&body)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    tx.commit().await.map_err(|e| ApiError(e.into()))?;
    result.map(|v| Json(v).into_response()).map_err(ApiError)
}
