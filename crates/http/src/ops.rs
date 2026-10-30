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
    let ops: Vec<&'static OpMeta> = registry.iter().map(|op| op.meta()).collect();
    for meta in &ops {
        r = r.route(meta.path, method_router(meta, BTreeMap::new()));
    }
    for (path, meta, fixed) in shadowed(&ops) {
        r = r.route(path, method_router(meta, fixed));
    }
    r
}

/// The handler for one operation. `fixed` are path parameters the route itself stands for (see
/// [`shadowed`]).
fn method_router(meta: &'static OpMeta, fixed: BTreeMap<String, String>) -> MethodRouter<Arc<Api>> {
    let tool = meta.tool;
    let handler = move |State(api): State<Arc<Api>>,
                        caller: Caller,
                        path: Option<Path<BTreeMap<String, String>>>,
                        Query(query): Query<Pairs>,
                        headers: HeaderMap,
                        body: Bytes| {
        let fixed = fixed.clone();
        async move {
            let mut path = path.map(|Path(p)| p).unwrap_or_default();
            path.extend(fixed);
            rest(&api, &caller, tool, path, query, &headers, &body)
                .await
                .unwrap_or_else(IntoResponse::into_response)
        }
    };
    match meta.method {
        Method::Get => get(handler),
        Method::Post => post(handler),
        Method::Put => put(handler),
        Method::Patch => patch(handler),
        Method::Delete => delete(handler),
    }
}

/// A literal path such as `/v1/customer/people/me/preferences` (POST) wins over the pattern
/// `/v1/customer/people/{customer_id}/preferences` (GET) for every method, so a GET to `.../me/...`
/// would get 405. For each pattern that covers another operation's more literal path without
/// sharing its method, the pattern's operation is mounted on that path too, with the literal
/// segments as its parameters (`customer_id = "me"`).
fn shadowed(
    ops: &[&'static OpMeta],
) -> Vec<(&'static str, &'static OpMeta, BTreeMap<String, String>)> {
    let mut out = Vec::new();
    for pattern in ops.iter().filter(|m| m.path.contains('{')) {
        let pat: Vec<&str> = pattern.path.split('/').collect();
        for other in ops {
            if other.path == pattern.path
                || ops
                    .iter()
                    .any(|m| m.path == other.path && m.method == pattern.method)
                || out.iter().any(|(p, m, _): &(&str, &OpMeta, _)| {
                    *p == other.path && m.method == pattern.method
                })
            {
                continue;
            }
            let lit: Vec<&str> = other.path.split('/').collect();
            if lit.len() != pat.len() {
                continue;
            }
            let mut fixed = BTreeMap::new();
            let covers = pat
                .iter()
                .zip(&lit)
                .all(|(p, l)| match p.strip_prefix('{') {
                    Some(name) if !l.starts_with('{') => {
                        fixed.insert(name.trim_end_matches('}').to_owned(), (*l).to_owned());
                        true
                    }
                    Some(_) => true,
                    None => p == l,
                });
            if covers && !fixed.is_empty() {
                out.push((other.path, *pattern, fixed));
            }
        }
    }
    out
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

/// Which tool set a manifest or tool call is for (`?set=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolSet {
    /// A-Commerce's tools (no `set`).
    Commerce,
    /// The screen-shaped facade of the evaluation (`set=facade`).
    Facade,
}

fn tool_set(query: &Pairs) -> Result<ToolSet, ApiError> {
    match query_value(query, "set") {
        None | Some("commerce") => Ok(ToolSet::Commerce),
        Some("facade") => Ok(ToolSet::Facade),
        Some(other) => Err(ToolError::invalid_input(format!(
            "There is no tool set `{other}`; leave `set` out for the store's tools."
        ))
        .detail(json!({"field": "set", "reason": "enum", "allowed": ["commerce", "facade"]}))
        .into()),
    }
}

/// The facade answers only in demo mode, and only to the back office's and the storefront's
/// people (and our agent acting for them): never to outside agents or the system.
fn facade_allowed(api: &Api, caller: &Caller) -> Result<(), ApiError> {
    if !api.runtime.demo_mode() {
        return Err(ToolError::new(
            acom_changeset::clock::DEMO_MODE_OFF,
            "The screen-shaped facade exists only for the evaluation, in demo mode.",
        )
        .detail(json!({"entity": "tool_set", "reason": "demo_mode_off"}))
        .into());
    }
    if !matches!(caller.principal.surface, Surface::Bo | Surface::Fo) {
        return Err(ToolError::new(
            ErrorCode::FORBIDDEN,
            "The screen-shaped facade is not available here.",
        )
        .detail(json!({"reason": "surface"}))
        .into());
    }
    Ok(())
}

/// `GET /v1/tools`: the tools this caller may use, with closed input schemas. `?set=facade`
/// lists the facade's screens instead (demo mode only), filtered the same way.
async fn manifest(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Query(query): Query<Pairs>,
) -> Response {
    async {
        let domain = query_value(&query, "domain");
        let wanted =
            |m: &OpMeta| visible(&caller.principal, m) && domain.is_none_or(|d| m.domain() == d);
        let mut items = match tool_set(&query)? {
            ToolSet::Commerce => api.registry.manifest(wanted),
            ToolSet::Facade => {
                facade_allowed(&api, &caller)?;
                api.facade.manifest(wanted)
            }
        };
        items.sort_by_key(|e| e.meta.tool);
        let total = items.len();
        Ok::<_, ApiError>(Json(json!({"items": items, "total": total})).into_response())
    }
    .await
    .unwrap_or_else(IntoResponse::into_response)
}

/// `POST /v1/tools/{tool}?set=facade`: one screen of the facade, run at once with the caller's
/// context. Body `{"input": {...}}`; there is no preview and no change set. Every real operation
/// the screen touches passes the same audience and scope check as a normal tool call.
async fn call_facade(
    api: &Api,
    caller: &Caller,
    tool: &str,
    body: &[u8],
) -> Result<Response, ApiError> {
    facade_allowed(api, caller)?;
    let screen = api.facade.get(tool).ok_or_else(|| {
        ToolError::new(
            ErrorCode::NOT_FOUND,
            format!("There is no screen named `{tool}`."),
        )
    })?;
    may_use(&caller.principal, screen.meta())?;
    let mut body = json_body(body)?;
    let input = body.remove("input").unwrap_or_else(|| json!({}));
    if let Some(k) = body.keys().next() {
        return Err(ToolError::invalid_input(format!("Unknown field `{k}`.")).into());
    }
    let cx = api.cx(caller).await?;
    let guard = |m: &OpMeta| may_use(&caller.principal, m).map_err(|ApiError(e)| e);
    let out = api
        .facade
        .call(tool, &api.registry, &cx, &guard, input)
        .await?;
    Ok(Json(out).into_response())
}

/// `POST /v1/tools/{tool}`: how the agent runtime calls a tool by name.
///
/// Body `{"input": {...}}` reads or previews; add `change_set_id` to put the preview in a
/// change set, or send `{"preview_id": …}` alone to run a preview that needs no approval.
/// With `?set=facade` the name is one of the facade's screens instead ([`call_facade`]).
async fn call_tool(
    State(api): State<Arc<Api>>,
    caller: Caller,
    Path(tool): Path<String>,
    Query(query): Query<Pairs>,
    body: Bytes,
) -> Response {
    async {
        if tool_set(&query)? == ToolSet::Facade {
            return call_facade(&api, &caller, &tool, &body).await;
        }
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
    use acom_contract::{ActionType, Group};

    const fn meta(tool: &'static str, method: Method, path: &'static str) -> OpMeta {
        OpMeta {
            address: "customer.preference.x",
            tool,
            group: Group::Engage,
            kind: Kind::Read,
            method,
            path,
            summary: "",
            consumes: &[],
            produces: &[],
            checks: &[],
            errors: &[],
            audience: Audience::Both,
            action_type: ActionType::SEARCH,
            person_scope: "",
        }
    }

    static GET_PREFS: OpMeta = meta(
        "get_preferences",
        Method::Get,
        "/v1/customer/people/{customer_id}/preferences",
    );
    static UPDATE_MINE: OpMeta = meta(
        "update_my_preferences",
        Method::Post,
        "/v1/customer/people/me/preferences",
    );
    static GET_PROFILE: OpMeta = meta("get_my_profile", Method::Get, "/v1/customer/people/me");
    static GET_MEMBER: OpMeta = meta(
        "get_member",
        Method::Get,
        "/v1/customer/people/{customer_id}",
    );

    #[test]
    fn patterns_also_answer_on_literal_paths_that_lack_their_method() {
        let found = shadowed(&[&GET_PREFS, &UPDATE_MINE, &GET_PROFILE, &GET_MEMBER]);
        assert_eq!(found.len(), 1, "{found:?}");
        let (path, op, fixed) = &found[0];
        assert_eq!(*path, "/v1/customer/people/me/preferences");
        assert_eq!(op.tool, "get_preferences");
        assert_eq!(fixed.get("customer_id").map(String::as_str), Some("me"));
        // Both GETs on /people/me and /people/{id} already have their own handler.
        let _ = Router::<Arc<Api>>::new()
            .route(
                UPDATE_MINE.path,
                method_router(&UPDATE_MINE, BTreeMap::new()),
            )
            .route(GET_PREFS.path, method_router(&GET_PREFS, BTreeMap::new()))
            .route(path, method_router(op, fixed.clone()));
    }

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

    /// The tool endpoints with the real registry and facade, no database (none of these calls
    /// reaches one), and tokens signed with a throwaway key.
    mod tool_sets {
        use std::collections::BTreeSet;

        use acom_auth::{Signer, Verifier, scopes};
        use acom_core::TenantId;
        use acom_core::enums::ActorKind;
        use acom_store::{Db, Runtime};
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        use super::*;

        /// A throwaway Ed25519 key pair from openssl; `None` (test skipped) without openssl.
        fn keys() -> Option<(Vec<u8>, Vec<u8>)> {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir =
                std::env::temp_dir().join(format!("acom-http-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).ok()?;
            let (private, public) = (dir.join("k.pem"), dir.join("p.pem"));
            let ok = std::process::Command::new("openssl")
                .args(["genpkey", "-algorithm", "ed25519", "-out"])
                .arg(&private)
                .status()
                .ok()?
                .success()
                && std::process::Command::new("openssl")
                    .args(["pkey", "-pubout", "-in"])
                    .arg(&private)
                    .arg("-out")
                    .arg(&public)
                    .status()
                    .ok()?
                    .success();
            let pair = ok.then(|| (std::fs::read(&private).ok(), std::fs::read(&public).ok()));
            std::fs::remove_dir_all(&dir).ok();
            match pair {
                Some((Some(a), Some(b))) => Some((a, b)),
                _ => None,
            }
        }

        struct Fixture {
            router: Router,
            signer: Signer,
            facade: Arc<acom_facade::Facade>,
        }

        fn fixture(demo_mode: bool) -> Option<Fixture> {
            let (private, public) = keys()?;
            let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../policies");
            let texts: Vec<(&str, String)> = acom_policy::FILES
                .iter()
                .map(|f| {
                    (
                        *f,
                        std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
                    )
                })
                .collect();
            let policy =
                acom_policy::PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str())))
                    .unwrap();
            // Never connected: no call below gets as far as the database.
            let db = Db::connect_lazy("postgres://nobody:none@127.0.0.1:9/none", 1).unwrap();
            let facade = Arc::new(acom_facade::Facade::build().unwrap());
            let api = Arc::new(Api {
                runtime: Arc::new(Runtime::new(db, policy, demo_mode)),
                registry: Arc::new(acom_registry::build().unwrap()),
                verifier: Arc::new(Verifier::from_pem(&public, "commerce").unwrap()),
                facade: facade.clone(),
            });
            Some(Fixture {
                router: crate::router(api),
                signer: Signer::from_pem(&private).unwrap(),
                facade,
            })
        }

        fn person(subject: &str, kind: ActorKind, surface: Surface) -> Principal {
            let scopes = if kind == ActorKind::Operator {
                scopes::admin()
            } else {
                scopes::shopper()
            };
            Principal {
                subject: subject.into(),
                kind,
                tenant: TenantId::parse("store_northwind").unwrap(),
                surface,
                scopes,
                name: None,
                session_id: Some("ses_test".into()),
            }
        }

        impl Fixture {
            async fn send(&self, who: &Principal, method: &str, uri: &str) -> (StatusCode, Value) {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let token = self.signer.sign(who, "req_test", now, 300).unwrap();
                let req = Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"input": {}}"#))
                    .unwrap();
                let res = self.router.clone().oneshot(req).await.unwrap();
                let status = res.status();
                let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
                    .await
                    .unwrap();
                (
                    status,
                    serde_json::from_slice(&bytes).unwrap_or(Value::Null),
                )
            }

            fn screens(&self) -> BTreeSet<String> {
                self.facade
                    .iter()
                    .map(|s| s.meta().tool.to_owned())
                    .collect()
            }
        }

        fn tools(body: &Value) -> BTreeSet<String> {
            body["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| i["tool"].as_str().map(str::to_owned))
                .collect()
        }

        #[tokio::test]
        async fn the_normal_manifest_never_lists_a_screen() {
            let Some(f) = fixture(true) else {
                eprintln!("openssl not available; skipping");
                return;
            };
            let screens = f.screens();
            let registry = acom_registry::build().unwrap();
            assert!(screens.iter().all(|s| registry.get(s).is_none()));

            let sj = person("adm_sj", ActorKind::Operator, Surface::Bo).on_behalf();
            let jamie = person("cus_jamie", ActorKind::Shopper, Surface::Fo).on_behalf();
            for who in [&sj, &jamie] {
                let (status, body) = f.send(who, "GET", "/v1/tools").await;
                assert_eq!(status, StatusCode::OK, "{body}");
                let normal = tools(&body);
                assert!(!normal.is_empty());
                assert!(normal.is_disjoint(&screens), "{normal:?}");
            }

            // The facade's own manifest: screens only, filtered by audience and scopes.
            let (status, body) = f.send(&sj, "GET", "/v1/tools?set=facade").await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let bo = tools(&body);
            assert!(bo.is_subset(&screens), "{bo:?}");
            for s in [
                "getPrdGridList",
                "getPrdGridListCnt",
                "savePrmtList",
                "saveRfndProc",
            ] {
                assert!(bo.contains(s), "{s}: {bo:?}");
            }
            assert!(!bo.contains("getCartList"), "a shopper's screen");
            assert_eq!(body["items"][0]["method"], "POST");
            let (_, body) = f.send(&jamie, "GET", "/v1/tools?set=facade").await;
            let fo = tools(&body);
            assert!(
                fo.contains("getCartList") && fo.contains("saveCartList"),
                "{fo:?}"
            );
            assert!(!fo.contains("savePrmtList"), "{fo:?}");

            // A screen is not a tool: without `set=facade` the name is unknown.
            let (status, body) = f.send(&sj, "POST", "/v1/tools/getPrdGridList").await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
            // …and a tool is not a screen.
            let (status, _) = f
                .send(&sj, "POST", "/v1/tools/search_products?set=facade")
                .await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            // A shopper cannot run an operator's screen.
            let (status, body) = f
                .send(&jamie, "POST", "/v1/tools/savePrmtList?set=facade")
                .await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
            let (status, _) = f.send(&sj, "GET", "/v1/tools?set=other").await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        }

        #[tokio::test]
        async fn the_facade_answers_only_in_demo_mode_and_only_to_the_two_chats() {
            let Some(f) = fixture(false) else {
                eprintln!("openssl not available; skipping");
                return;
            };
            let sj = person("adm_sj", ActorKind::Operator, Surface::Bo).on_behalf();
            let (status, body) = f.send(&sj, "GET", "/v1/tools?set=facade").await;
            assert_eq!(status, StatusCode::CONFLICT, "{body}");
            assert_eq!(body["error"]["code"], "demo_mode_off");
            let (status, body) = f
                .send(&sj, "POST", "/v1/tools/getPrdGridList?set=facade")
                .await;
            assert_eq!(status, StatusCode::CONFLICT, "{body}");
            let (status, _) = f.send(&sj, "GET", "/v1/tools").await;
            assert_eq!(status, StatusCode::OK, "the normal set is unaffected");

            let Some(f) = fixture(true) else {
                return;
            };
            let outside = person("cus_jamie", ActorKind::ExternalAgent, Surface::Mcp);
            let (status, body) = f.send(&outside, "GET", "/v1/tools?set=facade").await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        }
    }
}
