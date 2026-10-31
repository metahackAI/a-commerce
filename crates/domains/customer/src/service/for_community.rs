//! What `community` reads from customer: who a shopper writing a post or a reply is, by the name
//! the store shows (before community masks it) and whether their account may still write.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};

/// A shopper as a community writer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writer {
    pub id: String,
    /// The given name when the shopper has one, else the full name (`Jamie`).
    pub name: String,
    /// `active`, `dormant`, `blocked`, `pending_withdrawal` or `withdrawn`.
    pub member_state: String,
}

/// One shopper as a writer, or `None` when the store has no such shopper.
pub async fn writer(tx: &mut TxCx, customer_id: &str) -> Result<Option<Writer>, ToolError> {
    let row: Option<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT id, name, given_name, member_state
         FROM customer.person WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))?;
    Ok(row.map(|(id, name, given, member_state)| Writer {
        id,
        name: given
            .map(|g| g.trim().to_owned())
            .filter(|g| !g.is_empty())
            .unwrap_or(name),
        member_state,
    }))
}
