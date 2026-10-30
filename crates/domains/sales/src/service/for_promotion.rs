//! What promotion reads from sales: a cart's lines, priced from the catalog, for the
//! `calculate_benefits` tool. Promotion cannot depend on sales, so sales installs
//! [`benefit_cart`] into `acom_promotion::service::for_sales` when it registers.

use acom_catalog::service::for_sales as catalog;
use acom_customer::service::for_sales as customer;
use acom_promotion::service::for_sales::{BenefitCart, Line, Reading};
use acom_store::TxCx;

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
