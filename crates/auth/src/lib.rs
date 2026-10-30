//! Who is calling and what they may do.
//!
//! The gateway is the only signer of internal tokens; commerce and agent only verify them.
//! `human:*` scopes are never present on tokens minted for chat turns, so no prompt can make an
//! agent approve, order, return, or undo. Design: `docs/backend/gateway.md`.
