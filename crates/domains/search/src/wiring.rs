//! Connects catalog to the domains above it (see `acom_catalog::neighbors`). Search sits above
//! catalog, customer, promotion, merchandising and sales in the crate graph, so it is the
//! place where catalog's lookups meet their service functions. Installed once by `register`.

use acom_catalog::neighbors::{
    CategoryLink, CategoryTerm, Neighbors, PurchasedItem, SalePrice, UnitsSold,
};
use acom_contract::ToolError;
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;

/// The service functions behind catalog's lookups.
#[derive(Debug, Clone, Copy, Default)]
pub struct Wiring;

impl Neighbors for Wiring {
    fn customer_tier<'a>(
        &'a self,
        tx: &'a mut TxCx,
        customer_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, ToolError>> {
        Box::pin(acom_customer::service::for_catalog::tier_of(
            tx,
            customer_id,
        ))
    }

    fn tier_exists<'a>(
        &'a self,
        tx: &'a mut TxCx,
        tier: &'a str,
    ) -> BoxFuture<'a, Result<bool, ToolError>> {
        Box::pin(acom_customer::service::for_catalog::tier_exists(tx, tier))
    }

    fn sale_prices<'a>(
        &'a self,
        tx: &'a mut TxCx,
        offer_ids: &'a [String],
        tier: Option<&'a str>,
        at: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Vec<SalePrice>, ToolError>> {
        Box::pin(async move {
            let rows =
                acom_promotion::service::for_catalog::sale_prices(tx, offer_ids, tier, at).await?;
            Ok(rows
                .into_iter()
                .map(|r| SalePrice {
                    offer_id: r.offer_id,
                    price: r.price,
                    price_currency: r.price_currency,
                    valid_for_member_tier: r.valid_for_member_tier,
                    promotion_id: r.promotion_id,
                    name: r.promotion_name,
                    valid_through: r.valid_through,
                })
                .collect())
        })
    }

    fn categories<'a>(
        &'a self,
        tx: &'a mut TxCx,
        product_ids: Option<&'a [String]>,
    ) -> BoxFuture<'a, Result<Vec<CategoryLink>, ToolError>> {
        Box::pin(async move {
            let rows =
                acom_merchandising::service::for_catalog::placements(tx, product_ids).await?;
            Ok(rows
                .into_iter()
                .map(|p| CategoryLink {
                    product_id: p.product_group_id,
                    category_id: p.category_id,
                    name: p.name,
                    is_primary: p.is_primary,
                    direct: p.direct,
                })
                .collect())
        })
    }

    fn category_products<'a>(
        &'a self,
        tx: &'a mut TxCx,
        category_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<Vec<String>>, ToolError>> {
        Box::pin(acom_merchandising::service::for_catalog::products_in(
            tx,
            category_id,
        ))
    }

    fn purchased_items<'a>(
        &'a self,
        tx: &'a mut TxCx,
        customer_id: &'a str,
        order_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<PurchasedItem>, ToolError>> {
        Box::pin(async move {
            let rows =
                acom_sales::service::for_catalog::order_lines(tx, customer_id, order_id).await?;
            Ok(rows
                .into_iter()
                .map(|l| PurchasedItem {
                    order_item_id: l.id,
                    variant_id: l.product_id,
                    order_item_status: l.order_item_status,
                })
                .collect())
        })
    }

    fn units_sold<'a>(
        &'a self,
        tx: &'a mut TxCx,
        variant_ids: &'a [String],
        days: u32,
    ) -> BoxFuture<'a, Result<UnitsSold, ToolError>> {
        Box::pin(async move {
            let (window, by_variant) =
                acom_sales::service::for_catalog::units_sold(tx, variant_ids, days).await?;
            Ok(UnitsSold {
                valid_from: Some(window.valid_from),
                valid_through: Some(window.valid_through),
                by_variant,
            })
        })
    }

    fn category_terms<'a>(
        &'a self,
        tx: &'a mut TxCx,
    ) -> BoxFuture<'a, Result<Vec<CategoryTerm>, ToolError>> {
        Box::pin(async move {
            let rows = acom_merchandising::service::for_catalog::terms(tx).await?;
            Ok(rows
                .into_iter()
                .map(|t| CategoryTerm {
                    id: t.id,
                    name: t.name,
                    term_code: t.term_code,
                    term_set: t.term_set,
                    active: t.active,
                })
                .collect())
        })
    }

    fn place_product<'a>(
        &'a self,
        tx: &'a mut TxCx,
        product_id: &'a str,
        category_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ToolError>> {
        Box::pin(acom_merchandising::service::for_catalog::place_product(
            tx,
            product_id,
            category_id,
        ))
    }

    fn remove_placements<'a>(
        &'a self,
        tx: &'a mut TxCx,
        product_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ToolError>> {
        Box::pin(acom_merchandising::service::for_catalog::remove_placements(
            tx, product_id,
        ))
    }
}
