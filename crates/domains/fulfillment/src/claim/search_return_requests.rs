//! `search_return_requests`: the return requests waiting for review, one queue row each.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::Money;
use acom_core::enums::{ClaimReason, ClaimStatus};
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::claim_lines;
use crate::errors::*;
use crate::model::delivery::{days_since, local_date};
use crate::model::returns::photo_count;
use crate::repo::{self, ClaimFilter, ClaimOrder};
use crate::{delivered_at, shopper};

/// Where a claim waits in the review queue.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
pub enum ReturnSort {
    /// Waiting longest first (the default).
    #[serde(rename = "requested_at")]
    RequestedAt,
    /// Largest amount first.
    #[serde(rename = "-amount")]
    AmountDescending,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchReturnRequestsInput {
    /// Only return requests in these states. Default: requested.
    #[schemars(example = ["requested"])]
    pub status: Option<Vec<ClaimStatus>>,
    /// Only requests worth at least this much, in the store's currency.
    #[schemars(example = json!({"value": 200.00, "currency": "USD"}))]
    pub min_amount: Option<Money>,
    /// Only requests with these reasons.
    #[schemars(example = ["defect", "damaged_in_transit"])]
    pub reason: Option<Vec<ClaimReason>>,
    /// requested_at: waiting longest first (default); -amount: largest first.
    #[schemars(example = &"requested_at")]
    pub sort: Option<ReturnSort>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchReturnRequests;

impl Operation for SearchReturnRequests {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.search_return_requests",
        tool: "search_return_requests",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/return-requests",
        summary: "Return requests waiting for review, one queue row each (claim, order, customer, item, amount, reason, photos, days since delivery). The first call when reviewing returns in a batch.",
        consumes: &[],
        produces: &[Slot::many("claim_id")],
        checks: &[],
        errors: &[INVALID_STATUS, CURRENCY_MISMATCH],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchReturnRequestsInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchReturnRequests {
    async fn read(&self, cx: &Cx, input: SearchReturnRequestsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let statuses: Vec<String> = input
            .status
            .unwrap_or_else(|| vec![ClaimStatus::Requested])
            .iter()
            .map(|s| s.as_str().to_owned())
            .collect();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if let Some(min) = &input.min_amount
            && repo::other_currency_exists(&mut tx, min.currency.as_str()).await?
        {
            return Err(ToolError::new(
                CURRENCY_MISMATCH,
                format!("Return amounts are not in {}.", min.currency),
            )
            .detail(json!({"field": "min_amount"}))
            .next("search_return_requests"));
        }
        let filter = ClaimFilter {
            claim_type: Some("return".into()),
            statuses,
            reasons: input
                .reason
                .unwrap_or_default()
                .iter()
                .map(|r| r.as_str().to_owned())
                .collect(),
            customer_id: shopper(&cx.principal),
            min_amount: input.min_amount.map(|m| m.value),
            ..ClaimFilter::default()
        };
        let order = match input.sort.unwrap_or(ReturnSort::RequestedAt) {
            ReturnSort::RequestedAt => ClaimOrder::Oldest,
            ReturnSort::AmountDescending => ClaimOrder::Largest,
        };
        let (total, rows) =
            repo::search_claims(&mut tx, &filter, order, page.limit, page.offset).await?;
        let claim_ids: Vec<String> = rows.iter().map(|c| c.id.clone()).collect();
        let order_ids: Vec<String> = rows.iter().map(|c| c.order_id.clone()).collect();
        let lines = claim_lines(&mut tx, &claim_ids).await?;
        let orders = sales::orders(&mut tx, &order_ids).await?;
        let deliveries = repo::deliveries_of_orders(&mut tx, &order_ids).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let now = cx.clock.now();
        let zone = cx.time.zone;
        let mut items = Vec::new();
        for c in &rows {
            let ls = lines.get(&c.id).map(Vec::as_slice).unwrap_or_default();
            let delivered = delivered_at(&deliveries, &c.order_id);
            items.push(json!({
                "claim_id": c.id,
                "order_id": c.order_id,
                "order_number": orders.iter().find(|o| o.id == c.order_id).map(|o| &o.order_number),
                "customer_id": c.customer_id,
                "order_item_id": ls.first().map(|l| &l.order_item_id),
                "item": ls.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(" + "),
                "quantity": ls.iter().map(|l| l.quantity).sum::<i32>(),
                "amount": c.amount()?,
                "reason": c.claim_reason,
                "reason_text": c.reason_text,
                "photos": photo_count(&c.evidence),
                "delivered_at": delivered.map(|d| local_date(d, zone).to_string()),
                "days_since_delivery": delivered.map(|d| days_since(d, now, zone)),
                "requested_at": cx.time.format(c.start_time),
                "status": c.claim_status,
            }));
        }
        let mut out = json!({"items": items, "total": total});
        if let Some(next) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(next);
        }
        Ok(out)
    }
}
