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
mod consent;
mod employee;
mod faq_notice;
mod inquiry;
mod membership;
mod password;
mod person;
mod preference;
mod progress;
mod remedy;
mod repo;
mod secret;
#[cfg(test)]
mod testdb;
mod verification;
mod watch;
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

// ── person ──────────────────────────────────────────────────────────────────────────────
/// Sign-up or a contact change without a valid `verification_token`.
pub const IDENTITY_NOT_VERIFIED: ErrorCode =
    ErrorCode::new("identity_not_verified", ErrorClass::Domain);
/// A new contact must be verified again before it is saved.
pub const REVERIFY_REQUIRED: ErrorCode = ErrorCode::new("reverify_required", ErrorClass::Domain);
pub const MANDATORY_CONSENT_MISSING: ErrorCode =
    ErrorCode::new("mandatory_consent_missing", ErrorClass::Domain);
pub const DUPLICATE_MEMBER: ErrorCode =
    ErrorCode::new("duplicate_member", ErrorClass::AlreadyExists);
/// The email or sign-in id belonged to a member who withdrew and is still on hold.
pub const ID_REUSE_BLOCKED: ErrorCode = ErrorCode::new("id_reuse_blocked", ErrorClass::Domain);
/// The same person withdrew recently (`withdrawal.rejoin.block_days`).
pub const REJOIN_BLOCKED: ErrorCode = ErrorCode::new("rejoin_blocked", ErrorClass::Domain);
pub const ALREADY_BLOCKED: ErrorCode = ErrorCode::new("already_blocked", ErrorClass::InvalidState);
pub const NOT_BLOCKED: ErrorCode = ErrorCode::new("not_blocked", ErrorClass::InvalidState);
pub const MEMBER_NOT_ACTIVE: ErrorCode =
    ErrorCode::new("member_not_active", ErrorClass::InvalidState);
pub const ALREADY_WITHDRAWN: ErrorCode =
    ErrorCode::new("already_withdrawn", ErrorClass::InvalidState);
/// The identity check method is not turned on for that purpose (`identity_verification`).
pub const METHOD_NOT_ENABLED: ErrorCode =
    ErrorCode::new("method_not_enabled", ErrorClass::InvalidInput);
pub const CODE_MISMATCH: ErrorCode = ErrorCode::new("code_mismatch", ErrorClass::Domain);
pub const VERIFICATION_EXPIRED: ErrorCode =
    ErrorCode::new("verification_expired", ErrorClass::Domain);
/// Every attempt of a code or link was used; start a new one.
pub const TOO_MANY_ATTEMPTS: ErrorCode =
    ErrorCode::new("too_many_attempts", ErrorClass::LimitExceeded);
pub const INVALID_FORMAT: ErrorCode = ErrorCode::new("invalid_format", ErrorClass::InvalidInput);
pub const REGISTRY_MISMATCH: ErrorCode = ErrorCode::new("registry_mismatch", ErrorClass::Domain);
pub const ALREADY_VERIFIED: ErrorCode =
    ErrorCode::new("already_verified", ErrorClass::InvalidState);
pub const NOT_BUSINESS_MEMBER: ErrorCode =
    ErrorCode::new("not_business_member", ErrorClass::InvalidState);
/// Recovery links go only to a contact the member already proved they own.
pub const NO_VERIFIED_CONTACT: ErrorCode =
    ErrorCode::new("no_verified_contact", ErrorClass::Domain);
// Person-only endpoints (their errors never name a next tool).
pub const WITHDRAWAL_BLOCKED: ErrorCode = ErrorCode::new("withdrawal_blocked", ErrorClass::Domain);
pub const REAUTHENTICATION_FAILED: ErrorCode =
    ErrorCode::new("reauthentication_failed", ErrorClass::Unauthenticated);
pub const REAUTHENTICATION_LOCKED: ErrorCode =
    ErrorCode::new("reauthentication_locked", ErrorClass::Locked);
pub const CONFIRMATION_REQUIRED: ErrorCode =
    ErrorCode::new("confirmation_required", ErrorClass::InvalidInput);
pub const FORBIDDEN_INITIATOR: ErrorCode =
    ErrorCode::new("forbidden_initiator", ErrorClass::Forbidden);
pub const INVALID_IDENTIFIER_FORMAT: ErrorCode =
    ErrorCode::new("invalid_identifier_format", ErrorClass::InvalidInput);
/// A reset token or code that does not exist, expired, or was used; the three are not told
/// apart.
pub const INVALID_OR_EXPIRED: ErrorCode = ErrorCode::new("invalid_or_expired", ErrorClass::Domain);
pub const PASSWORD_POLICY: ErrorCode = ErrorCode::new("password_policy", ErrorClass::Domain);
pub const PASSWORD_REUSED: ErrorCode = ErrorCode::new("password_reused", ErrorClass::Domain);
pub const INVALID_CONTACT_FORMAT: ErrorCode =
    ErrorCode::new("invalid_contact_format", ErrorClass::InvalidInput);
pub const CURRENT_PASSWORD_WRONG: ErrorCode =
    ErrorCode::new("current_password_wrong", ErrorClass::Unauthenticated);

// ── membership ──────────────────────────────────────────────────────────────────────────
pub const TIER_RANK_CONFLICT: ErrorCode = ErrorCode::new("tier_rank_conflict", ErrorClass::Domain);
pub const ENTRY_TIER_MISSING: ErrorCode = ErrorCode::new("entry_tier_missing", ErrorClass::Domain);
/// A tier to remove is named by running promotions, coupons or displays.
pub const TIER_IN_USE: ErrorCode = ErrorCode::new("tier_in_use", ErrorClass::PolicyViolation);
/// Removing a tier needs `tier_migrations` saying where its members go.
pub const MIGRATION_REQUIRED: ErrorCode = ErrorCode::new("migration_required", ErrorClass::Domain);
pub const COUPON_NOT_FOUND: ErrorCode = ErrorCode::new("coupon_not_found", ErrorClass::NotFound);
pub const TIER_LIMIT_EXCEEDED: ErrorCode =
    ErrorCode::new("tier_limit_exceeded", ErrorClass::LimitExceeded);
pub const AS_OF_IN_FUTURE: ErrorCode = ErrorCode::new("as_of_in_future", ErrorClass::InvalidInput);
pub const EVALUATION_IN_PROGRESS: ErrorCode =
    ErrorCode::new("evaluation_in_progress", ErrorClass::InvalidState);
pub const COUPON_UNAVAILABLE: ErrorCode = ErrorCode::new("coupon_unavailable", ErrorClass::Domain);
pub const REASON_REQUIRED: ErrorCode = ErrorCode::new("reason_required", ErrorClass::InvalidInput);
pub const OVERRIDE_TOO_LONG: ErrorCode =
    ErrorCode::new("override_too_long", ErrorClass::InvalidInput);

// ── consent ─────────────────────────────────────────────────────────────────────────────
/// Terms and privacy are withdrawn only by leaving the store.
pub const CANNOT_REVOKE_MANDATORY: ErrorCode =
    ErrorCode::new("cannot_revoke_mandatory", ErrorClass::Domain);
pub const UNKNOWN_CONSENT_TYPE: ErrorCode =
    ErrorCode::new("unknown_consent_type", ErrorClass::InvalidInput);

// ── employee ────────────────────────────────────────────────────────────────────────────
pub const DUPLICATE_EMPLOYEE_NUMBER: ErrorCode =
    ErrorCode::new("duplicate_employee_number", ErrorClass::AlreadyExists);
pub const EMPLOYEE_NOT_FOUND: ErrorCode =
    ErrorCode::new("employee_not_found", ErrorClass::NotFound);

// ── inquiry ─────────────────────────────────────────────────────────────────────────────
pub const INQUIRY_NOT_FOUND: ErrorCode = ErrorCode::new("inquiry_not_found", ErrorClass::NotFound);
pub const INVALID_INQUIRY_TYPE: ErrorCode =
    ErrorCode::new("invalid_inquiry_type", ErrorClass::InvalidInput);
/// Nobody can take the inquiry by round robin; assign it by hand.
pub const NO_AVAILABLE_AGENT: ErrorCode = ErrorCode::new("no_available_agent", ErrorClass::Domain);
pub const INQUIRY_CLOSED: ErrorCode = ErrorCode::new("inquiry_closed", ErrorClass::InvalidState);

// ── remedy ──────────────────────────────────────────────────────────────────────────────
pub const REMEDY_NOT_FOUND: ErrorCode = ErrorCode::new("remedy_not_found", ErrorClass::NotFound);
pub const INVALID_LIABILITY: ErrorCode =
    ErrorCode::new("invalid_liability", ErrorClass::InvalidInput);
pub const ORDER_NOT_FOUND: ErrorCode = ErrorCode::new("order_not_found", ErrorClass::NotFound);
/// Paying cash out needs the payout approver's role (`remedy.approval_levels`).
pub const WRONG_APPROVER: ErrorCode = ErrorCode::new("wrong_approver", ErrorClass::Forbidden);
pub const ALREADY_DECIDED: ErrorCode = ErrorCode::new("already_decided", ErrorClass::InvalidState);
pub const PAYOUT_NOT_APPROVED: ErrorCode =
    ErrorCode::new("payout_not_approved", ErrorClass::Domain);

// ── faq_notice ──────────────────────────────────────────────────────────────────────────
pub const FAQ_NOT_FOUND: ErrorCode = ErrorCode::new("faq_not_found", ErrorClass::NotFound);
pub const NOTICE_NOT_FOUND: ErrorCode = ErrorCode::new("notice_not_found", ErrorClass::NotFound);
pub const ALREADY_PUBLISHED: ErrorCode =
    ErrorCode::new("already_published", ErrorClass::InvalidState);

/// Adds every customer operation to the registry.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    b.read(person::GetMyProfile)
        .read(person::GetReturnHistory)
        .read(person::SearchMembers)
        .read(person::GetMember)
        .write(person::RegisterMember)
        .write(person::UpdateMember)
        .write(person::BlockMember)
        .write(person::UnblockMember)
        .write(person::VerifyIdentity)
        .read(person::CheckAvailability)
        .write(person::VerifyBusiness)
        .read(person::CheckWithdrawalEligibility)
        .write(person::SendPasswordReset)
        .person(person::WithdrawAccount)
        .person(person::RequestPasswordReset)
        .person(person::ConfirmPasswordReset)
        .person(person::RecoverLoginId)
        .person(person::ChangePassword)
        .read(membership::ListTiers)
        .read(membership::GetTier)
        .read(membership::GetMemberTier)
        .write(membership::DefineTierProgram)
        .read(membership::PreviewTierEvaluation)
        .write(membership::RunTierEvaluation)
        .write(membership::SetMemberTier)
        .read(membership::ListTierHistory)
        .read(address::ListAddresses)
        .write(address::AddAddress)
        .write(address::UpdateAddress)
        .write(address::SetPrimaryAddress)
        .read(preference::GetPreferences)
        .write(preference::UpdateMyPreferences)
        .person(preference::UndoMemory)
        .read(wishlist::ListWishlist)
        .write(wishlist::AddToWishlist)
        .write(wishlist::RemoveFromWishlist)
        .read(faq_notice::ListFaqs)
        .read(faq_notice::GetFaq)
        .read(faq_notice::ListNotices)
        .read(faq_notice::GetNotice)
        .write(faq_notice::PublishNotice)
        .read(consent::GetConsent)
        .write(consent::SetConsent)
        .write(employee::RegisterEmployee)
        .write(employee::SetDiscountLimit)
        .read(inquiry::SearchInquiries)
        .read(inquiry::GetInquiry)
        .write(inquiry::CreateInquiry)
        .write(inquiry::AssignInquiry)
        .write(inquiry::RespondInquiry)
        .write(inquiry::CloseInquiry)
        .write(remedy::RequestRemedy)
        .write(remedy::ApproveRemedy)
        .write(remedy::PayRemedy);
    // Scheduled work (platform.job): the regular tier evaluation of each program.
    b.job(watch::EvaluateTiers);
}

#[cfg(test)]
mod tests {
    use acom_contract::{Kind, RegistryBuilder};

    #[test]
    fn every_operation_registers_with_a_closed_schema() {
        let mut b = RegistryBuilder::default();
        super::register(&mut b);
        let registry = b.build().unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(registry.len(), 54);
        for op in registry.iter() {
            let m = op.meta();
            assert_eq!(m.domain(), "customer", "{}", m.tool);
            assert!(m.path.starts_with("/v1/customer/"), "{}", m.tool);
            if m.kind == Kind::Person {
                let scope = if m.tool == "undo_memory" {
                    "human:undo"
                } else {
                    "human:account"
                };
                assert_eq!(m.person_scope, scope, "{}", m.tool);
            } else {
                assert!(m.person_scope.is_empty(), "{}", m.tool);
            }
        }
    }
}
