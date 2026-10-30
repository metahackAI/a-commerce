//! What catalog and search need from promotions: the member and sale prices in effect for a
//! viewer, so a product page and a search result show the price the shopper will pay.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// One sale price of one offer (`promotion.unit_price_specification`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct SalePriceRow {
    pub id: String,
    pub offer_id: String,
    pub price: Decimal,
    pub price_currency: String,
    /// `None` is everyone.
    pub valid_for_member_tier: Option<String>,
    pub promotion_id: Option<String>,
    /// The promotion's name, for example "VIP weekend".
    pub promotion_name: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: Option<DateTime<Utc>>,
}

/// Sale prices of these offers in effect at `at` for everyone or for `tier`. Prices are computed
/// from their period, not switched on and off (`catalog.yml pricing.sale_price.compute:
/// on_demand`); `valid_through` includes its whole minute (docs/api/conventions.md §5). Prices
/// of a promotion that is a draft, paused or ended do not apply.
pub async fn sale_prices(
    tx: &mut TxCx,
    offer_ids: &[String],
    tier: Option<&str>,
    at: DateTime<Utc>,
) -> Result<Vec<SalePriceRow>, ToolError> {
    sqlx::query_as::<_, SalePriceRow>(
        "SELECT u.id, u.offer_id, u.price, u.price_currency::text AS price_currency,
                u.valid_for_member_tier, u.promotion_id, p.name AS promotion_name,
                u.valid_from, u.valid_through
         FROM promotion.unit_price_specification u
         LEFT JOIN promotion.promotion p ON p.tenant_id = u.tenant_id AND p.id = u.promotion_id
         WHERE u.tenant_id = $1 AND u.offer_id = ANY($2::text[])
           AND u.price_type = 'sale_price'
           AND (u.valid_for_member_tier IS NULL OR u.valid_for_member_tier = $3)
           AND u.valid_from <= $4
           AND (u.valid_through IS NULL OR $4 < u.valid_through + interval '1 minute')
           AND (u.promotion_id IS NULL OR p.promotion_status IN ('scheduled', 'running'))",
    )
    .bind(tx.tenant.as_str())
    .bind(offer_ids)
    .bind(tier)
    .bind(at)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}
