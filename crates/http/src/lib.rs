//! The REST view of the registry.
//!
//! Builds axum routes from operation metadata (reads as GET, writes with `?preview=true` or a
//! `preview_id`), serves the tool manifest at `GET /v1/tools`, and turns `ToolError` into
//! `{"error": {...}}` with the status from the error catalog (`docs/api/errors.md`).
