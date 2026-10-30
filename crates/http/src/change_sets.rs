//! Change-set endpoints that are not tools (docs/api/platform.md#change_set): the runtime's
//! drafting endpoints (`changeset:draft`) and the person-only decisions (`human:*`).
//! The reads (`get_change_set`, `search_change_sets`, `verify_change_set`) are tools in the
//! platform domain.

use std::sync::Arc;

use acom_changeset::change_set::RequestUpdate;
use acom_changeset::{ItemRef, NewChangeSet, change_set};
use acom_contract::{ErrorCode, ToolError};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::idempotency;
use crate::{Api, ApiError, Caller};

pub(crate) fn routes() -> Router<Arc<Api>> {
    Router::new()
        .route("/v1/platform/change-sets", post(create))
        .route(
            "/v1/platform/change-sets/{change_set_id}",
            patch(update_request),
        )
        .route(
            "/v1/platform/change-sets/{change_set_id}/items",
            post(add_item).put(replace_items),
        )
        .route(
            "/v1/platform/change-sets/{change_set_id}/items/{preview_id}",
            delete(remove_item),
        )
        .route(
            "/v1/platform/change-sets/{change_set_id}/submit",
            post(submit),
        )
        .route(
            "/v1/platform/change-sets/{change_set_id}/approve",
            post(approve),
        )
        .route(
            "/v1/platform/change-sets/{change_set_id}/reject",
            post(reject),
        )
        .route("/v1/platform/change-sets/{change_set_id}/undo", post(undo))
        .route("/v1/platform/previews/{preview_id}", get(preview))
}

fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    let v = Value::Object(crate::ops::json_body(body)?);
    serde_json::from_value(v).map_err(|e| ApiError(ToolError::invalid_input(e.to_string())))
}

fn done(r: Result<Value, ApiError>) -> Response {
    match r {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Person-only: the scope must be on a person's own request, and errors carry no next tool.
fn person(caller: &Caller, scope: &str) -> Result<(), ApiError> {
    if caller.principal.can_act_as_person(scope) {
        Ok(())
    } else {
        Err(ApiError(
            ToolError::new(
                ErrorCode::FORBIDDEN,
                "Only a person can do this, from the button on the change set.",
            )
            .detail(json!({"reason": "scope", "required_scope": scope})),
        ))
    }
}

fn no_next(e: ApiError) -> ApiError {
    ApiError(ToolError {
        next_tool: None,
        ..e.0
    })
}

async fn create(State(api): State<Arc<Api>>, caller: Caller, body: Bytes) -> Response {
    done(
        async {
            caller.require("changeset:draft")?;
            let new: NewChangeSet = parse(&body)?;
            let cx = api.cx(&caller).await?;
            Ok(change_set::create(&cx, new).await?)
        }
        .await,
    )
}

async fn update_request(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Path(cs): Path<String>,
    body: Bytes,
) -> Response {
    done(
        async {
            caller.require("changeset:draft")?;
            let update: RequestUpdate = parse(&body)?;
            let cx = api.cx(&caller).await?;
            Ok(change_set::update_request(&cx, &cs, update).await?)
        }
        .await,
    )
}

async fn add_item(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Path(cs): Path<String>,
    body: Bytes,
) -> Response {
    done(
        async {
            caller.require("changeset:draft")?;
            let item: ItemRef = parse(&body)?;
            let cx = api.cx(&caller).await?;
            Ok(change_set::add_item(&cx, &cs, item).await?)
        }
        .await,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Items {
    items: Vec<ItemRef>,
}

async fn replace_items(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Path(cs): Path<String>,
    body: Bytes,
) -> Response {
    done(
        async {
            caller.require("changeset:draft")?;
            let Items { items } = parse(&body)?;
            let cx = api.cx(&caller).await?;
            Ok(change_set::replace_items(&cx, &cs, items).await?)
        }
        .await,
    )
}

async fn remove_item(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Path((cs, preview_id)): Path<(String, String)>,
) -> Response {
    done(
        async {
            caller.require("changeset:draft")?;
            let cx = api.cx(&caller).await?;
            Ok(change_set::remove_item(&cx, &cs, &preview_id).await?)
        }
        .await,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Version {
    version: i32,
}

async fn submit(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Path(cs): Path<String>,
    body: Bytes,
) -> Response {
    done(
        async {
            caller.require("changeset:draft")?;
            let Version { version } = parse(&body)?;
            let cx = api.cx(&caller).await?;
            Ok(change_set::submit(&cx, &cs, version).await?)
        }
        .await,
    )
}

async fn approve(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Path(cs): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    async {
        person(&caller, "human:approve")?;
        let Version { version } = parse(&body)?;
        let cx = api.cx(&caller).await?;
        let endpoint = format!("POST /v1/platform/change-sets/{cs}/approve");
        idempotency::run(
            &cx,
            &headers,
            &endpoint,
            &json!({"version": version}),
            || change_set::approve(&api.registry, &cx, &cs, version),
        )
        .await
    }
    .await
    .unwrap_or_else(|e| no_next(e).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reason {
    reason: String,
}

async fn reject(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Path(cs): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    async {
        person(&caller, "human:reject")?;
        let Reason { reason } = parse(&body)?;
        let cx = api.cx(&caller).await?;
        let endpoint = format!("POST /v1/platform/change-sets/{cs}/reject");
        idempotency::run(&cx, &headers, &endpoint, &json!({"reason": reason}), || {
            change_set::reject(&cx, &cs, &reason)
        })
        .await
    }
    .await
    .unwrap_or_else(|e| no_next(e).into_response())
}

async fn undo(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Path(cs): Path<String>,
    headers: HeaderMap,
) -> Response {
    async {
        person(&caller, "human:undo")?;
        let cx = api.cx(&caller).await?;
        let endpoint = format!("POST /v1/platform/change-sets/{cs}/undo");
        idempotency::run(&cx, &headers, &endpoint, &json!({}), || {
            change_set::undo(&api.registry, &cx, &cs)
        })
        .await
    }
    .await
    .unwrap_or_else(|e| no_next(e).into_response())
}

async fn preview(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Path(preview_id): Path<String>,
) -> Response {
    done(
        async {
            if !caller.principal.has_scope("changeset:draft") {
                caller.require("tools:read:platform")?;
            }
            let cx = api.cx(&caller).await?;
            Ok(acom_changeset::preview::get(&cx, &preview_id).await?)
        }
        .await,
    )
}
