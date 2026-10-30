//! The one place that knows every domain.
//!
//! Each domain crate exposes `register`; this crate calls them all and builds the registry
//! the commerce service serves over REST and MCP, and that `cargo xtask codegen` exports.
