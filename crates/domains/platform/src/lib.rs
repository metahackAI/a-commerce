//! The `platform` domain: policies, change sets, action log, enumerations, roles, stores, tasks.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/platform.md`.
//! Layout: `model` (entities and state machines), `ops` (one type per operation implementing
//! `ReadOp`/`WriteOp`), `repo` (SQL). Other domains are reached only through the service
//! traits they export, never through their tables.
