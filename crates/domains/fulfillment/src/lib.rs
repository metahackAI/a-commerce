//! The `fulfillment` domain: claims (cancel, return, exchange), inspection, refunds, deliveries, shipping rates.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/fulfillment.md`.
//! Layout: `model` (entities and state machines), `ops` (one type per operation implementing
//! `ReadOp`/`WriteOp`), `repo` (SQL). Other domains are reached only through the service
//! traits they export, never through their tables.
