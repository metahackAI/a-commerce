//! The `search` domain: product search, compare, autocomplete, lexicon, index.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/search.md`.
//! Layout: `model` (pure logic: query terms and synonyms, comparison values, facets,
//! suggestions), `ops` (one type per operation implementing `ReadOp`), `repo` (full-text SQL),
//! `wiring` (catalog's lookups in the domains above it). Products, prices and stock come from
//! `acom_catalog::service::for_search`, never from catalog tables.

pub mod model;
pub mod service;

mod ops;
mod repo;
mod wiring;

use acom_contract::{ErrorClass, ErrorCode, RegistryBuilder};
use acom_store::Cx;

/// More products than a comparison takes (`compare.max_products`).
pub const TOO_MANY_PRODUCTS: ErrorCode =
    ErrorCode::new("too_many_products", ErrorClass::LimitExceeded);
/// An attribute the store does not compare by (`compare.attributes`).
pub const UNKNOWN_ATTRIBUTE: ErrorCode =
    ErrorCode::new("unknown_attribute", ErrorClass::InvalidInput);
/// A prefix with nothing to complete.
pub const INVALID_PREFIX: ErrorCode = ErrorCode::new("invalid_prefix", ErrorClass::InvalidInput);
/// The text search could not answer.
pub const INDEX_UNAVAILABLE: ErrorCode =
    ErrorCode::new("index_unavailable", ErrorClass::UpstreamUnavailable);

/// Adds every operation of this domain to the registry, and gives catalog its lookups in the
/// domains above it (member prices, categories, tiers, purchases).
pub fn register(b: &mut RegistryBuilder<Cx>) {
    acom_catalog::neighbors::install(wiring::Wiring);
    b.read(ops::products::SearchProducts)
        .read(ops::compare::CompareProducts)
        .read(ops::autocomplete::Autocomplete)
        .read(ops::facets::GetFacets);
}
