//! A seller's activity, read through the services other domains export for partner: the
//! products it sells (catalog), their order lines (sales), parcels and claims (fulfillment) and
//! product questions (catalog). Partner only knows which products are the seller's.

use std::collections::{BTreeMap, BTreeSet};

use acom_catalog::service::for_partner::{self as catalog, QuestionCounts, SellerProduct};
use acom_contract::ToolError;
use acom_fulfillment::service::for_partner as fulfillment;
use acom_sales::service::for_partner as sales;
use acom_store::TxCx;
use chrono::{DateTime, NaiveDate, Utc};

use crate::model::{self, ClaimLine, Line, Metrics, Settlement, SettlementTerms, Shipment};
use crate::repo;
use crate::rules::Rules;

/// The products a seller sells, as catalog has them.
pub(crate) async fn products(
    tx: &mut TxCx,
    seller_id: &str,
) -> Result<Vec<SellerProduct>, ToolError> {
    let ids: Vec<String> = repo::products(tx, seller_id)
        .await?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    catalog::products(tx, &ids).await
}

fn variants(products: &[SellerProduct]) -> Vec<String> {
    products
        .iter()
        .flat_map(|p| p.variant_ids.iter().cloned())
        .collect()
}

fn product_ids(products: &[SellerProduct]) -> Vec<String> {
    products.iter().map(|p| p.product_id.clone()).collect()
}

/// Lines, parcels and claims of a seller's products for orders placed in `[from, until)`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Activity {
    pub lines: Vec<Line>,
    pub shipments: Vec<Shipment>,
    pub claims: Vec<ClaimLine>,
}

pub(crate) async fn gather(
    tx: &mut TxCx,
    products: &[SellerProduct],
    from: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<Activity, ToolError> {
    let category: BTreeMap<&str, Option<&String>> = products
        .iter()
        .flat_map(|p| {
            p.variant_ids
                .iter()
                .map(move |v| (v.as_str(), p.category_id.as_ref()))
        })
        .collect();
    let found = sales::lines(tx, &variants(products), from, until).await?;
    let orders: Vec<String> = found
        .iter()
        .map(|l| l.order_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let items: Vec<String> = found.iter().map(|l| l.order_item_id.clone()).collect();
    let shipments = fulfillment::shipments(tx, &orders).await?;
    let claims = fulfillment::claims(tx, &items).await?;
    Ok(Activity {
        lines: found
            .into_iter()
            .map(|l| Line {
                category_id: category
                    .get(l.variant_id.as_str())
                    .copied()
                    .flatten()
                    .cloned(),
                order_id: l.order_id,
                order_item_id: l.order_item_id,
                order_date: l.order_date,
                confirmed_at: l.confirmed_at,
                item_status: l.order_item_status,
                quantity: l.order_quantity,
                price: l.price,
                discount: l.discount_value,
            })
            .collect(),
        shipments: shipments
            .into_iter()
            .map(|s| Shipment {
                order_id: s.order_id,
                items: s.order_item_ids,
                shipped_at: s.shipped_at,
                delivered_at: s.delivered_at,
            })
            .collect(),
        claims: claims
            .into_iter()
            .map(|c| ClaimLine {
                claim_id: c.claim_id,
                claim_type: c.claim_type,
                claim_status: c.claim_status,
                liability: c.liability,
                order_item_id: c.order_item_id,
            })
            .collect(),
    })
}

/// Product questions about a seller's products: asked and answered in the period, waiting now.
pub(crate) async fn questions(
    tx: &mut TxCx,
    products: &[SellerProduct],
    from: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<QuestionCounts, ToolError> {
    catalog::questions(tx, &product_ids(products), from, until).await
}

/// The metrics of a seller over the store days `from` to `through`.
pub(crate) async fn metrics(
    tx: &mut TxCx,
    rules: &Rules,
    products: &[SellerProduct],
    from: NaiveDate,
    through: NaiveDate,
) -> Result<Metrics, ToolError> {
    let start = model::day_start(&tx.time, from);
    let until = model::day_start(&tx.time, through.succ_opt().unwrap_or(through));
    let a = gather(tx, products, start, until).await?;
    let q = questions(tx, products, start, until).await?;
    let now = tx.clock.now().min(until);
    Ok(model::metrics(
        &a.lines,
        &a.shipments,
        &a.claims,
        (q.asked, q.answered),
        now,
        rules.ship_within_days,
        tx.currency,
    ))
}

/// Orders with a line of the seller's products still in progress.
pub(crate) async fn open_orders(
    tx: &mut TxCx,
    products: &[SellerProduct],
) -> Result<i64, ToolError> {
    sales::open_order_count(tx, &variants(products)).await
}

/// What the store owes the seller now and has not paid out (an estimate: payouts are made
/// outside A-Commerce).
pub(crate) async fn settlement(
    tx: &mut TxCx,
    rules: &Rules,
    products: &[SellerProduct],
    terms: &SettlementTerms,
) -> Result<Settlement, ToolError> {
    let now = tx.clock.now();
    let today = model::store_date(&tx.time, now);
    let from = terms.window_start(today, rules.payout_delay_days);
    let start = model::day_start(&tx.time, from);
    let a = gather(tx, products, start, now).await?;
    Ok(model::settlement(
        &a.lines,
        &a.shipments,
        terms,
        today,
        rules.payout_delay_days,
        tx.currency,
        &tx.time,
    ))
}
