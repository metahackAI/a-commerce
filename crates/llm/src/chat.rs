//! What a model call sends and gets back, independent of the provider.

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::usage::Price;

/// Who wrote a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// One message of a chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Tool calls an assistant message made (native tool calling).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// The call a tool message answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::new(Role::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::new(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::new(Role::Assistant, content)
    }

    pub fn tool_result(call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            tool_call_id: Some(call_id.into()),
            ..Self::new(Role::Tool, content)
        }
    }
}

/// A tool the model may call natively: its name, one line, and its closed input schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// A tool call, native or parsed from text. `arguments` is always a JSON value (decoded when
/// the provider sent it as a string).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// A JSON Schema the answer must follow (guided decoding).
#[derive(Debug, Clone, PartialEq)]
pub struct OutputSchema {
    /// Short name providers want for the schema, for example `plan`.
    pub name: String,
    pub schema: Value,
}

/// One request to a model.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    /// Why the call is made (`route`, `plan`, `arguments`, `reply`); recorded with the usage.
    pub purpose: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub output: Option<OutputSchema>,
    /// Sampling temperature. Planning and routing run at 0 so the same input gives the same plan.
    pub temperature: f32,
    pub max_tokens: Option<u32>,
}

impl ChatRequest {
    pub fn new(purpose: &str, messages: Vec<Message>) -> Self {
        Self {
            purpose: purpose.to_owned(),
            messages,
            tools: Vec::new(),
            output: None,
            temperature: 0.0,
            max_tokens: None,
        }
    }

    /// Asks for JSON that follows `schema`.
    pub fn json(mut self, name: &str, schema: Value) -> Self {
        self.output = Some(OutputSchema {
            name: name.to_owned(),
            schema,
        });
        self
    }

    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = Some(n);
        self
    }
}

/// Tokens counted by the provider for one call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// What a model answered.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatResponse {
    /// The provider's model id.
    pub model: String,
    pub text: String,
    /// Native tool calls. Use [`ChatResponse::calls`] to include calls written as text.
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub finish_reason: Option<String>,
}

impl ChatResponse {
    /// The JSON object the model answered with: the guided output, a native tool call's
    /// arguments, or the first JSON object in the text (fenced or bare).
    pub fn json(&self) -> Result<Value, LlmError> {
        if let Some(v) = crate::parse::extract_json(&self.text) {
            return Ok(v);
        }
        if let Some(call) = self.tool_calls.first()
            && call.arguments.is_object()
        {
            return Ok(call.arguments.clone());
        }
        Err(LlmError::NoJson(preview(&self.text)))
    }

    /// Tool calls: native ones, or the ones written as text when there are none.
    pub fn calls(&self, known_tools: &[&str]) -> Vec<ToolCall> {
        if self.tool_calls.is_empty() {
            crate::parse::tool_calls_in_text(&self.text, known_tools)
        } else {
            self.tool_calls.clone()
        }
    }
}

fn preview(text: &str) -> String {
    let t: String = text.chars().take(120).collect();
    if t.len() < text.len() {
        format!("{t}…")
    } else {
        t
    }
}

/// Why a model call failed. Every variant says what to do about it.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LlmError {
    #[error("{0}")]
    NotConfigured(String),
    #[error("the model endpoint answered {status}: {message}")]
    Http { status: u16, message: String },
    #[error("the model endpoint could not be reached: {0}")]
    Transport(String),
    #[error("the model did not answer within {0} seconds")]
    Timeout(u64),
    /// The endpoint failed the last calls in a row; it is left alone for a while.
    #[error("the model has not answered lately; the next try is in {0} seconds")]
    Resting(u64),
    #[error("the model's answer could not be read: {0}")]
    BadResponse(String),
    #[error("the model answered without the JSON that was asked for: {0}")]
    NoJson(String),
}

impl LlmError {
    /// Whether trying the same call again (or on another model) may help.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http { status, .. } => *status == 429 || *status >= 500,
            Self::Transport(_)
            | Self::Timeout(_)
            | Self::Resting(_)
            | Self::NoJson(_)
            | Self::BadResponse(_) => true,
            Self::NotConfigured(_) => false,
        }
    }

    /// Whether the endpoint itself is down (unreachable, silent, failing, or resting after that),
    /// as opposed to an answer that could not be used.
    pub fn is_down(&self) -> bool {
        match self {
            Self::Http { status, .. } => *status >= 500,
            Self::Transport(_) | Self::Timeout(_) | Self::Resting(_) => true,
            Self::NotConfigured(_) | Self::NoJson(_) | Self::BadResponse(_) => false,
        }
    }
}

/// Where streamed answer text goes while the model writes it.
pub type DeltaSink<'a> = &'a (dyn Fn(&str) + Send + Sync);

/// Names and prices of one configured model.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelInfo {
    /// The alias agent definitions use (`glimmer`, `spark`).
    pub alias: String,
    /// The provider's model id, recorded in `agent.model_call.model`.
    pub model: String,
    /// What people see in the turn footer ("Muse Glimmer 30B").
    pub display_name: String,
    pub price: Option<Price>,
}

/// One chat model behind any provider. The runtime only talks to this trait, so tests use a
/// scripted fake and providers can change without touching the harness.
pub trait ChatModel: Send + Sync + std::fmt::Debug {
    fn info(&self) -> &ModelInfo;

    /// Sends one request. With `on_delta`, answer text is passed on as it streams in; the
    /// returned response always holds the whole answer.
    fn chat<'a>(
        &'a self,
        request: &'a ChatRequest,
        on_delta: Option<DeltaSink<'a>>,
    ) -> BoxFuture<'a, Result<ChatResponse, LlmError>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_comes_from_text_or_a_native_call() {
        let r = ChatResponse {
            text: "Here you go:\n```json\n{\"agent\": \"promotion\"}\n```".into(),
            ..Default::default()
        };
        assert_eq!(r.json().unwrap(), json!({"agent": "promotion"}));

        let r = ChatResponse {
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "plan".into(),
                arguments: json!({"steps": []}),
            }],
            ..Default::default()
        };
        assert_eq!(r.json().unwrap(), json!({"steps": []}));

        let r = ChatResponse {
            text: "I cannot help with that.".into(),
            ..Default::default()
        };
        assert!(matches!(r.json(), Err(LlmError::NoJson(_))));
    }

    #[test]
    fn only_transient_errors_are_retryable() {
        assert!(
            LlmError::Http {
                status: 503,
                message: String::new()
            }
            .is_retryable()
        );
        assert!(
            !LlmError::Http {
                status: 401,
                message: String::new()
            }
            .is_retryable()
        );
        assert!(!LlmError::NotConfigured("x".into()).is_retryable());
    }
}
