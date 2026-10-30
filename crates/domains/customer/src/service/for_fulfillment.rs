//! What `fulfillment` reads from customer: a shopper's saved addresses, for delivery-date
//! estimates. Return history is counted from fulfillment's own claims.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};

/// A saved address, as printed on a parcel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub id: String,
    pub customer_id: String,
    pub name: Option<String>,
    pub street_address: String,
    pub address_locality: String,
    pub address_region: Option<String>,
    pub postal_code: String,
    /// ISO 3166-1 alpha-2.
    pub address_country: String,
}

/// One of the shopper's own saved addresses, or `None` when the shopper has no such address.
pub async fn address(
    tx: &mut TxCx,
    customer_id: &str,
    address_id: &str,
) -> Result<Option<Address>, ToolError> {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        String,
        String,
        Option<String>,
        String,
        String,
        Option<String>,
        String,
        String,
    )> = sqlx::query_as(
        "SELECT id, customer_id, name, street_address, address_locality, address_region,
                postal_code, address_country::text
         FROM customer.postal_address
         WHERE tenant_id = $1 AND id = $2 AND customer_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(address_id)
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))?;
    Ok(row.map(|r| Address {
        id: r.0,
        customer_id: r.1,
        name: r.2,
        street_address: r.3,
        address_locality: r.4,
        address_region: r.5,
        postal_code: r.6,
        address_country: r.7.trim().to_owned(),
    }))
}
