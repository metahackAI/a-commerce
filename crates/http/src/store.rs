//! `GET /v1/platform/store`: the store a request runs in, as the request context sees it — its
//! currency, time zone, and current time (the demo clock in demo mode). Other services (the
//! agent) ask here instead of reading commerce's tables.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::json;

use crate::{Api, Caller};

pub(crate) fn routes() -> Router<Arc<Api>> {
    Router::new().route("/v1/platform/store", get(store))
}

async fn store(State(api): State<Arc<Api>>, caller: Caller) -> Response {
    match api.cx(&caller).await {
        Ok(cx) => axum::Json(json!({
            "store_id": cx.tenant.as_str(),
            "currency": cx.currency.as_str(),
            "time_zone": cx.time.zone.name(),
            "now": cx.time.format(cx.clock.now()),
        }))
        .into_response(),
        Err(e) => e.into_response(),
    }
}
