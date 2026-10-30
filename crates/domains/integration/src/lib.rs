//! The `integration` domain: adapters between the core domains and outside channels, carriers
//! and payment providers; bulk imports and inbound events. Business rules stay in the core
//! domains. Specified in `docs/api/integration.md`; rules in `policies/integration.yml`.
//!
//! Layout (docs/backend/README.md §2.2): one folder per resource of the spec (`product_import`,
//! `order`, `claim`, `channel`, `event`), one file per operation named after its tool.

use acom_contract::RegistryBuilder;
use acom_store::Cx;

/// Adds every operation of this domain to the registry.
pub fn register(_b: &mut RegistryBuilder<Cx>) {}
