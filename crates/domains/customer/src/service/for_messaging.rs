//! What `messaging` reads from customer: whether a shopper exists, their name (masked in
//! previews), and which channels they can be reached on.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};

/// How a shopper can be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub id: String,
    pub name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    /// `None` when the shopper gave no email.
    pub email: Option<String>,
    /// E.164; `None` when the shopper gave no phone.
    pub telephone: Option<String>,
    /// `active`, `dormant`, `blocked`, `pending_withdrawal` or `withdrawn`.
    pub member_state: String,
}

/// One shopper's contact details, or `None` when the store has no such shopper.
pub async fn contact(tx: &mut TxCx, customer_id: &str) -> Result<Option<Contact>, ToolError> {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    )> = sqlx::query_as(
        "SELECT id, name, given_name, family_name, email, telephone, member_state
         FROM customer.person WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))?;
    Ok(row.map(|r| Contact {
        id: r.0,
        name: r.1,
        given_name: r.2,
        family_name: r.3,
        email: r.4,
        telephone: r.5,
        member_state: r.6,
    }))
}
