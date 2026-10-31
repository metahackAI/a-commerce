//! What promotion reads from sales: a cart's lines, priced from the catalog, for the
//! `calculate_benefits` tool; and orders and members' purchases ([`order_facts`],
//! [`member_purchases`]) for coupon redemption, purchase-triggered event entries and segments.
//! Promotion cannot depend on sales, so these are installed into
//! `acom_promotion::service::for_sales`: the cart reader when sales registers, the order readers
//! by `acom-registry`.

use acom_catalog::service::for_sales as catalog;
use acom_customer::service::for_sales as customer;
use acom_promotion::service::for_sales::{BenefitCart, Line, MemberPurchases, OrderFacts, Reading};
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::repo;

/// Cart `cart_id` with its lines priced, and the tier of `customer_id`. `None` when there is
/// no such cart.
pub fn benefit_cart<'a>(
    tx: &'a mut TxCx,
    cart_id: &'a str,
    customer_id: &'a str,
) -> Reading<'a, Option<BenefitCart>> {
    Box::pin(async move {
        let Some(cart) = repo::cart_by_id(tx, cart_id).await? else {
            return Ok(None);
        };
        let rows = repo::lines(tx, cart_id).await?;
        let ids: Vec<String> = rows.iter().map(|r| r.product_id.clone()).collect();
        let facts = catalog::variants(tx, &ids).await?;
        let lines = rows
            .iter()
            .filter_map(|r| {
                let v = facts.iter().find(|f| f.variant_id == r.product_id)?;
                Some(Line {
                    cart_line_id: r.id.clone(),
                    variant_id: v.variant_id.clone(),
                    product_id: v.product_id.clone(),
                    offer_id: v.offer_id.clone(),
                    quantity: u32::try_from(r.order_quantity).unwrap_or(0),
                    list_price: v.list_price,
                    cost: v.cost,
                })
            })
            .collect();
        Ok(Some(BenefitCart {
            cart_id: cart.id,
            customer_id: cart.customer_id,
            session_id: cart.session_id,
            tier_id: customer::member_tier(tx, customer_id).await?,
            lines,
        }))
    })
}

#[derive(sqlx::FromRow)]
struct OrderRow {
    id: String,
    customer_id: Option<String>,
    order_date: DateTime<Utc>,
    order_status: String,
    sales_channel: String,
    subtotal_value: Decimal,
    subtotal_currency: String,
    discount_value: Decimal,
    discount_currency: String,
    coupon_id: Option<String>,
    variant_ids: Vec<String>,
}

/// The orders among `order_ids` that exist, with the variants not cancelled on them.
pub fn order_facts<'a>(tx: &'a mut TxCx, order_ids: &'a [String]) -> Reading<'a, Vec<OrderFacts>> {
    Box::pin(async move {
        let rows = sqlx::query_as::<_, OrderRow>(
            "SELECT o.id, o.customer_id, o.order_date, o.order_status, o.sales_channel,
                    o.subtotal_value, o.subtotal_currency::text AS subtotal_currency,
                    o.discount_value, o.discount_currency::text AS discount_currency, o.coupon_id,
                    coalesce(array_agg(DISTINCT i.product_id)
                               FILTER (WHERE i.order_item_status <> 'cancelled'),
                             '{}') AS variant_ids
             FROM sales.sales_order o
             LEFT JOIN sales.order_item i ON i.tenant_id = o.tenant_id AND i.order_id = o.id
             WHERE o.tenant_id = $1 AND o.id = ANY($2)
             GROUP BY o.id",
        )
        .bind(tx.tenant.as_str())
        .bind(order_ids)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(repo::db)?;
        rows.into_iter()
            .map(|r| {
                Ok(OrderFacts {
                    subtotal: repo::money(r.subtotal_value, &r.subtotal_currency)?,
                    discount: repo::money(r.discount_value, &r.discount_currency)?,
                    order_id: r.id,
                    customer_id: r.customer_id,
                    order_date: r.order_date,
                    order_status: r.order_status,
                    sales_channel: r.sales_channel,
                    coupon_id: r.coupon_id,
                    variant_ids: r.variant_ids,
                })
            })
            .collect()
    })
}

#[derive(sqlx::FromRow)]
struct PurchasesRow {
    customer_id: String,
    last_order_at: DateTime<Utc>,
    spend: Decimal,
    currency: String,
    variant_ids: Vec<String>,
}

/// Every member's last order, spend (net of returned and cancelled items) and the variants they
/// kept, over the orders that were not cancelled.
pub fn member_purchases<'a>(tx: &'a mut TxCx) -> Reading<'a, Vec<MemberPurchases>> {
    Box::pin(async move {
        let rows = sqlx::query_as::<_, PurchasesRow>(
            "WITH taken AS (
               SELECT order_id, sum(price * order_quantity - discount_value) AS back
               FROM sales.order_item
               WHERE tenant_id = $1 AND order_item_status IN ('returned', 'cancelled')
               GROUP BY order_id),
             kept AS (
               SELECT o.customer_id, array_agg(DISTINCT i.product_id) AS variant_ids
               FROM sales.sales_order o
               JOIN sales.order_item i ON i.tenant_id = o.tenant_id AND i.order_id = o.id
               WHERE o.tenant_id = $1 AND o.customer_id IS NOT NULL
                 AND o.order_status <> 'cancelled'
                 AND i.order_item_status NOT IN ('returned', 'cancelled')
               GROUP BY o.customer_id)
             SELECT o.customer_id, max(o.order_date) AS last_order_at,
                    sum(greatest(o.total_payment_due_value - coalesce(t.back, 0), 0)) AS spend,
                    min(o.total_payment_due_currency::text) AS currency,
                    coalesce(min(k.variant_ids), '{}') AS variant_ids
             FROM sales.sales_order o
             LEFT JOIN taken t ON t.order_id = o.id
             LEFT JOIN kept k ON k.customer_id = o.customer_id
             WHERE o.tenant_id = $1 AND o.customer_id IS NOT NULL
               AND o.order_status <> 'cancelled'
             GROUP BY o.customer_id",
        )
        .bind(tx.tenant.as_str())
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(repo::db)?;
        rows.into_iter()
            .map(|r| {
                Ok(MemberPurchases {
                    spend: repo::money(r.spend, &r.currency)?,
                    customer_id: r.customer_id,
                    last_order_at: r.last_order_at,
                    variant_ids: r.variant_ids,
                })
            })
            .collect()
    })
}
