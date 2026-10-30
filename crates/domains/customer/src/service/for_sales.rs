//! What the cart, checkout and order placement read about a shopper: who they are, their tier,
//! their saved card, and their saved addresses.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use serde::{Deserialize, Serialize};

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// A shopper as checkout sees them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shopper {
    pub id: String,
    pub name: String,
    pub member_state: String,
    /// The tier of their first program, `None` without a membership.
    pub tier_id: Option<String>,
    /// Last four digits of the saved card, `None` when no card is saved.
    pub card_last4: Option<String>,
}

/// A saved address (`customer.postal_address`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedAddress {
    pub id: String,
    pub customer_id: String,
    pub label: Option<String>,
    pub name: Option<String>,
    pub street_address: String,
    pub address_locality: String,
    pub address_region: Option<String>,
    pub postal_code: String,
    pub address_country: String,
    pub telephone: Option<String>,
    pub is_primary: bool,
    pub version: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct AddressRow {
    id: String,
    customer_id: String,
    label: Option<String>,
    name: Option<String>,
    street_address: String,
    address_locality: String,
    address_region: Option<String>,
    postal_code: String,
    address_country: String,
    telephone: Option<String>,
    is_primary: bool,
    version: i32,
}

impl From<AddressRow> for SavedAddress {
    fn from(r: AddressRow) -> Self {
        Self {
            id: r.id,
            customer_id: r.customer_id,
            label: r.label,
            name: r.name,
            street_address: r.street_address,
            address_locality: r.address_locality,
            address_region: r.address_region,
            postal_code: r.postal_code,
            address_country: r.address_country.trim().to_owned(),
            telephone: r.telephone,
            is_primary: r.is_primary,
            version: i64::from(r.version),
        }
    }
}

const ADDRESS_COLUMNS: &str = "id, customer_id, label, name, street_address, address_locality,
    address_region, postal_code, address_country, telephone, is_primary, version";

/// The shopper, or `None` when there is no such person in the store.
pub async fn shopper(tx: &mut TxCx, customer_id: &str) -> Result<Option<Shopper>, ToolError> {
    let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, name, member_state, card_last4 FROM customer.person
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let Some((id, name, member_state, card_last4)) = row else {
        return Ok(None);
    };
    let tier_id = member_tier(tx, &id).await?;
    Ok(Some(Shopper {
        id,
        name,
        member_state,
        tier_id,
        card_last4,
    }))
}

/// The shopper's current tier id (`vip`), `None` for people without a membership. With several
/// programs, the first program by id decides.
pub async fn member_tier(tx: &mut TxCx, customer_id: &str) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT tier_id FROM customer.program_membership
         WHERE tenant_id = $1 AND customer_id = $2
         ORDER BY member_program_id LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One of the shopper's saved addresses; `None` when it is not theirs or does not exist.
pub async fn address(
    tx: &mut TxCx,
    customer_id: &str,
    address_id: &str,
) -> Result<Option<SavedAddress>, ToolError> {
    let row = sqlx::query_as::<_, AddressRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ADDRESS_COLUMNS} FROM customer.postal_address
         WHERE tenant_id = $1 AND customer_id = $2 AND id = $3"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(address_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(row.map(SavedAddress::from))
}

/// Where the shopper's parcels go by default: the primary shipping address, else the first
/// shipping address they saved.
pub async fn default_shipping_address(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Option<SavedAddress>, ToolError> {
    let row = sqlx::query_as::<_, AddressRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ADDRESS_COLUMNS} FROM customer.postal_address
         WHERE tenant_id = $1 AND customer_id = $2 AND 'shipping' = ANY(address_usage)
         ORDER BY is_primary DESC, created_at, id LIMIT 1"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(row.map(SavedAddress::from))
}
