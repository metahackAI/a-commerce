//! The `sales` domain: cart, order preparation, orders, payments, receipts, subscriptions, gift orders.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/sales.md`.
//! Layout: `model` (entities and state machines), `ops` (one type per operation implementing
//! `ReadOp`/`WriteOp`), `repo` (SQL). Other domains are reached only through the service
//! traits they export, never through their tables.
