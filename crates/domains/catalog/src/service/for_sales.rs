//! What the cart, checkout and order placement read from the catalog: a variant's product,
//! options, list price, cost and stock; finding a variant by size and color; and taking stock
//! when an order is placed.

use acom_contract::{ErrorCode, ToolError};
use acom_core::{Currency, Money};
use acom_store::{StoreError, TxCx};
use rust_decimal::Decimal;
use serde_json::Value;

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

fn money(value: Decimal, currency: &str, what: &str) -> Result<Money, ToolError> {
    let currency = Currency::parse(currency.trim())
        .map_err(|e| ToolError::new(ErrorCode::INTERNAL, format!("{what}: {e}")))?;
    Ok(Money::new(value, currency))
}

/// One purchasable variant with its offer.
#[derive(Debug, Clone, PartialEq)]
pub struct VariantFacts {
    /// `var_…`
    pub variant_id: String,
    /// `prd_…`, the product people name.
    pub product_id: String,
    /// The product's name, as printed on orders ("Packable windbreaker").
    pub product_name: String,
    pub color: Option<String>,
    pub size: Option<String>,
    /// `acom:saleType` word: `always`, `pre_order`, `subscription`, `gift_card`.
    pub sale_type: String,
    /// The product's `approval_status` word; only `active` products are sold.
    pub approval_status: String,
    /// `off_…`
    pub offer_id: String,
    /// Unit list price.
    pub list_price: Money,
    /// Unit cost, for the price floor; `None` when unknown.
    pub cost: Option<Money>,
    /// `schema:availability` word.
    pub availability: String,
    /// Units that can be sold now.
    pub inventory_level: i64,
    /// The offer row's version, for previews' basis.
    pub offer_version: i64,
}

impl VariantFacts {
    /// Whether the variant is on sale now: an active product whose offer is in stock (or taking
    /// pre-orders and back orders).
    pub fn sellable(&self) -> bool {
        self.approval_status == "active"
            && matches!(
                self.availability.as_str(),
                "in_stock" | "pre_order" | "back_order"
            )
    }

    /// Whether `quantity` units can be sold now.
    pub fn has_stock(&self, quantity: i64) -> bool {
        match self.availability.as_str() {
            "pre_order" | "back_order" => true,
            _ => self.inventory_level >= quantity,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct VariantRow {
    variant_id: String,
    product_id: String,
    product_name: String,
    color: Option<String>,
    size: Option<String>,
    sale_type: String,
    approval_status: String,
    offer_id: String,
    price: Decimal,
    price_currency: String,
    cost_value: Option<Decimal>,
    cost_currency: Option<String>,
    availability: String,
    inventory_level: i32,
    offer_version: i32,
}

impl VariantRow {
    fn facts(self) -> Result<VariantFacts, ToolError> {
        let list_price = money(self.price, &self.price_currency, &self.offer_id)?;
        let cost = match (self.cost_value, self.cost_currency.as_deref()) {
            (Some(v), Some(c)) => Some(money(v, c, &self.offer_id)?),
            _ => None,
        };
        Ok(VariantFacts {
            variant_id: self.variant_id,
            product_id: self.product_id,
            product_name: self.product_name,
            color: self.color,
            size: self.size,
            sale_type: self.sale_type,
            approval_status: self.approval_status,
            offer_id: self.offer_id,
            list_price,
            cost,
            availability: self.availability,
            inventory_level: i64::from(self.inventory_level),
            offer_version: i64::from(self.offer_version),
        })
    }
}

const VARIANT_SELECT: &str = "SELECT v.id AS variant_id, g.id AS product_id, g.name AS product_name,
       v.color, v.size, v.sale_type, g.approval_status, o.id AS offer_id, o.price,
       o.price_currency::text AS price_currency, o.cost_value, o.cost_currency::text AS cost_currency,
       o.availability, o.inventory_level, o.version AS offer_version
     FROM catalog.product v
     JOIN catalog.product_group g ON g.tenant_id = v.tenant_id AND g.id = v.product_group_id
     JOIN catalog.offer o ON o.tenant_id = v.tenant_id AND o.product_id = v.id";

/// The given variants that exist and have an offer, in no particular order.
pub async fn variants(
    tx: &mut TxCx,
    variant_ids: &[String],
) -> Result<Vec<VariantFacts>, ToolError> {
    if variant_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, VariantRow>(sqlx::AssertSqlSafe(format!(
        "{VARIANT_SELECT} WHERE v.tenant_id = $1 AND v.id = ANY($2)"
    )))
    .bind(tx.tenant.as_str())
    .bind(variant_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    let facts = rows
        .into_iter()
        .map(VariantRow::facts)
        .collect::<Result<Vec<_>, _>>()?;
    with_scheduled_prices(tx, facts).await
}

/// A scheduled list price in effect now replaces the offer's own price
/// (`pricing.scheduled_change.compute: on_demand`).
async fn with_scheduled_prices(
    tx: &mut TxCx,
    mut facts: Vec<VariantFacts>,
) -> Result<Vec<VariantFacts>, ToolError> {
    let at = tx.clock.now();
    let offers: Vec<String> = facts.iter().map(|f| f.offer_id.clone()).collect();
    let now = crate::write::scheduled_prices(tx, &offers, at).await?;
    for f in &mut facts {
        if let Some(p) = now.get(&f.offer_id) {
            f.list_price = Money::new(*p, f.list_price.currency);
        }
    }
    Ok(facts)
}

/// What looking a variant up by its product's options found.
#[derive(Debug, Clone, PartialEq)]
pub enum VariantMatch {
    Found(Box<VariantFacts>),
    /// There is no such product.
    NoProduct,
    /// The product exists but not with that size and color; these are the ones it has.
    NoSuchOption {
        sizes: Vec<String>,
        colors: Vec<String>,
    },
}

/// Finds the variant of `product_id` with this size and color (letter case ignored).
pub async fn find_variant(
    tx: &mut TxCx,
    product_id: &str,
    size: &str,
    color: &str,
) -> Result<VariantMatch, ToolError> {
    let rows = sqlx::query_as::<_, VariantRow>(sqlx::AssertSqlSafe(format!(
        "{VARIANT_SELECT} WHERE v.tenant_id = $1 AND v.product_group_id = $2 ORDER BY v.id"
    )))
    .bind(tx.tenant.as_str())
    .bind(product_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    if rows.is_empty() {
        let exists: Option<String> = sqlx::query_scalar(
            "SELECT id FROM catalog.product_group WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(product_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        return Ok(match exists {
            None => VariantMatch::NoProduct,
            Some(_) => VariantMatch::NoSuchOption {
                sizes: Vec::new(),
                colors: Vec::new(),
            },
        });
    }
    let same = |a: &Option<String>, b: &str| {
        a.as_deref()
            .is_some_and(|a| a.eq_ignore_ascii_case(b.trim()))
    };
    let mut sizes: Vec<String> = Vec::new();
    let mut colors: Vec<String> = Vec::new();
    for r in rows {
        if same(&r.size, size) && same(&r.color, color) {
            let found = with_scheduled_prices(tx, vec![r.facts()?]).await?;
            return Ok(match found.into_iter().next() {
                Some(f) => VariantMatch::Found(Box::new(f)),
                None => VariantMatch::NoProduct,
            });
        }
        if let Some(s) = &r.size
            && !sizes.contains(s)
        {
            sizes.push(s.clone());
        }
        if let Some(c) = &r.color
            && !colors.contains(c)
        {
            colors.push(c.clone());
        }
    }
    Ok(VariantMatch::NoSuchOption { sizes, colors })
}

/// A product as a list shows it: its name, lowest list price, and whether any variant can be
/// bought.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductSummary {
    pub product_id: String,
    pub name: String,
    pub price: Money,
    /// `in_stock` when any variant can be bought, else the first variant's availability word.
    pub availability: String,
}

/// Summaries of the given products that exist and have at least one offer.
pub async fn product_summaries(
    tx: &mut TxCx,
    product_ids: &[String],
) -> Result<Vec<ProductSummary>, ToolError> {
    if product_ids.is_empty() {
        return Ok(Vec::new());
    }
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, Decimal, String, bool, String)> = sqlx::query_as(
        "SELECT g.id, g.name, min(o.price), min(o.price_currency::text),
                bool_or(o.availability = 'in_stock' AND o.inventory_level > 0),
                min(o.availability)
         FROM catalog.product_group g
         JOIN catalog.product v ON v.tenant_id = g.tenant_id AND v.product_group_id = g.id
         JOIN catalog.offer o ON o.tenant_id = v.tenant_id AND o.product_id = v.id
         WHERE g.tenant_id = $1 AND g.id = ANY($2)
         GROUP BY g.id, g.name",
    )
    .bind(tx.tenant.as_str())
    .bind(product_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    rows.into_iter()
        .map(|(id, name, price, currency, any_in_stock, first)| {
            Ok(ProductSummary {
                price: money(price, &currency, &id)?,
                product_id: id,
                name,
                availability: if any_in_stock {
                    "in_stock".to_owned()
                } else {
                    first
                },
            })
        })
        .collect()
}

/// A product's sellable stock, for the stock-out forecast (`watchers.stock_out_forecast`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductStock {
    /// `prd_…`
    pub product_id: String,
    pub name: String,
    /// Units on hand over the variants counted.
    pub inventory_level: i64,
    /// The variants counted (`var_…`).
    pub variant_ids: Vec<String>,
}

/// Every product's stock over its variants whose offer is not in one of `exclude_availability`
/// (`discontinued`, `pre_order`), in product order.
pub async fn product_stock(
    tx: &mut TxCx,
    exclude_availability: &[String],
) -> Result<Vec<ProductStock>, ToolError> {
    let rows: Vec<(String, String, i64, Vec<String>)> = sqlx::query_as(
        "SELECT g.id, g.name, sum(o.inventory_level)::bigint, array_agg(v.id ORDER BY v.id)
         FROM catalog.product_group g
         JOIN catalog.product v ON v.tenant_id = g.tenant_id AND v.product_group_id = g.id
         JOIN catalog.offer o ON o.tenant_id = v.tenant_id AND o.product_id = v.id
         WHERE g.tenant_id = $1 AND NOT (o.availability = ANY($2))
         GROUP BY g.id, g.name ORDER BY g.id",
    )
    .bind(tx.tenant.as_str())
    .bind(exclude_availability)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(rows
        .into_iter()
        .map(
            |(product_id, name, inventory_level, variant_ids)| ProductStock {
                product_id,
                name,
                inventory_level,
                variant_ids,
            },
        )
        .collect())
}

/// Takes `quantity` units of a variant's stock for a placed order. Returns `false` (and changes
/// nothing) when there are not enough units. A variant whose stock reaches zero turns
/// out of stock when `catalog.inventory.auto_out_of_stock` says so.
pub async fn take_stock(tx: &mut TxCx, variant_id: &str, quantity: i64) -> Result<bool, ToolError> {
    let quantity = i32::try_from(quantity)
        .map_err(|_| ToolError::invalid_input("The quantity is too large."))?;
    // Pre-orders and back orders are sold beyond the stock on hand, which then stays at zero.
    let left: Option<(i32, String)> = sqlx::query_as(
        "UPDATE catalog.offer
         SET inventory_level = greatest(inventory_level - $3, 0), updated_at = now(),
             version = version + 1
         WHERE tenant_id = $1 AND product_id = $2
           AND (inventory_level >= $3 OR availability IN ('pre_order', 'back_order'))
         RETURNING inventory_level, availability",
    )
    .bind(tx.tenant.as_str())
    .bind(variant_id)
    .bind(quantity)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let Some((level, availability)) = left else {
        return Ok(false);
    };
    if level == 0 && availability == "in_stock" && auto_out_of_stock(tx) {
        sqlx::query(
            "UPDATE catalog.offer SET availability = 'out_of_stock', sold_out_at = $3,
               updated_at = now()
             WHERE tenant_id = $1 AND product_id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(variant_id)
        .bind(tx.clock.now())
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    Ok(true)
}

/// `catalog.inventory.auto_out_of_stock`: whether an empty variant turns out of stock.
fn auto_out_of_stock(tx: &TxCx) -> bool {
    tx.policy
        .lookup("catalog.inventory.auto_out_of_stock.set_availability")
        .and_then(Value::as_str)
        == Some("out_of_stock")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(availability: &str, level: i64, status: &str) -> VariantFacts {
        VariantFacts {
            variant_id: "var_windbreaker_m_slate".into(),
            product_id: "prd_windbreaker".into(),
            product_name: "Packable windbreaker".into(),
            color: Some("Slate".into()),
            size: Some("M".into()),
            sale_type: "always".into(),
            approval_status: status.into(),
            offer_id: "off_windbreaker_m_slate".into(),
            list_price: Money::new(Decimal::new(8900, 2), Currency::USD),
            cost: None,
            availability: availability.into(),
            inventory_level: level,
            offer_version: 1,
        }
    }

    #[test]
    fn sells_active_products_with_stock() {
        assert!(facts("in_stock", 18, "active").sellable());
        assert!(facts("in_stock", 18, "active").has_stock(18));
        assert!(!facts("in_stock", 18, "active").has_stock(19));
        assert!(!facts("out_of_stock", 0, "active").sellable());
        assert!(!facts("in_stock", 5, "draft").sellable());
        assert!(facts("back_order", 0, "active").has_stock(3));
    }
}
