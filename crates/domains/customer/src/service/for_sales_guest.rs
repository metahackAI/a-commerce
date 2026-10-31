//! What `sales.order.verify_guest` asks about the person behind an order: whether an email or a
//! phone number someone typed is theirs. Only yes or no; the contact itself never leaves here.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};

/// Whether `email` (letter case and spaces ignored) or `telephone` (only its digits compared)
/// is the contact of shopper `customer_id`. `false` for an unknown shopper.
pub async fn contact_matches(
    tx: &mut TxCx,
    customer_id: &str,
    email: Option<&str>,
    telephone: Option<&str>,
) -> Result<bool, ToolError> {
    let email = email
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty());
    let digits = telephone
        .map(|t| t.chars().filter(char::is_ascii_digit).collect::<String>())
        .filter(|d| !d.is_empty());
    if email.is_none() && digits.is_none() {
        return Ok(false);
    }
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM customer.person
           WHERE tenant_id = $1 AND id = $2
             AND (($3::text IS NOT NULL AND lower(trim(email)) = $3)
               OR ($4::text IS NOT NULL
                   AND regexp_replace(coalesce(telephone, ''), '[^0-9]', '', 'g') = $4)))",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(email)
    .bind(digits)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))
}
