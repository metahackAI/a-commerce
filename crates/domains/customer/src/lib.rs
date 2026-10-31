//! The `customer` domain: members, tier programs, addresses, consent, preferences, inquiries, remedies.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/customer.md`.
//! Layout follows the API hierarchy (docs/backend/README.md §2.2): one folder per resource
//! (`person`, `membership`, `address`, `preference`, `wishlist`), one file per operation named
//! after its tool, each operation one type implementing `ReadOp`/`WriteOp`/`PersonOp`. A
//! resource's `mod.rs` holds what its operations share; `repo` holds the domain's SQL, and
//! `wording` and `progress` are pure rules with unit tests. Other domains are reached only
//! through `service::providers`, which they fill in at start-up.

pub mod service;

mod access;
mod address;
mod membership;
mod person;
mod preference;
mod progress;
mod repo;
#[cfg(test)]
mod testdb;
mod wishlist;
mod wording;

use acom_contract::{ErrorClass, ErrorCode, RegistryBuilder};
use acom_store::Cx;

/// A guest asked for something only a signed-in shopper has.
pub const NOT_SIGNED_IN: ErrorCode = ErrorCode::new("not_signed_in", ErrorClass::Unauthenticated);
pub const CUSTOMER_NOT_FOUND: ErrorCode =
    ErrorCode::new("customer_not_found", ErrorClass::NotFound);
pub const TIER_NOT_FOUND: ErrorCode = ErrorCode::new("tier_not_found", ErrorClass::NotFound);
pub const PROGRAM_NOT_FOUND: ErrorCode = ErrorCode::new("program_not_found", ErrorClass::NotFound);
pub const ADDRESS_NOT_FOUND: ErrorCode = ErrorCode::new("address_not_found", ErrorClass::NotFound);
pub const MEMORY_NOT_FOUND: ErrorCode = ErrorCode::new("memory_not_found", ErrorClass::NotFound);
pub const PRODUCT_NOT_FOUND: ErrorCode = ErrorCode::new("product_not_found", ErrorClass::NotFound);
/// Unmasking personal data needs a reason for the action log.
pub const ACCESS_REASON_REQUIRED: ErrorCode =
    ErrorCode::new("access_reason_required", ErrorClass::InvalidInput);
pub const UNMASK_FORBIDDEN: ErrorCode = ErrorCode::new("unmask_forbidden", ErrorClass::Forbidden);
pub const INVALID_FILTER: ErrorCode = ErrorCode::new("invalid_filter", ErrorClass::InvalidInput);
pub const ADDRESS_LIMIT_EXCEEDED: ErrorCode =
    ErrorCode::new("address_limit_exceeded", ErrorClass::LimitExceeded);
pub const INVALID_POSTAL_CODE: ErrorCode =
    ErrorCode::new("invalid_postal_code", ErrorClass::InvalidInput);
/// The shopper said something about another person (a gift recipient); it is not remembered.
pub const GIFT_RECIPIENT_DATA: ErrorCode =
    ErrorCode::new("gift_recipient_data", ErrorClass::Domain);
pub const SENSITIVE_DATA: ErrorCode = ErrorCode::new("sensitive_data", ErrorClass::Domain);
/// Every memory to add is already there, or there is nothing to remove.
pub const NOTHING_TO_CHANGE: ErrorCode = ErrorCode::new("nothing_to_change", ErrorClass::Domain);

/// Adds every customer operation to the registry.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    b.read(person::GetMyProfile)
        .read(person::GetReturnHistory)
        .read(person::SearchMembers)
        .read(person::GetMember)
        .read(membership::ListTiers)
        .read(membership::GetTier)
        .read(membership::GetMemberTier)
        .read(address::ListAddresses)
        .write(address::AddAddress)
        .write(address::UpdateAddress)
        .write(address::SetPrimaryAddress)
        .read(preference::GetPreferences)
        .write(preference::UpdateMyPreferences)
        .person(preference::UndoMemory)
        .read(wishlist::ListWishlist)
        .write(wishlist::AddToWishlist)
        .write(wishlist::RemoveFromWishlist);
}

#[cfg(test)]
mod tests {
    use acom_contract::{Kind, RegistryBuilder};

    #[test]
    fn every_operation_registers_with_a_closed_schema() {
        let mut b = RegistryBuilder::default();
        super::register(&mut b);
        let registry = b.build().unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(registry.len(), 17);
        for op in registry.iter() {
            let m = op.meta();
            assert_eq!(m.domain(), "customer", "{}", m.tool);
            assert!(m.path.starts_with("/v1/customer/"), "{}", m.tool);
            if m.kind == Kind::Person {
                assert_eq!(m.person_scope, "human:undo");
            } else {
                assert!(m.person_scope.is_empty(), "{}", m.tool);
            }
        }
    }
}
