//! What customer reads from sales: a member's orders (tier progress) and product cards from the
//! catalog (the wishlist). Customer cannot depend on sales, so sales installs these readers into
//! `acom_customer::service::providers` when it registers.

use acom_catalog::service::for_sales as catalog;
use acom_customer::service::providers::{MemberOrder, ProductCard, Reading};
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::repo;

/// The member's orders placed at or after `since`, net of returned and cancelled lines.
pub fn member_orders<'a>(
    tx: &'a mut TxCx,
    customer_id: &'a str,
    since: DateTime<Utc>,
) -> Reading<'a, Vec<MemberOrder>> {
    Box::pin(async move {
        let filter = repo::OrderFilter {
            customer_id: Some(customer_id.to_owned()),
            from: Some(since),
            order_by: "order_date, id",
            limit: i64::MAX,
            ..repo::OrderFilter::default()
        };
        let (orders, _) = repo::orders(tx, &filter).await?;
        let ids: Vec<String> = orders.iter().map(|o| o.id.clone()).collect();
        let items = repo::items(tx, &ids).await?;
        let delivered = repo::delivered_at(tx, &ids).await?;
        let mut out = Vec::new();
        for o in orders.into_iter().filter(|o| o.order_status != "cancelled") {
            let taken_back: Decimal = items
                .iter()
                .filter(|i| {
                    i.order_id == o.id
                        && matches!(i.order_item_status.as_str(), "returned" | "cancelled")
                })
                .map(|i| i.price * Decimal::from(i.order_quantity) - i.discount_value)
                .sum();
            let paid = (o.total_payment_due_value - taken_back).max(Decimal::ZERO);
            let delivered_at = if matches!(o.order_status.as_str(), "delivered" | "returned") {
                delivered
                    .iter()
                    .find(|(id, _)| id == &o.id)
                    .map(|(_, t)| *t)
            } else {
                None
            };
            out.push(MemberOrder {
                amount: repo::money(paid, &o.total_payment_due_currency)?,
                order_id: o.id,
                order_number: o.order_number,
                sales_channel: o.sales_channel,
                delivered_at,
            });
        }
        Ok(out)
    })
}

/// Name, lowest list price and availability of these products.
pub fn product_cards<'a>(
    tx: &'a mut TxCx,
    product_ids: &'a [String],
) -> Reading<'a, Vec<ProductCard>> {
    Box::pin(async move {
        Ok(catalog::product_summaries(tx, product_ids)
            .await?
            .into_iter()
            .map(|p| ProductCard {
                product_id: p.product_id,
                name: p.name,
                price: p.price,
                availability: p.availability,
            })
            .collect())
    })
}
