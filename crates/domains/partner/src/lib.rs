//! The `partner` domain: sellers that list in the store, their contracts and grades.
//! Specified in `docs/api/partner.md`; rules in `policies/partner.yml`.
//!
//! Layout (docs/backend/README.md §2.2): one folder per resource of the spec (`seller`,
//! `contract`, `grade`, `dashboard`), one file per operation named after its tool.

use acom_contract::RegistryBuilder;
use acom_store::Cx;

/// Adds every operation of this domain to the registry.
pub fn register(_b: &mut RegistryBuilder<Cx>) {}
