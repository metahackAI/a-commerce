//! Talking to models.
//!
//! One `ChatModel` trait with an OpenAI-compatible implementation (self-hosted vLLM for Muse
//! Glimmer, the Meta Model API for Muse Spark). Structured output through guided decoding,
//! fallback parsers for tool calls written as text, deterministic argument repair against
//! closed tool schemas, and per-call usage records. Design: `docs/backend/agent.md` §8.

pub mod chat;
pub mod config;
pub mod openai;
pub mod parse;
pub mod repair;
pub mod usage;

pub use chat::{
    ChatModel, ChatRequest, ChatResponse, DeltaSink, LlmError, Message, ModelInfo, OutputSchema,
    Role, ToolCall, ToolDef, Usage,
};
pub use config::{ModelConfig, ModelSet, ModelsFile};
pub use openai::{OpenAiCompatible, StructuredOutput};
pub use repair::{RepairEnv, Repaired, repair};
pub use usage::{Price, UsageRecord};
