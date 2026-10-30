//! The agent runtime (the harness).
//!
//! Turns one request into tool calls and answers with UI blocks. It knows tools only through
//! the commerce service's manifest and calls them over HTTP with the caller's delegated token;
//! it never approves or runs a change set. Design: `docs/backend/agent.md`.
