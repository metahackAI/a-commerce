//! Talking to models.
//!
//! One `ChatModel` trait with an OpenAI-compatible implementation (self-hosted vLLM for Muse
//! Glimmer, the Meta Model API for Muse Spark). Structured output through guided decoding,
//! fallback parsers for tool calls written as text, deterministic argument repair against
//! closed tool schemas, and per-call usage records. Design: `docs/backend/agent.md` §8.
