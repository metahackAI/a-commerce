//! A store's demo clock (`platform.demo_clock`): the time every request of a demo store reads
//! (docs/backend/commerce.md §5.3). Moving it forward runs the scheduled jobs that come due in
//! between, in `run_at` order, before the clock moves, so the scenes (Friday morning operations,
//! Saturday shopping, Monday) play out as they would in real time. Moving it again to the same
//! time runs nothing: every job that ran is done or scheduled after that time.
//!
//! Runtime-only (`GET`/`PUT /v1/platform/demo-clock`), never a tool: the model cannot move time.

use acom_contract::{ErrorClass, ErrorCode, Registry, ToolError};
use acom_store::Cx;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::{db_err, jobs};

/// `PUT /v1/platform/demo-clock` when commerce does not run in demo mode.
pub const DEMO_MODE_OFF: ErrorCode = ErrorCode::new("demo_mode_off", ErrorClass::InvalidState);

/// Whether the store's clock moves with real time: a demo clock that is running, or no demo
/// clock (the system clock).
pub async fn is_running(cx: &Cx, demo_mode: bool) -> Result<bool, ToolError> {
    if !demo_mode {
        return Ok(true);
    }
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let running: Option<bool> =
        sqlx::query_scalar("SELECT is_running FROM platform.demo_clock WHERE tenant_id = $1")
            .bind(cx.tenant.as_str())
            .fetch_optional(&mut *tx.tx)
            .await
            .map_err(db_err)?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(running.unwrap_or(true))
}

/// `GET /v1/platform/demo-clock`: `{now, is_running, demo_mode}`.
pub async fn read(cx: &Cx, demo_mode: bool) -> Result<Value, ToolError> {
    Ok(json!({
        "now": cx.time.format(cx.clock.now()),
        "is_running": is_running(cx, demo_mode).await?,
        "demo_mode": demo_mode,
    }))
}

/// `PUT /v1/platform/demo-clock {now, is_running}`: runs every job due at `now` (each once),
/// then sets the clock. The answer lists the jobs that ran.
pub async fn move_to(
    registry: &Registry<Cx>,
    cx: &Cx,
    demo_mode: bool,
    now: DateTime<Utc>,
    is_running: bool,
) -> Result<Value, ToolError> {
    if !demo_mode {
        return Err(ToolError::new(
            DEMO_MODE_OFF,
            "This store runs on real time; the demo clock only moves in demo mode.",
        )
        .detail(json!({"entity": "demo_clock", "reason": "demo_mode_off"})));
    }
    let previous = cx.clock.now();
    // Jobs first: until the clock moves, the worker does not see them as due, so this request
    // runs them all and can list them.
    let ran = jobs::run_due(registry, cx, now).await?;
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    sqlx::query(
        "INSERT INTO platform.demo_clock (tenant_id, now, is_running) VALUES ($1, $2, $3)
         ON CONFLICT (tenant_id) DO UPDATE SET now = excluded.now, is_running = excluded.is_running,
           updated_at = now(), version = platform.demo_clock.version + 1",
    )
    .bind(cx.tenant.as_str())
    .bind(now)
    .bind(is_running)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    tx.commit().await.map_err(ToolError::from)?;
    tracing::info!(store = %cx.tenant, from = %previous, to = %now, jobs = ran.len(),
                   "demo clock moved");
    Ok(json!({
        "now": cx.time.format(now),
        "is_running": is_running,
        "demo_mode": true,
        "previous": cx.time.format(previous),
        "ran": ran,
    }))
}
