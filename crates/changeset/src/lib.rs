//! Previews and change sets.
//!
//! Stores every preview with its plan, checks, basis versions, and policy version; runs a
//! write from a `preview_id` only after the approval rules allow it; and owns the change-set
//! state machine (`drafting → waiting_for_approval → running → verifying → live`, plus
//! `rejected`, `failed`, `undone`). Only this crate, inside the commerce service, executes an
//! approved change set. Design: `docs/backend/commerce.md` §2–3.
