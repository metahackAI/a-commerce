//! The `receipt` resource (`docs/api/sales.md` §receipt): `issue_receipt` and `get_receipt`.
//! A receipt is recorded here; the tax service that income-deduction receipts and tax invoices
//! would be filed with is outside this system, and the output says so.

pub mod get_receipt;
pub mod issue_receipt;

use acom_contract::{ErrorCode, ToolError};
use acom_core::StoreTime;
use serde_json::{Value, json};

use crate::outside_system;
use crate::repo::{self, ReceiptRow};

/// A receipt as people read it.
pub(crate) fn receipt_json(
    r: &ReceiptRow,
    order_number: &str,
    time: &StoreTime,
) -> Result<Value, ToolError> {
    Ok(json!({
        "id": r.id,
        "order_id": r.order_id,
        "order_number": order_number,
        "receipt_type": r.receipt_type,
        "issued_at": time.format(r.issued_at),
        "amount": repo::money(r.total_payment_due_value, &r.total_payment_due_currency)?,
        "tax_id": r.tax_id,
        "tax_filing": tax_filing(&r.receipt_type),
    }))
}

/// What happens with the tax service for a receipt type: a proof of expense is the store's own
/// document; the others would be filed, and that service is not connected.
pub(crate) fn tax_filing(receipt_type: &str) -> Value {
    match receipt_type {
        "proof" => Value::Null,
        _ => outside_system(
            "tax_service",
            "The tax service is not connected: this receipt is recorded here and was not filed.",
        ),
    }
}

pub(crate) fn no_receipt(order_id: &str, receipt_type: Option<&str>) -> ToolError {
    ToolError::new(
        ErrorCode::NOT_FOUND,
        match receipt_type {
            Some(t) => format!("Order {order_id} has no {} receipt.", t.replace('_', " ")),
            None => format!("Order {order_id} has no receipt yet."),
        },
    )
    .detail(json!({"entity": "receipt", "order_id": order_id, "receipt_type": receipt_type}))
    .next("issue_receipt")
}
