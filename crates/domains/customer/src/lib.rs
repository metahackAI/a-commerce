//! The `customer` domain: members, tier programs, addresses, consent, preferences, inquiries, remedies.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/customer.md`.
//! Layout: `model` (entities and state machines), `ops` (one type per operation implementing
//! `ReadOp`/`WriteOp`), `repo` (SQL). Other domains are reached only through the service
//! traits they export, never through their tables.
