//! What catalog and search need from customers: a shopper's member tier (to price for them)
//! and whether a tier exists (to price for it on an operator's request).

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// The customer's member tier, the highest-ranked one if they hold several; `None` when they
/// are in no program.
pub async fn tier_of(tx: &mut TxCx, customer_id: &str) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT t.id
         FROM customer.program_membership m
         JOIN customer.member_program_tier t ON t.tenant_id = m.tenant_id AND t.id = m.tier_id
         WHERE m.tenant_id = $1 AND m.customer_id = $2
         ORDER BY t.rank DESC, t.id
         LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Whether the store has a member tier with this id.
pub async fn tier_exists(tx: &mut TxCx, tier: &str) -> Result<bool, ToolError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM customer.member_program_tier
                        WHERE tenant_id = $1 AND id = $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(tier)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}
