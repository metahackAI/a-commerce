//! An OpenAI-compatible chat completions client: the self-hosted vLLM (Muse Glimmer) and the
//! Meta Model API (Muse Spark) speak the same protocol.
//!
//! Request building, response parsing, and stream reading are plain functions so they are
//! tested without a network.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::chat::{
    ChatModel, ChatRequest, ChatResponse, DeltaSink, LlmError, Message, ModelInfo, Role,
    StreamVariant, ToolCall, Usage,
};

/// How a provider is asked for JSON that follows a schema.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutput {
    /// `response_format: {"type": "json_schema", ...}` (OpenAI, vLLM, most compatible APIs).
    #[default]
    JsonSchema,
    /// vLLM's `guided_json` request field.
    GuidedJson,
    /// `response_format: {"type": "json_object"}` and the schema in the system message.
    JsonObject,
    /// Only the schema in the system message; the answer is parsed from text.
    Prompt,
}

/// Calls in a row that may fail with the endpoint down before it is left alone.
const TRIP_AFTER: u32 = 2;
/// How long an endpoint that keeps failing is left alone before one call tries it again.
const REST: Duration = Duration::from_secs(45);

/// Stops waiting on an endpoint that is down: after `TRIP_AFTER` calls in a row that timed out,
/// could not connect or got a 5xx, calls fail at once for `REST`; then one call tries again, and
/// an answer closes it.
#[derive(Debug, Default)]
struct Breaker {
    failures: u32,
    resting_until: Option<Instant>,
}

impl Breaker {
    /// How long calls still wait before trying again, if the endpoint is resting.
    fn resting(&mut self, now: Instant) -> Option<Duration> {
        let until = self.resting_until?;
        if now < until {
            return Some(until - now);
        }
        // Rest is over: let this call through, and trip again at once if it fails.
        self.resting_until = None;
        self.failures = TRIP_AFTER - 1;
        None
    }

    fn record(&mut self, down: bool, now: Instant) {
        if !down {
            self.failures = 0;
            return;
        }
        self.failures += 1;
        if self.failures >= TRIP_AFTER {
            self.resting_until = Some(now + REST);
        }
    }
}

/// One OpenAI-compatible endpoint and model.
#[derive(Debug)]
pub struct OpenAiCompatible {
    info: ModelInfo,
    url: String,
    api_key: Option<String>,
    structured: StructuredOutput,
    timeout: Duration,
    http: reqwest::Client,
    breaker: Mutex<Breaker>,
}

impl OpenAiCompatible {
    pub fn new(
        info: ModelInfo,
        base_url: &str,
        api_key: Option<String>,
        structured: StructuredOutput,
        timeout: Duration,
    ) -> Result<Self, LlmError> {
        let http = reqwest::Client::builder()
            .build()
            .map_err(|e| LlmError::NotConfigured(format!("cannot build the HTTP client: {e}")))?;
        Ok(Self {
            info,
            url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
            api_key,
            structured,
            timeout,
            http,
            breaker: Mutex::new(Breaker::default()),
        })
    }

    /// The JSON body for one request.
    pub fn body(&self, req: &ChatRequest, stream: bool) -> Value {
        request_body(&self.info.model, self.structured, req, stream)
    }

    async fn send(
        &self,
        req: &ChatRequest,
        on_delta: Option<DeltaSink<'_>>,
    ) -> Result<ChatResponse, LlmError> {
        let resting = self
            .breaker
            .lock()
            .map_or(None, |mut b| b.resting(Instant::now()));
        if let Some(wait) = resting {
            return Err(LlmError::Resting(wait.as_secs().max(1)));
        }
        let res = self.call(req, on_delta).await;
        // Only the endpoint's own failures count; an answer that could not be used means it is up.
        let down = res.as_ref().err().is_some_and(LlmError::is_down);
        if let Ok(mut b) = self.breaker.lock() {
            b.record(down, Instant::now());
        }
        res
    }

    async fn call(
        &self,
        req: &ChatRequest,
        on_delta: Option<DeltaSink<'_>>,
    ) -> Result<ChatResponse, LlmError> {
        let started = Instant::now();
        let left = |started: Instant| self.timeout.saturating_sub(started.elapsed());
        let mut call = self
            .http
            .post(&self.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(self.body(req, on_delta.is_some()).to_string());
        if let Some(key) = &self.api_key {
            call = call.bearer_auth(key);
        }
        let secs = self.timeout.as_secs();
        let mut res = tokio::time::timeout(left(started), call.send())
            .await
            .map_err(|_| LlmError::Timeout(secs))?
            .map_err(|e| LlmError::Transport(e.without_url().to_string()))?;
        let status = res.status();
        if !status.is_success() {
            let text = tokio::time::timeout(left(started), res.text())
                .await
                .map_err(|_| LlmError::Timeout(secs))?
                .unwrap_or_default();
            return Err(LlmError::Http {
                status: status.as_u16(),
                message: error_message(&text),
            });
        }
        let Some(sink) = on_delta else {
            let bytes = tokio::time::timeout(left(started), res.bytes())
                .await
                .map_err(|_| LlmError::Timeout(secs))?
                .map_err(|e| LlmError::Transport(e.without_url().to_string()))?;
            let v: Value =
                serde_json::from_slice(&bytes).map_err(|e| LlmError::BadResponse(e.to_string()))?;
            return parse_completion(&v);
        };
        let mut stream = StreamReader::default();
        loop {
            let chunk = tokio::time::timeout(left(started), res.chunk())
                .await
                .map_err(|_| LlmError::Timeout(secs))?
                .map_err(|e| LlmError::Transport(e.without_url().to_string()))?;
            let Some(chunk) = chunk else { break };
            for delta in stream.feed(&chunk) {
                sink(&delta);
            }
        }
        stream.finish()
    }
}

impl ChatModel for OpenAiCompatible {
    fn info(&self) -> &ModelInfo {
        &self.info
    }

    fn chat<'a>(
        &'a self,
        request: &'a ChatRequest,
        on_delta: Option<DeltaSink<'a>>,
    ) -> BoxFuture<'a, Result<ChatResponse, LlmError>> {
        Box::pin(self.send(request, on_delta))
    }
}

/// The provider's error sentence, or the start of the body.
fn error_message(body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    parsed
        .as_ref()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("message"))
                .or_else(|| v.get("detail"))
        })
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| body.chars().take(200).collect())
}

fn role(r: Role) -> &'static str {
    match r {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn message_json(m: &Message) -> Value {
    let mut v = json!({"role": role(m.role), "content": m.content});
    if !m.tool_calls.is_empty() {
        v["tool_calls"] = m
            .tool_calls
            .iter()
            .map(|c| {
                json!({"id": c.id, "type": "function",
                       "function": {"name": c.name, "arguments": c.arguments.to_string()}})
            })
            .collect();
    }
    if let Some(id) = &m.tool_call_id {
        v["tool_call_id"] = json!(id);
    }
    v
}

/// The chat completions body for `req`.
pub fn request_body(
    model: &str,
    structured: StructuredOutput,
    req: &ChatRequest,
    stream: bool,
) -> Value {
    let mut messages: Vec<Value> = req.messages.iter().map(message_json).collect();
    let structured = if stream && req.stream_variant == StreamVariant::GuidedJson {
        StructuredOutput::GuidedJson
    } else {
        structured
    };
    let mut body = json!({"model": model, "temperature": req.temperature});
    if let Some(n) = req.max_tokens {
        body["max_tokens"] = json!(n);
    }
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| {
                json!({"type": "function", "function": {
                    "name": t.name, "description": t.description, "parameters": t.parameters}})
            })
            .collect();
        body["tool_choice"] = json!("auto");
    }
    // The streamed ways a provider may need (`StreamVariant`), for streamed calls only; plain
    // text is also how a call is asked again without its schema when the server failed on it.
    let variant = if stream || req.stream_variant == StreamVariant::PlainText {
        req.stream_variant
    } else {
        StreamVariant::Configured
    };
    if matches!(
        variant,
        StreamVariant::WithoutThinking | StreamVariant::PlainText
    ) {
        body["chat_template_kwargs"] = json!({"enable_thinking": false});
    }
    if let (StreamVariant::PlainText, Some(out)) = (variant, &req.output) {
        // The schema's one text field is the answer: ask for that text alone.
        let field = out
            .schema
            .pointer("/properties")
            .and_then(Value::as_object)
            .and_then(|p| p.keys().next().cloned())
            .unwrap_or_else(|| "text".into());
        messages.push(json!({"role": "system", "content": format!(
            "Write only the {field} itself as plain sentences: no JSON, no quotes around it, no heading.")}));
    } else if let (StreamVariant::StructuredOutputs, Some(out)) = (variant, &req.output) {
        body["structured_outputs"] = json!({"json": out.schema});
    } else if let Some(out) = &req.output {
        match structured {
            StructuredOutput::JsonSchema => {
                body["response_format"] = json!({"type": "json_schema",
                    "json_schema": {"name": out.name, "schema": out.schema, "strict": false}});
            }
            StructuredOutput::GuidedJson => body["guided_json"] = out.schema.clone(),
            StructuredOutput::JsonObject => {
                body["response_format"] = json!({"type": "json_object"});
            }
            StructuredOutput::Prompt => {}
        }
        if matches!(
            structured,
            StructuredOutput::JsonObject | StructuredOutput::Prompt
        ) {
            messages.push(json!({"role": "system", "content": format!(
                "Answer with one JSON object and nothing else. It must follow this JSON Schema:\n{}",
                out.schema)}));
        }
    }
    body["messages"] = Value::Array(messages);
    if stream {
        body["stream"] = json!(true);
        if req.stream_variant != StreamVariant::WithoutUsage {
            body["stream_options"] = json!({"include_usage": true});
        }
    }
    body
}

fn usage_of(v: &Value) -> Usage {
    let n = |k: &str| {
        v.get(k)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0)
    };
    Usage {
        input_tokens: n("prompt_tokens"),
        output_tokens: n("completion_tokens"),
    }
}

/// Arguments arrive as a JSON string; models sometimes send an object instead.
fn arguments(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) if s.trim().is_empty() => json!({}),
        Some(Value::String(s)) => crate::parse::extract_json(s).unwrap_or_else(|| json!({})),
        Some(o @ Value::Object(_)) => o.clone(),
        _ => json!({}),
    }
}

/// A non-streamed chat completion.
pub fn parse_completion(v: &Value) -> Result<ChatResponse, LlmError> {
    let choice = v
        .pointer("/choices/0")
        .ok_or_else(|| LlmError::BadResponse("no choices in the answer".into()))?;
    let message = choice.get("message").cloned().unwrap_or(Value::Null);
    let tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .enumerate()
                .filter_map(|(n, c)| {
                    let f = c.get("function")?;
                    Some(ToolCall {
                        id: c
                            .get("id")
                            .and_then(Value::as_str)
                            .map_or_else(|| format!("call_{}", n + 1), str::to_owned),
                        name: f.get("name")?.as_str()?.to_owned(),
                        arguments: arguments(f.get("arguments")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ChatResponse {
        model: v
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        text: message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        tool_calls,
        usage: v.get("usage").map(usage_of).unwrap_or_default(),
        finish_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

#[derive(Debug, Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// How much of a streamed body is kept in case it turns out not to be a stream.
const HEAD_BYTES: usize = 1 << 20;

/// Reads a streamed chat completion (server-sent events) chunk by chunk.
#[derive(Debug, Default)]
pub struct StreamReader {
    pending: Vec<u8>,
    /// The start of the body, for a provider that answered `stream: true` with one JSON object.
    head: Vec<u8>,
    /// `data:` lines read: none means the body was not an event stream.
    data_lines: usize,
    /// Text some providers send apart from the answer (`reasoning_content`, `reasoning`).
    reasoning: usize,
    /// The start of that text, to say what it was when no answer came.
    reasoning_head: String,
    /// Whether that text is the answer itself: a provider whose reasoning parser never sees the
    /// end of reasoning sends a schema-held answer (it starts with `{`) in the reasoning field.
    reasoning_is_answer: Option<bool>,
    /// An error the provider sent inside the stream (`data: {"error": …}`).
    error: Option<String>,
    /// The first `data:` line, to say what a stream without an answer carried.
    first: Option<String>,
    model: String,
    text: String,
    calls: BTreeMap<u64, PartialCall>,
    usage: Usage,
    finish_reason: Option<String>,
}

impl StreamReader {
    /// Takes the next bytes; returns the answer text that arrived in them.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        if self.head.len() < HEAD_BYTES {
            let room = HEAD_BYTES - self.head.len();
            self.head.extend_from_slice(&bytes[..bytes.len().min(room)]);
        }
        self.pending.extend_from_slice(bytes);
        let mut deltas = Vec::new();
        while let Some(end) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            if let Some(d) = self.line(line.trim_end_matches(['\r', '\n'])) {
                deltas.push(d);
            }
        }
        deltas
    }

    fn line(&mut self, line: &str) -> Option<String> {
        let data = line.strip_prefix("data:")?.trim_start();
        self.data_lines += 1;
        if data == "[DONE]" {
            return None;
        }
        if self.first.is_none() {
            self.first = Some(data.chars().take(200).collect());
        }
        let v: Value = serde_json::from_str(data).ok()?;
        if self.error.is_none()
            && (v.get("error").is_some()
                || v.get("object").and_then(Value::as_str) == Some("error"))
        {
            let e = v.get("error").unwrap_or(&v);
            self.error = Some(
                e.get("message")
                    .and_then(Value::as_str)
                    .map_or_else(|| e.to_string(), str::to_owned),
            );
        }
        if let Some(m) = v.get("model").and_then(Value::as_str) {
            m.clone_into(&mut self.model);
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            self.usage = usage_of(u);
        }
        let choice = v.pointer("/choices/0")?;
        if let Some(r) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(r.to_owned());
        }
        let delta = choice.get("delta")?;
        for c in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let index = c.get("index").and_then(Value::as_u64).unwrap_or(0);
            let call = self.calls.entry(index).or_default();
            if let Some(id) = c.get("id").and_then(Value::as_str) {
                id.clone_into(&mut call.id);
            }
            if let Some(f) = c.get("function") {
                if let Some(n) = f.get("name").and_then(Value::as_str) {
                    call.name.push_str(n);
                }
                if let Some(a) = f.get("arguments").and_then(Value::as_str) {
                    call.arguments.push_str(a);
                }
            }
        }
        let mut carried = None;
        for key in ["reasoning_content", "reasoning"] {
            if let Some(r) = delta.get(key).and_then(Value::as_str) {
                self.reasoning += r.chars().count();
                if self.reasoning_head.chars().count() < 160 {
                    self.reasoning_head.push_str(r);
                }
                // Decided by its first visible character, and only while no answer came the
                // usual way: JSON is the answer, anything else is thinking and stays out.
                if self.reasoning_is_answer.is_none()
                    && self.text.is_empty()
                    && let Some(c) = r.trim_start().chars().next()
                {
                    self.reasoning_is_answer = Some(c == '{');
                }
                if self.reasoning_is_answer == Some(true) && !r.is_empty() {
                    self.text.push_str(r);
                    carried = Some(r.to_owned());
                }
            }
        }
        if carried.is_some() {
            return carried;
        }
        let text = delta.get("content").and_then(Value::as_str)?;
        if text.is_empty() {
            return None;
        }
        self.text.push_str(text);
        Some(text.to_owned())
    }

    /// The whole answer once the stream ended. A body that was one JSON object (a provider
    /// that does not stream) is read as a plain completion; a stream that carried no answer, only
    /// reasoning, is an error that says so.
    pub fn finish(mut self) -> Result<ChatResponse, LlmError> {
        if !self.pending.is_empty() {
            let rest = String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned();
            self.line(rest.trim());
        }
        if self.data_lines == 0 {
            let v: Value = serde_json::from_slice(&self.head).map_err(|e| {
                LlmError::BadResponse(format!("neither an event stream nor JSON: {e}"))
            })?;
            return parse_completion(&v);
        }
        if let Some(e) = self.error.take() {
            return Err(LlmError::BadResponse(format!(
                "the stream ended with an error: {e}"
            )));
        }
        if self.text.trim().is_empty() && self.calls.is_empty() {
            if self.reasoning > 0 {
                let head: String = self.reasoning_head.chars().take(160).collect();
                return Err(LlmError::BadResponse(format!(
                    "the stream carried no answer text, only {} characters of reasoning: {head}",
                    self.reasoning
                )));
            }
            return Err(LlmError::BadResponse(format!(
                "the stream carried no answer ({} data lines; the first: {})",
                self.data_lines,
                self.first.as_deref().unwrap_or("none")
            )));
        }
        let tool_calls = self
            .calls
            .into_values()
            .enumerate()
            .filter(|(_, c)| !c.name.is_empty())
            .map(|(n, c)| ToolCall {
                id: if c.id.is_empty() {
                    format!("call_{}", n + 1)
                } else {
                    c.id
                },
                name: c.name,
                arguments: arguments(Some(&Value::String(c.arguments))),
            })
            .collect();
        Ok(ChatResponse {
            model: self.model,
            text: self.text,
            tool_calls,
            usage: self.usage,
            finish_reason: self.finish_reason,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_endpoint_rests_then_gets_one_try() {
        let t0 = Instant::now();
        let mut b = Breaker::default();
        b.record(true, t0);
        assert_eq!(b.resting(t0), None, "one failure is not enough");
        b.record(true, t0);
        assert_eq!(
            b.resting(t0 + Duration::from_secs(5)),
            Some(REST - Duration::from_secs(5))
        );
        // After the rest one call goes through; if it fails, the endpoint rests again at once.
        let later = t0 + REST + Duration::from_secs(1);
        assert_eq!(b.resting(later), None);
        b.record(true, later);
        assert!(b.resting(later).is_some());
        // An answer closes it.
        let after = later + REST + Duration::from_secs(1);
        assert_eq!(b.resting(after), None);
        b.record(false, after);
        b.record(true, after);
        assert_eq!(b.resting(after), None);
    }

    #[test]
    fn only_the_endpoints_own_failures_count_as_down() {
        assert!(LlmError::Timeout(30).is_down());
        assert!(LlmError::Transport("refused".into()).is_down());
        assert!(
            LlmError::Http {
                status: 503,
                message: String::new()
            }
            .is_down()
        );
        assert!(
            !LlmError::Http {
                status: 400,
                message: String::new()
            }
            .is_down()
        );
        assert!(!LlmError::NoJson("prose".into()).is_down());
    }
    use crate::chat::ToolDef;

    fn plan_request() -> ChatRequest {
        ChatRequest::new(
            "plan",
            vec![Message::system("Plan."), Message::user("Run a sale.")],
        )
        .json("plan", json!({"type": "object", "properties": {}}))
        .max_tokens(800)
    }

    #[test]
    fn bodies_ask_for_json_the_way_the_provider_understands() {
        let b = request_body("m", StructuredOutput::JsonSchema, &plan_request(), false);
        assert_eq!(b["model"], "m");
        assert_eq!(b["response_format"]["type"], "json_schema");
        assert_eq!(b["response_format"]["json_schema"]["name"], "plan");
        assert_eq!(b["max_tokens"], 800);
        assert!(b.get("stream").is_none());
        assert_eq!(b["messages"].as_array().unwrap().len(), 2);

        let b = request_body("m", StructuredOutput::GuidedJson, &plan_request(), true);
        assert_eq!(b["guided_json"]["type"], "object");
        assert_eq!(b["stream"], true);
        assert_eq!(b["stream_options"]["include_usage"], true);

        // The streamed variants: vLLM's own field, or no usage option.
        let guided = plan_request().stream_variant(StreamVariant::GuidedJson);
        let b = request_body("m", StructuredOutput::JsonSchema, &guided, true);
        assert!(b.get("response_format").is_none());
        assert_eq!(b["guided_json"]["type"], "object");
        let b = request_body("m", StructuredOutput::JsonSchema, &guided, false);
        assert_eq!(
            b["response_format"]["type"], "json_schema",
            "only a stream changes the field"
        );
        let current = plan_request().stream_variant(StreamVariant::StructuredOutputs);
        let b = request_body("m", StructuredOutput::JsonSchema, &current, true);
        assert!(b.get("response_format").is_none());
        assert_eq!(b["structured_outputs"]["json"]["type"], "object");
        let plain = plan_request().stream_variant(StreamVariant::PlainText);
        let b = request_body("m", StructuredOutput::JsonSchema, &plain, true);
        assert!(b.get("response_format").is_none() && b.get("structured_outputs").is_none());
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        let last = b["messages"].as_array().unwrap().last().unwrap();
        assert!(
            last["content"]
                .as_str()
                .unwrap()
                .contains("plain sentences")
        );
        let b = request_body("m", StructuredOutput::JsonSchema, &plain, false);
        assert!(
            b.get("response_format").is_none(),
            "plain text holds without a stream too"
        );
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        let calm = plan_request().stream_variant(StreamVariant::WithoutThinking);
        let b = request_body("m", StructuredOutput::JsonSchema, &calm, true);
        assert_eq!(b["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(b["response_format"]["type"], "json_schema");
        let b = request_body("m", StructuredOutput::JsonSchema, &calm, false);
        assert!(
            b.get("chat_template_kwargs").is_none(),
            "only a stream changes the body"
        );
        let bare = plan_request().stream_variant(StreamVariant::WithoutUsage);
        let b = request_body("m", StructuredOutput::JsonSchema, &bare, true);
        assert_eq!(b["stream"], true);
        assert!(b.get("stream_options").is_none());

        let b = request_body("m", StructuredOutput::Prompt, &plan_request(), false);
        assert!(b.get("response_format").is_none());
        let last = b["messages"].as_array().unwrap().last().unwrap();
        assert!(last["content"].as_str().unwrap().contains("JSON Schema"));

        let mut r = plan_request();
        r.tools.push(ToolDef {
            name: "list_tiers".into(),
            description: "Member tiers.".into(),
            parameters: json!({"type": "object"}),
        });
        r.messages.push(Message {
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "list_tiers".into(),
                arguments: json!({}),
            }],
            ..Message::assistant("")
        });
        r.messages
            .push(Message::tool_result("c1", "{\"total\": 3}"));
        let b = request_body("m", StructuredOutput::JsonSchema, &r, false);
        assert_eq!(b["tools"][0]["function"]["name"], "list_tiers");
        assert_eq!(
            b["messages"][2]["tool_calls"][0]["function"]["arguments"],
            "{}"
        );
        assert_eq!(b["messages"][3]["tool_call_id"], "c1");
    }

    #[test]
    fn reads_a_completion_with_native_tool_calls() {
        let v = json!({
            "model": "muse-glimmer-30b",
            "choices": [{"finish_reason": "tool_calls", "message": {"role": "assistant", "content": null,
                "tool_calls": [{"id": "call_9", "type": "function",
                    "function": {"name": "search_products", "arguments": "{\"query\": \"light jacket\"}"}}]}}],
            "usage": {"prompt_tokens": 812, "completion_tokens": 37}
        });
        let r = parse_completion(&v).unwrap();
        assert_eq!(r.model, "muse-glimmer-30b");
        assert_eq!(r.tool_calls[0].name, "search_products");
        assert_eq!(r.tool_calls[0].arguments["query"], "light jacket");
        assert_eq!(
            r.usage,
            Usage {
                input_tokens: 812,
                output_tokens: 37
            }
        );
        assert!(parse_completion(&json!({"choices": []})).is_err());
    }

    #[test]
    fn reads_a_stream_split_anywhere() {
        let events = concat!(
            "data: {\"model\":\"m\",\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"I drafted \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"change set #1042.\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"list_tiers\",\"arguments\":\"{\\\"li\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"mit\\\": 3}\"}}]},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":6}}\n\n",
            "data: [DONE]\n\n"
        );
        let mut reader = StreamReader::default();
        let mut deltas = Vec::new();
        for chunk in events.as_bytes().chunks(7) {
            deltas.extend(reader.feed(chunk));
        }
        assert_eq!(deltas.concat(), "I drafted change set #1042.");
        let r = reader.finish().unwrap();
        assert_eq!(r.text, "I drafted change set #1042.");
        assert_eq!(r.tool_calls[0].arguments, json!({"limit": 3}));
        assert_eq!(r.usage.output_tokens, 6);
        assert_eq!(r.finish_reason.as_deref(), Some("stop"));
        assert_eq!(r.model, "m");
    }

    #[test]
    fn a_provider_that_does_not_stream_is_read_as_one_answer() {
        let body = r#"{"model":"m","choices":[{"message":{"content":"{\"text\": \"Hi.\"}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#;
        let mut reader = StreamReader::default();
        for chunk in body.as_bytes().chunks(5) {
            assert!(reader.feed(chunk).is_empty());
        }
        let r = reader.finish().unwrap();
        assert_eq!(r.text, "{\"text\": \"Hi.\"}");
        assert_eq!(r.usage.output_tokens, 4);
    }

    #[test]
    fn a_schema_held_answer_in_the_reasoning_field_is_the_answer() {
        let events = concat!(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"{\\\"text\\\": \\\"Two \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"jackets.\\\"}\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut reader = StreamReader::default();
        let deltas = reader.feed(events.as_bytes());
        assert_eq!(deltas.concat(), "{\"text\": \"Two jackets.\"}");
        assert_eq!(
            reader.finish().unwrap().text,
            "{\"text\": \"Two jackets.\"}"
        );
        // Thinking in the reasoning field is not the answer.
        let thinking = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"Let me see.\"}}]}\n\ndata: [DONE]\n\n";
        let mut reader = StreamReader::default();
        assert!(reader.feed(thinking.as_bytes()).is_empty());
        let e = reader.finish().unwrap_err().to_string();
        assert!(e.contains("reasoning: Let me see."), "{e}");
    }

    #[test]
    fn an_error_inside_the_stream_is_the_error() {
        let events = "data: {\"object\":\"error\",\"message\":\"guided decoding failed\"}\n\ndata: [DONE]\n\n";
        let mut reader = StreamReader::default();
        reader.feed(events.as_bytes());
        let e = reader.finish().unwrap_err().to_string();
        assert!(e.contains("guided decoding failed"), "{e}");
        let empty =
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\ndata: [DONE]\n\n";
        let mut reader = StreamReader::default();
        reader.feed(empty.as_bytes());
        let e = reader.finish().unwrap_err().to_string();
        assert!(e.contains("no answer (2 data lines"), "{e}");
    }

    #[test]
    fn provider_errors_become_sentences() {
        assert_eq!(
            error_message(r#"{"error": {"message": "Invalid API key"}}"#),
            "Invalid API key"
        );
        assert_eq!(error_message("Bad Gateway"), "Bad Gateway");
    }
}
