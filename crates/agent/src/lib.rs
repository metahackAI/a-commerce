//! The agent runtime (the harness).
//!
//! Turns one request into tool calls and answers with UI blocks. It knows tools only through
//! the commerce service's manifest and calls them over HTTP with the caller's delegated token;
//! it never approves or runs a change set. Design: `docs/backend/agent.md`.
//!
//! | module | part of the turn (agent.md §) |
//! |---|---|
//! | [`runtime`] | the turn loop, limits, cancellation (§1) |
//! | [`router`], [`profile`] | choosing a specialist agent; agent definitions as data (§2) |
//! | [`toolsearch`], [`tools`] | candidate tools from the manifest (§3) |
//! | [`planner`] | the plan schema, prompt, and validation (§4) |
//! | [`executor`] | slot binding and step results (§5) |
//! | [`checker`] | intent checks and the reply's numbers (§6–§7) |
//! | [`blocks`] | UI blocks built from tool output (§7) |
//! | [`memory`] | conversation context (§9) |
//! | [`client`] | the `ToolClient` seam to commerce |
//! | [`stream`] | SSE events |
//! | [`store`] | the `agent` schema |

pub mod blocks;
pub mod checker;
pub mod client;
pub mod executor;
pub mod memory;
pub mod planner;
pub mod profile;
pub mod router;
pub mod runtime;
pub mod store;
pub mod stream;
pub mod tools;
pub mod toolsearch;

pub use client::{HttpToolClient, ToolClient};
pub use profile::{ChatSurface, Profiles};
pub use runtime::{Prepared, Runtime, TurnRequest};
pub use stream::{Emitter, Event, encode};
