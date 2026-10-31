//! The `community` domain: shoppers' boards, posts and replies, and their moderation.
//! Specified in `docs/api/community.md`; rules in `policies/community.yml`.
//!
//! Layout (docs/backend/README.md §2.2): one folder per resource of the spec (`board`, `post`,
//! `reply`), one file per operation named after its tool.

use acom_contract::RegistryBuilder;
use acom_store::Cx;

/// Adds every operation of this domain to the registry.
pub fn register(_b: &mut RegistryBuilder<Cx>) {}
