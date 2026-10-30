//! `GET /v1/platform/events`: the store's progress events as server-sent events
//! (`change_set.updated`, `task.updated`, `job.updated`). The engine writes them with
//! `pg_notify('acom_events', …)` inside the transaction that made the change, so an event is
//! sent only once the change is committed.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures::{Stream, StreamExt};
use serde_json::Value;
use sqlx::postgres::PgListener;

use crate::{Api, ApiError, Caller};

pub(crate) fn routes() -> Router<Arc<Api>> {
    Router::new().route("/v1/platform/events", get(events))
}

async fn events(State(api): State<Arc<Api>>, caller: Caller) -> Response {
    match stream(&api, &caller).await {
        Ok(s) => Sse::new(s)
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
            .into_response(),
        Err(e) => e.into_response(),
    }
}

async fn stream(
    api: &Api,
    caller: &Caller,
) -> Result<impl Stream<Item = Result<Event, Infallible>> + use<>, ApiError> {
    caller.require("tools:read:platform")?;
    let tenant = caller.principal.tenant.as_str().to_owned();
    let mut listener = PgListener::connect_with(api.runtime.db.pool())
        .await
        .map_err(|e| ApiError(acom_store::StoreError::from(e).into()))?;
    listener
        .listen("acom_events")
        .await
        .map_err(|e| ApiError(acom_store::StoreError::from(e).into()))?;
    Ok(listener.into_stream().filter_map(move |n| {
        let tenant = tenant.clone();
        async move {
            let n = n.ok()?;
            let mut v: Value = serde_json::from_str(n.payload()).ok()?;
            if v.get("tenant").and_then(Value::as_str) != Some(tenant.as_str()) {
                return None;
            }
            let kind = v.get("type").and_then(Value::as_str)?.to_owned();
            if let Value::Object(o) = &mut v {
                o.remove("tenant");
                o.remove("type");
            }
            Some(Ok(Event::default().event(kind).data(v.to_string())))
        }
    }))
}
