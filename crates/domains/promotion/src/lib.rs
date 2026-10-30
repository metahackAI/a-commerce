//! The `promotion` domain: promotions, member sale prices, coupons, benefits, loyalty, events, segments.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/promotion.md`.
//! Layout: `model` (entities and state machines), `ops` (one type per operation implementing
//! `ReadOp`/`WriteOp`), `repo` (SQL). Other domains are reached only through the service
//! traits they export, never through their tables.

pub mod benefit;
pub mod service;

use acom_contract::RegistryBuilder;
use acom_store::Cx;

/// Adds every operation of this domain to the registry.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    benefit::register(b);
}
