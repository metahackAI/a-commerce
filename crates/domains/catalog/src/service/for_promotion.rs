//! What the promotion domain reads from the catalog: products with every variant's offer (list
//! price, cost, stock), so `set_sale_prices` can price a sale, keep it above the price floor and
//! leave out products that are short of stock. Plain reads over the caller's transaction.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use rust_decimal::Decimal;

/// The offer of one variant.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantOffer {
    /// `off_…`, the offer a sale price is attached to.
    pub offer_id: String,
    /// `var_…`, the variant the offer sells.
    pub variant_id: String,
    pub color: Option<String>,
    pub size: Option<String>,
    /// The list price; a sale never changes it.
    pub price: Decimal,
    pub price_currency: String,
    /// What the store pays for one unit; `None` when it was never entered.
    pub cost_value: Option<Decimal>,
    pub cost_currency: Option<String>,
    /// Units that can be sold now.
    pub inventory_level: i32,
    /// `availability` word of the offer.
    pub availability: String,
    /// Row version of the offer, for a preview's basis.
    pub version: i32,
}

/// A product (`prd_…`) and the offers of its variants.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductOffers {
    pub product_id: String,
    pub name: String,
    /// `approval_status` word; only `active` products are on sale.
    pub approval_status: String,
    /// Variants that have an offer, in variant id order.
    pub offers: Vec<VariantOffer>,
}

impl ProductOffers {
    /// Units of all variants together.
    pub fn stock(&self) -> i64 {
        self.offers
            .iter()
            .map(|o| i64::from(o.inventory_level))
            .sum()
    }
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    product_id: String,
    name: String,
    approval_status: String,
    variant_id: Option<String>,
    color: Option<String>,
    size: Option<String>,
    offer_id: Option<String>,
    price: Option<Decimal>,
    price_currency: Option<String>,
    cost_value: Option<Decimal>,
    cost_currency: Option<String>,
    inventory_level: Option<i32>,
    availability: Option<String>,
    version: Option<i32>,
}

/// The products among `product_ids` that exist in this store, in the order asked, each with its
/// variants' offers. Ids that do not exist are left out; the caller reports them.
pub async fn products_with_offers(
    tx: &mut TxCx,
    product_ids: &[String],
) -> Result<Vec<ProductOffers>, ToolError> {
    let rows = sqlx::query_as::<_, Row>(
        "SELECT g.id AS product_id, g.name, g.approval_status, p.id AS variant_id, p.color, p.size,
                o.id AS offer_id, o.price, o.price_currency, o.cost_value, o.cost_currency,
                o.inventory_level, o.availability, o.version
         FROM catalog.product_group g
         LEFT JOIN catalog.product p ON p.tenant_id = g.tenant_id AND p.product_group_id = g.id
         LEFT JOIN catalog.offer o ON o.tenant_id = p.tenant_id AND o.product_id = p.id
         WHERE g.tenant_id = $1 AND g.id = ANY($2)
         ORDER BY g.id, p.id",
    )
    .bind(tx.tenant.as_str())
    .bind(product_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))?;

    let mut out: Vec<ProductOffers> = Vec::new();
    for r in rows {
        if out.last().is_none_or(|p| p.product_id != r.product_id) {
            out.push(ProductOffers {
                product_id: r.product_id.clone(),
                name: r.name.clone(),
                approval_status: r.approval_status.clone(),
                offers: Vec::new(),
            });
        }
        let (Some(offer_id), Some(variant_id), Some(price), Some(currency)) =
            (r.offer_id, r.variant_id, r.price, r.price_currency)
        else {
            continue;
        };
        if let Some(p) = out.last_mut() {
            p.offers.push(VariantOffer {
                offer_id,
                variant_id,
                color: r.color,
                size: r.size,
                price,
                price_currency: currency,
                cost_value: r.cost_value,
                cost_currency: r.cost_currency,
                inventory_level: r.inventory_level.unwrap_or(0),
                availability: r.availability.unwrap_or_default(),
                version: r.version.unwrap_or(1),
            });
        }
    }
    // Back to the order the caller asked for.
    out.sort_by_key(|p| {
        product_ids
            .iter()
            .position(|id| *id == p.product_id)
            .unwrap_or(usize::MAX)
    });
    Ok(out)
}
