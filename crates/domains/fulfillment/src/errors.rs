//! The error codes fulfillment operations return (docs/api/errors.md §2, fulfillment row), with
//! the class that sets the HTTP status.

use acom_contract::{ErrorClass, ErrorCode};

pub const CLAIM_NOT_FOUND: ErrorCode = ErrorCode::new("claim_not_found", ErrorClass::NotFound);
pub const ORDER_NOT_FOUND: ErrorCode = ErrorCode::new("order_not_found", ErrorClass::NotFound);
pub const ORDER_ITEM_NOT_FOUND: ErrorCode =
    ErrorCode::new("order_item_not_found", ErrorClass::NotFound);
pub const REFUND_NOT_FOUND: ErrorCode = ErrorCode::new("refund_not_found", ErrorClass::NotFound);
pub const DELIVERY_NOT_FOUND: ErrorCode =
    ErrorCode::new("delivery_not_found", ErrorClass::NotFound);
pub const TRACKING_NOT_FOUND: ErrorCode =
    ErrorCode::new("tracking_not_found", ErrorClass::NotFound);
pub const PRODUCT_NOT_FOUND: ErrorCode = ErrorCode::new("product_not_found", ErrorClass::NotFound);
pub const ADDRESS_NOT_FOUND: ErrorCode = ErrorCode::new("address_not_found", ErrorClass::NotFound);

pub const NOT_YOUR_ORDER: ErrorCode = ErrorCode::new("not_your_order", ErrorClass::Forbidden);

pub const INVALID_STATUS: ErrorCode = ErrorCode::new("invalid_status", ErrorClass::InvalidInput);
pub const INVALID_FILTER: ErrorCode = ErrorCode::new("invalid_filter", ErrorClass::InvalidInput);
pub const CURRENCY_MISMATCH: ErrorCode =
    ErrorCode::new("currency_mismatch", ErrorClass::InvalidInput);
pub const REASON_REQUIRED: ErrorCode = ErrorCode::new("reason_required", ErrorClass::InvalidInput);

pub const ALREADY_CANCELLED: ErrorCode =
    ErrorCode::new("already_cancelled", ErrorClass::InvalidState);
pub const ALREADY_REFUNDED: ErrorCode =
    ErrorCode::new("already_refunded", ErrorClass::InvalidState);
pub const INSPECTION_PENDING: ErrorCode =
    ErrorCode::new("inspection_pending", ErrorClass::InvalidState);
pub const NOT_REVOCABLE: ErrorCode = ErrorCode::new("not_revocable", ErrorClass::UndoWindowClosed);

pub const NOT_A_RETURN: ErrorCode = ErrorCode::new("not_a_return", ErrorClass::Domain);
pub const OUTSIDE_RETURN_WINDOW: ErrorCode =
    ErrorCode::new("outside_return_window", ErrorClass::Domain);
pub const NOT_RETURNABLE: ErrorCode = ErrorCode::new("not_returnable", ErrorClass::Domain);
pub const NOT_CANCELLABLE: ErrorCode = ErrorCode::new("not_cancellable", ErrorClass::Domain);
pub const NOT_SHIPPED_YET: ErrorCode = ErrorCode::new("not_shipped_yet", ErrorClass::Domain);
pub const ALREADY_SHIPPED: ErrorCode = ErrorCode::new("already_shipped", ErrorClass::Domain);
pub const ALREADY_HANDED_OVER: ErrorCode =
    ErrorCode::new("already_handed_over", ErrorClass::Domain);
pub const WHOLE_QUANTITY_ONLY: ErrorCode =
    ErrorCode::new("whole_quantity_only", ErrorClass::Domain);
pub const QUANTITY_OVER_CANCELLABLE: ErrorCode =
    ErrorCode::new("quantity_over_cancellable", ErrorClass::Domain);
pub const REASON_NOT_SELECTABLE: ErrorCode =
    ErrorCode::new("reason_not_selectable", ErrorClass::Domain);
pub const CLAIM_IN_PROGRESS: ErrorCode = ErrorCode::new("claim_in_progress", ErrorClass::Domain);
pub const REFUND_DESTINATION_REQUIRED: ErrorCode =
    ErrorCode::new("refund_destination_required", ErrorClass::Domain);
pub const EVIDENCE_REQUIRED: ErrorCode = ErrorCode::new("evidence_required", ErrorClass::Domain);
pub const CHARGE_UNPAID: ErrorCode = ErrorCode::new("charge_unpaid", ErrorClass::Domain);
pub const REPLACEMENT_UNAVAILABLE: ErrorCode =
    ErrorCode::new("replacement_unavailable", ErrorClass::Domain);
pub const AMOUNT_OVER_PAYMENT: ErrorCode =
    ErrorCode::new("amount_over_payment", ErrorClass::Domain);
pub const COUPON_NOT_DEDUCTED: ErrorCode =
    ErrorCode::new("coupon_not_deducted", ErrorClass::Domain);
pub const ADDRESS_NOT_DELIVERABLE: ErrorCode =
    ErrorCode::new("address_not_deliverable", ErrorClass::Domain);
pub const OUT_OF_STOCK: ErrorCode = ErrorCode::new("out_of_stock", ErrorClass::Domain);
pub const LIABILITY_COST_CONFLICT: ErrorCode =
    ErrorCode::new("liability_cost_conflict", ErrorClass::Domain);
pub const PAYMENT_GATEWAY_ERROR: ErrorCode =
    ErrorCode::new("payment_gateway_error", ErrorClass::UpstreamUnavailable);
