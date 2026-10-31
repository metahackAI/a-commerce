//! The `payment` resource (`docs/api/sales.md` §payment): `get_payment`.

pub mod get_payment;

use acom_contract::{ErrorClass, ErrorCode};

pub(crate) const PAYMENT_NOT_FOUND: ErrorCode =
    ErrorCode::new("payment_not_found", ErrorClass::NotFound);
