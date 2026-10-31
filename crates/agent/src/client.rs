//! How the agent calls commerce: the `ToolClient` seam, with the HTTP implementation used in
//! service and a scripted one in tests.
//!
//! The client only knows the tool endpoint (`POST /v1/tools/{tool}`), the manifest, and the
//! runtime's change-set drafting endpoints. It has no way to approve, reject, undo, place an
//! order, or send a return request: those are person-only endpoints, and the token it carries
//! (the gateway's on-behalf token, passed through unchanged) has no `human:*` scope anyway.
//!
//! For the evaluation only, a turn with `tool_set: facade` uses the screen-shaped facade the
//! same way (`?set=facade` on the manifest and the tool endpoint); commerce serves it in demo
//! mode only.

use acom_contract::{ErrorCode, Preview, ToolError};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::tools::{Manifest, ToolSpec};

/// `POST /v1/platform/change-sets`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewChangeSet {
    pub title: String,
    pub request_text: Vec<String>,
    pub interpretation: Vec<Value>,
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// One item of `PUT /v1/platform/change-sets/{id}/items`. Without `depends_on`, commerce derives
/// it from the `$item[n].slot` placeholders in the item's input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemRef {
    pub preview_id: String,
}

/// What the drafting endpoints answer.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChangeSetHead {
    pub id: String,
    /// The number people see (`#1042`); only `create` returns it.
    #[serde(default)]
    pub number: Option<i64>,
    pub version: i32,
    pub state: String,
}

/// The commerce service as the runtime uses it.
pub trait ToolClient: Send + Sync {
    /// `GET /v1/tools`: the tools this caller may use.
    fn manifest(&self) -> BoxFuture<'_, Result<Vec<ToolSpec>, ToolError>>;

    /// A read tool.
    fn read<'a>(
        &'a self,
        tool: &'a str,
        input: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>>;

    /// A write's preview, optionally put straight into a change set.
    fn preview<'a>(
        &'a self,
        tool: &'a str,
        input: &'a Value,
        change_set_id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Preview, ToolError>>;

    /// Runs a preview that needs no approval (a shopper's cart, a remembered preference).
    /// Returns the whole result: `{ok, id, undo}` plus what the tool adds (`memory`).
    fn commit<'a>(
        &'a self,
        tool: &'a str,
        preview_id: &'a str,
    ) -> BoxFuture<'a, Result<Value, ToolError>>;

    fn create_change_set<'a>(
        &'a self,
        new: &'a NewChangeSet,
    ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>>;

    /// Replaces a change set's items (a correction made by talking): version + 1.
    fn replace_items<'a>(
        &'a self,
        change_set_id: &'a str,
        items: &'a [ItemRef],
    ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>>;

    /// Hands a drafted set to people: `waiting_for_approval`. Never approves.
    fn submit<'a>(
        &'a self,
        change_set_id: &'a str,
        version: i32,
    ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>>;

    /// `PATCH /v1/platform/change-sets/{id}`: a correction made by talking joins the set's
    /// request text.
    fn append_request<'a>(
        &'a self,
        change_set_id: &'a str,
        sentence: &'a str,
    ) -> BoxFuture<'a, Result<(), ToolError>> {
        let _ = (change_set_id, sentence);
        Box::pin(async { Ok(()) })
    }

    /// `GET /v1/platform/store`: the store's currency, time zone and current time (the demo
    /// clock in demo mode), as commerce sees them. The agent never reads commerce's tables.
    fn store(&self) -> BoxFuture<'_, Result<crate::store::StoreSettings, ToolError>> {
        Box::pin(async {
            Err(ToolError::new(
                ErrorCode::UPSTREAM_UNAVAILABLE,
                "This client cannot ask for the store's settings.",
            ))
        })
    }

    /// `GET /v1/tools?set=facade`: the evaluation's screen-shaped facade (demo mode only).
    fn facade_manifest(&self) -> BoxFuture<'_, Result<Vec<ToolSpec>, ToolError>> {
        Box::pin(async { Err(no_facade()) })
    }

    /// `POST /v1/tools/{tool}?set=facade`: one facade call, read or write alike, run at once.
    fn facade_call<'a>(
        &'a self,
        tool: &'a str,
        input: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        let _ = (tool, input);
        Box::pin(async { Err(no_facade()) })
    }
}

fn no_facade() -> ToolError {
    ToolError::new(
        ErrorCode::UPSTREAM_UNAVAILABLE,
        "This client cannot reach the screen-shaped facade.",
    )
}

/// The HTTP client for one turn: the commerce base URL, the caller's token, the request id.
#[derive(Debug, Clone)]
pub struct HttpToolClient {
    base: String,
    token: String,
    request_id: String,
    http: reqwest::Client,
}

impl HttpToolClient {
    pub fn new(http: reqwest::Client, base: &str, token: &str, request_id: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
            request_id: request_id.to_owned(),
            http,
        }
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, ToolError> {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .header("x-request-id", &self.request_id);
        if let Some(b) = body {
            req = req
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(b.to_string());
        }
        let res = req.send().await.map_err(|e| unavailable(&e))?;
        let status = res.status();
        let bytes = res.bytes().await.map_err(|e| unavailable(&e))?;
        let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(v);
        }
        Err(error_from(status.as_u16(), &v))
    }

    async fn tool(&self, tool: &str, body: Value) -> Result<Value, ToolError> {
        self.send(
            reqwest::Method::POST,
            &format!("/v1/tools/{tool}"),
            Some(body),
        )
        .await
    }
}

fn unavailable(e: &reqwest::Error) -> ToolError {
    ToolError::new(
        ErrorCode::UPSTREAM_UNAVAILABLE,
        "The commerce service did not answer; try again in a moment.",
    )
    .detail(json!({"upstream": "commerce", "retryable": true, "reason": e.to_string()}))
}

/// `{"error": {...}}` as a `ToolError`; anything else as an upstream error with the status.
fn error_from(status: u16, body: &Value) -> ToolError {
    body.get("error")
        .and_then(|e| serde_json::from_value::<ToolError>(e.clone()).ok())
        .unwrap_or_else(|| {
            ToolError::new(
                ErrorCode::UPSTREAM_UNAVAILABLE,
                format!("The commerce service answered {status} without an error body."),
            )
            .detail(json!({"upstream": "commerce", "status": status}))
        })
}

fn decode<T: serde::de::DeserializeOwned>(v: Value, what: &str) -> Result<T, ToolError> {
    serde_json::from_value(v).map_err(|e| {
        ToolError::new(
            ErrorCode::INTERNAL,
            format!("The commerce service sent a {what} the agent cannot read: {e}"),
        )
    })
}

impl ToolClient for HttpToolClient {
    fn manifest(&self) -> BoxFuture<'_, Result<Vec<ToolSpec>, ToolError>> {
        Box::pin(async move {
            let v = self.send(reqwest::Method::GET, "/v1/tools", None).await?;
            Ok(decode::<Manifest>(v, "tool manifest")?.items)
        })
    }

    fn read<'a>(
        &'a self,
        tool: &'a str,
        input: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        Box::pin(self.tool(tool, json!({"input": input})))
    }

    fn preview<'a>(
        &'a self,
        tool: &'a str,
        input: &'a Value,
        change_set_id: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Preview, ToolError>> {
        Box::pin(async move {
            let mut body = json!({"input": input});
            if let Some(cs) = change_set_id {
                body["change_set_id"] = json!(cs);
            }
            decode(self.tool(tool, body).await?, "preview")
        })
    }

    fn commit<'a>(
        &'a self,
        tool: &'a str,
        preview_id: &'a str,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        Box::pin(self.tool(tool, json!({"preview_id": preview_id})))
    }

    fn create_change_set<'a>(
        &'a self,
        new: &'a NewChangeSet,
    ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>> {
        Box::pin(async move {
            let body = serde_json::to_value(new).unwrap_or(Value::Null);
            let v = self
                .send(
                    reqwest::Method::POST,
                    "/v1/platform/change-sets",
                    Some(body),
                )
                .await?;
            decode(v, "change set")
        })
    }

    fn replace_items<'a>(
        &'a self,
        change_set_id: &'a str,
        items: &'a [ItemRef],
    ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>> {
        Box::pin(async move {
            let v = self
                .send(
                    reqwest::Method::PUT,
                    &format!("/v1/platform/change-sets/{change_set_id}/items"),
                    Some(json!({"items": items})),
                )
                .await?;
            decode(v, "change set")
        })
    }

    fn submit<'a>(
        &'a self,
        change_set_id: &'a str,
        version: i32,
    ) -> BoxFuture<'a, Result<ChangeSetHead, ToolError>> {
        Box::pin(async move {
            let v = self
                .send(
                    reqwest::Method::POST,
                    &format!("/v1/platform/change-sets/{change_set_id}/submit"),
                    Some(json!({"version": version})),
                )
                .await?;
            decode(v, "change set")
        })
    }

    fn append_request<'a>(
        &'a self,
        change_set_id: &'a str,
        sentence: &'a str,
    ) -> BoxFuture<'a, Result<(), ToolError>> {
        Box::pin(async move {
            self.send(
                reqwest::Method::PATCH,
                &format!("/v1/platform/change-sets/{change_set_id}"),
                Some(json!({"append_request_text": sentence})),
            )
            .await
            .map(|_| ())
        })
    }

    fn facade_manifest(&self) -> BoxFuture<'_, Result<Vec<ToolSpec>, ToolError>> {
        Box::pin(async move {
            let v = self
                .send(reqwest::Method::GET, "/v1/tools?set=facade", None)
                .await?;
            Ok(decode::<Manifest>(v, "facade manifest")?.items)
        })
    }

    fn facade_call<'a>(
        &'a self,
        tool: &'a str,
        input: &'a Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        Box::pin(async move {
            let path = format!("/v1/tools/{tool}?set=facade");
            self.send(reqwest::Method::POST, &path, Some(json!({"input": input})))
                .await
        })
    }

    fn store(&self) -> BoxFuture<'_, Result<crate::store::StoreSettings, ToolError>> {
        Box::pin(async move {
            let v = self
                .send(reqwest::Method::GET, "/v1/platform/store", None)
                .await?;
            let unreadable = || {
                ToolError::new(
                    ErrorCode::INTERNAL,
                    "The commerce service sent store settings the agent cannot read.",
                )
            };
            let zone: chrono_tz::Tz = v["time_zone"]
                .as_str()
                .and_then(|z| z.parse().ok())
                .ok_or_else(unreadable)?;
            let now = v["now"]
                .as_str()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .ok_or_else(unreadable)?
                .with_timezone(&chrono::Utc);
            Ok(crate::store::StoreSettings {
                time: acom_core::StoreTime::new(zone),
                currency: v["currency"].as_str().unwrap_or("").to_owned(),
                now,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commerce_errors_keep_their_code_and_next_tool() {
        let e = error_from(
            422,
            &json!({"error": {"code": "below_price_floor", "class": "policy_violation",
                "message": "Under cost + 15%.", "next_tool": "set_sale_prices",
                "detail": {"rule": "price_floor"}}}),
        );
        assert_eq!(e.code, "below_price_floor");
        assert_eq!(e.next_tool.as_deref(), Some("set_sale_prices"));
        let e = error_from(502, &Value::Null);
        assert_eq!(e.code, "upstream_unavailable");
    }

    #[test]
    fn previews_and_heads_decode_from_the_wire_shape() {
        let p: Preview = decode(
            json!({"preview_id": "pv_1", "expires_at": null, "change_set_id": "cs_1042",
                   "changes": [{"op": "update", "entity": "unit_price_specification", "id": "",
                                "to": {"product_id": "prd_windbreaker", "percent_off": 20}}],
                   "checks": [{"rule": "max_discount", "passed": true, "detail": null, "fix_tool": null}],
                   "needs_approval": true, "approval_reason": "Changes 40 prices"}),
            "preview",
        )
        .unwrap();
        assert_eq!(p.change_set_id.as_deref(), Some("cs_1042"));
        let h: ChangeSetHead = decode(
            json!({"id": "cs_1042", "version": 2, "state": "drafting", "superseded": ["pv_1"]}),
            "change set",
        )
        .unwrap();
        assert_eq!(h.number, None);
    }
}
