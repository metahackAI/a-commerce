//! The MCP view of the registry for outside agents.
//!
//! Exposes only operations allowed by the caller's `external:*` scopes, in their wire form
//! (with `preview` and `preview_id`). Placing an order stays a person's action.
