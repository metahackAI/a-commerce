//! The `catalog` domain: products, variants, offers, inventory, brands, reviews, questions.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/catalog.md`.
//! Layout: `model` (entities and state machines), `ops` (one type per operation implementing
//! `ReadOp`/`WriteOp`), `repo` (SQL). Other domains are reached only through the service
//! traits they export, never through their tables.
