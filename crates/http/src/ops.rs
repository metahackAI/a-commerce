//! Routes generated from the registry, the tool manifest, and the agent's tool call.

use std::collections::BTreeMap;
use std::sync::Arc;

use acom_auth::{Principal, Surface};
use acom_contract::{Audience, DynOp, ErrorCode, Kind, Method, OpMeta, Registry, ToolError};
use acom_store::Cx;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, delete, get, patch, post, put};
use axum::{Json, Router};
use serde_json::{Map, Value, json};

use crate::coerce::coerce;
use crate::idempotency;
use crate::{Api, ApiError, Caller, forbidden_scope};

type Pairs = Vec<(String, String)>;

pub(crate) fn routes(registry: &Registry<Cx>) -> Router<Arc<Api>> {
    let mut r = Router::new()
        .route("/v1/tools", get(manifest))
        .route("/v1/tools/{tool}", post(call_tool));
    for op in registry.iter() {
        let meta = op.meta();
        let tool = meta.tool;
        let handler = move |State(api): State<Arc<Api>>,
                            caller: Caller,
                            path: Option<Path<BTreeMap<String, String>>>,
                            Query(query): Query<Pairs>,
                            headers: HeaderMap,
                            body: Bytes| async move {
            let path = path.map(|Path(p)| p).unwrap_or_default();
            rest(&api, &caller, tool, path, query, &headers, &body)
                .await
                .unwrap_or_else(IntoResponse::into_response)
        };
        let method: MethodRouter<Arc<Api>> = match meta.method {
            Method::Get => get(handler),
            Method::Post => post(handler),
            Method::Put => put(handler),
            Method::Patch => patch(handler),
            Method::Delete => delete(handler),
        };
        r = r.route(meta.path, method);
    }
    r
}

pub(crate) fn json_body(body: &[u8]) -> Result<Map<String, Value>, ApiError> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(Map::new());
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(_) => Err(ToolError::invalid_input("The body must be a JSON object.").into()),
        Err(e) => Err(ToolError::invalid_input(format!("The body is not valid JSON: {e}")).into()),
    }
}

/// Whether a caller from this surface may use a tool meant for this audience. Scopes are per
/// domain, so this is what keeps a shopper (or a guest) away from operator tools in the same
/// domain, such as `search_orders`, and an operator page away from a shopper's own cart.
fn audience_allows(surface: Surface, audience: Audience) -> bool {
    match audience {
        Audience::Both => true,
        Audience::Operator => matches!(surface, Surface::Bo | Surface::System),
        Audience::Shopper => matches!(surface, Surface::Fo | Surface::Mcp | Surface::System),
        Audience::Internal => surface == Surface::System,
    }
}

fn may_use(p: &Principal, meta: &OpMeta) -> Result<(), ApiError> {
    if !audience_allows(p.surface, meta.audience) {
        return Err(ApiError(
            ToolError::new(
                ErrorCode::FORBIDDEN,
                format!("`{}` is not available here.", meta.tool),
            )
            .detail(json!({"reason": "audience", "audience": meta.audience})),
        ));
    }
    let domain = meta.domain();
    match meta.kind {
        Kind::Read if !p.can_read(domain) => Err(forbidden_scope(&format!("tools:read:{domain}"))),
        Kind::Write | Kind::WriteApproval if !p.can_write(domain) => {
            Err(forbidden_scope(&format!("tools:write:{domain}")))
        }
        Kind::Person if !p.can_act_as_person(meta.person_scope) => Err(ApiError(
            ToolError::new(
                ErrorCode::FORBIDDEN,
                "Only a person can do this, from the button on the page.",
            )
            .detail(json!({"reason": "scope", "required_scope": meta.person_scope})),
        )),
        _ => Ok(()),
    }
}

fn query_value<'a>(query: &'a Pairs, key: &str) -> Option<&'a str> {
    query
        .iter()
        .rev()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// One REST call of a registered operation.
async fn rest(
    api: &Api,
    caller: &Caller,
    tool: &'static str,
    path: BTreeMap<String, String>,
    query: Pairs,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Response, ApiError> {
    let op = api
        .registry
        .get(tool)
        .ok_or_else(|| ToolError::new(ErrorCode::NOT_FOUND, format!("no tool `{tool}`")))?;
    let meta = op.meta();
    may_use(&caller.principal, meta)?;
    let schema = op.input_schema();
    let path_pairs: Pairs = path.into_iter().collect();
    let from_path = coerce(schema, &path_pairs)?;
    match meta.kind {
        Kind::Read => {
            let mut input = coerce(schema, &query)?;
            input.extend(json_body(body)?);
            input.extend(from_path);
            let cx = api.cx(caller).await?;
            Ok(Json(op.read(&cx, Value::Object(input)).await?).into_response())
        }
        Kind::Write | Kind::WriteApproval => {
            let preview = query_value(&query, "preview") == Some("true");
            let mut input = json_body(body)?;
            let cx = api.cx(caller).await?;
            if preview {
                let change_set_id = query_value(&query, "change_set_id");
                if change_set_id.is_some() {
                    caller.require("changeset:draft")?;
                }
                input.extend(from_path);
                let p = acom_changeset::preview(
                    &api.registry,
                    &cx,
                    tool,
                    Value::Object(input),
                    change_set_id,
                )
                .await?;
                return Ok(Json(p).into_response());
            }
            let preview_id = match input.remove("preview_id") {
                Some(Value::String(id)) if input.is_empty() => id,
                Some(_) => {
                    return Err(ToolError::new(
                        ErrorCode::PREVIEW_INPUT_CONFLICT,
                        "Send only `preview_id` to run a preview; the input was fixed when it was previewed.",
                    )
                    .into());
                }
                None => {
                    return Err(ToolError::invalid_input(
                        "Preview first with `?preview=true`, then send `{\"preview_id\": …}`.",
                    )
                    .detail(json!({"field": "preview_id", "reason": "missing"}))
                    .into());
                }
            };
            let done = acom_changeset::commit(&api.registry, &cx, tool, &preview_id).await?;
            Ok(Json(done).into_response())
        }
        Kind::Person => {
            let mut input = json_body(body)?;
            input.extend(from_path);
            let cx = api.cx(caller).await?;
            let endpoint = format!("{} {}", method_word(meta.method), meta.path);
            idempotency::run(
                &cx,
                headers,
                &endpoint,
                &Value::Object(input.clone()),
                || person(op.as_ref(), &cx, Value::Object(input)),
            )
            .await
            .map_err(|ApiError(e)| {
                ApiError(ToolError {
                    next_tool: None,
                    ..e
                })
            })
        }
    }
}

fn method_word(m: Method) -> &'static str {
    match m {
        Method::Get => "GET",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Patch => "PATCH",
        Method::Delete => "DELETE",
    }
}

async fn person(op: &dyn DynOp<Cx>, cx: &Cx, input: Value) -> Result<Value, ToolError> {
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let out = op.person(&mut tx, input).await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(out)
}

/// Whether a tool belongs in this caller's manifest.
fn visible(p: &Principal, meta: &OpMeta) -> bool {
    let audience = audience_allows(p.surface, meta.audience);
    let scoped = match meta.kind {
        Kind::Read => p.can_read(meta.domain()),
        Kind::Write | Kind::WriteApproval => p.can_write(meta.domain()),
        Kind::Person => false,
    };
    meta.is_tool() && audience && scoped
}

/// `GET /v1/tools`: the tools this caller may use, with closed input schemas.
async fn manifest(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Query(query): Query<Pairs>,
) -> Response {
    let domain = query_value(&query, "domain");
    let mut items = api
        .registry
        .manifest(|m| visible(&caller.principal, m) && domain.is_none_or(|d| m.domain() == d));
    items.sort_by_key(|e| e.meta.tool);
    let total = items.len();
    Json(json!({"items": items, "total": total})).into_response()
}

/// `POST /v1/tools/{tool}`: how the agent runtime calls a tool by name.
///
/// Body `{"input": {...}}` reads or previews; add `change_set_id` to put the preview in a
/// change set, or send `{"preview_id": …}` alone to run a preview that needs no approval.
async fn call_tool(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Path(tool): Path<String>,
    body: Bytes,
) -> Response {
    async {
        let op = api.registry.get(&tool).ok_or_else(|| {
            ToolError::new(
                ErrorCode::NOT_FOUND,
                format!("There is no tool named `{tool}`."),
            )
            .next("search_tools")
        })?;
        let meta = op.meta();
        if !meta.is_tool() {
            return Err(ToolError::new(ErrorCode::FORBIDDEN, "This is not a tool.").into());
        }
        may_use(&caller.principal, meta)?;
        let mut body = json_body(&body)?;
        let input = body.remove("input").unwrap_or_else(|| json!({}));
        let preview_id = body.remove("preview_id");
        let change_set_id = body.remove("change_set_id");
        if let Some(k) = body.keys().next() {
            return Err(ToolError::invalid_input(format!("Unknown field `{k}`.")).into());
        }
        let cx = api.cx(&caller).await?;
        let out = match meta.kind {
            Kind::Read => op.read(&cx, input).await?,
            _ => match (preview_id, change_set_id) {
                (Some(Value::String(id)), None) => {
                    json!(acom_changeset::commit(&api.registry, &cx, meta.tool, &id).await?)
                }
                (None, cs) => {
                    let cs = cs.as_ref().and_then(Value::as_str);
                    if cs.is_some() {
                        caller.require("changeset:draft")?;
                    }
                    json!(acom_changeset::preview(&api.registry, &cx, meta.tool, input, cs).await?)
                }
                _ => {
                    return Err(ToolError::invalid_input(
                        "Send either `input` (with an optional `change_set_id`) or `preview_id`.",
                    )
                    .into());
                }
            },
        };
        Ok::<_, ApiError>(Json(out).into_response())
    }
    .await
    .unwrap_or_else(IntoResponse::into_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audiences_follow_the_surface() {
        assert!(audience_allows(Surface::Bo, Audience::Operator));
        assert!(!audience_allows(Surface::Fo, Audience::Operator));
        assert!(!audience_allows(Surface::Mcp, Audience::Operator));
        assert!(audience_allows(Surface::Fo, Audience::Shopper));
        assert!(!audience_allows(Surface::Bo, Audience::Shopper));
        assert!(audience_allows(Surface::Fo, Audience::Both));
        assert!(!audience_allows(Surface::Bo, Audience::Internal));
        assert!(audience_allows(Surface::System, Audience::Internal));
    }
}
