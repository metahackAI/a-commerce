//! The `payment` resource (`docs/api/sales.md` §payment): `get_payment` and
//! `confirm_virtual_account_deposit`.

pub mod confirm_virtual_account_deposit;
pub mod get_payment;

use acom_contract::{ErrorClass, ErrorCode};

pub(crate) const PAYMENT_NOT_FOUND: ErrorCode =
    ErrorCode::new("payment_not_found", ErrorClass::NotFound);
