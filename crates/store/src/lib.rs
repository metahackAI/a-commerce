//! Persistence for the commerce service.
//!
//! Owns the request context (`Cx`: store, principal, clock, policy snapshot, pool) and the
//! transaction context (`TxCx`) that the operation traits in `acom-contract` run against.
//! Every transaction sets `app.tenant_id` so row-level security keeps stores apart.
//! Event tables are append-only; cross-domain notices go through the outbox; scheduled work
//! runs from the job table with `FOR UPDATE SKIP LOCKED`. Design: `docs/backend/commerce.md` §5.
