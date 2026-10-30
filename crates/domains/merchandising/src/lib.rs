//! The `merchandising` domain: categories, display slots, banners, popups, curated pages.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/merchandising.md`.
//! Layout: `model` (entities and state machines), `ops` (one type per operation implementing
//! `ReadOp`/`WriteOp`), `repo` (SQL). Other domains are reached only through the service
//! traits they export, never through their tables.
