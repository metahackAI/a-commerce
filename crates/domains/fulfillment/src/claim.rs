//! Claim reads: `search_return_requests` (the review queue), `evaluate_return_request` (the
//! policy's suggestion for one request), `search_claims` and `get_claim`.

use std::collections::BTreeMap;

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::Money;
use acom_core::enums::{ClaimReason, ClaimStatus, ClaimType, Liability};
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, Page, TxCx};
use chrono::Duration;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::errors::{
    CLAIM_NOT_FOUND, CURRENCY_MISMATCH, INVALID_FILTER, INVALID_STATUS, NOT_A_RETURN,
};
use crate::model::delivery::{days_since, local_date};
use crate::model::returns::{ClaimLine, EvalFacts, evaluate, photo_count, refund_amount};
use crate::repo::{self, ClaimFilter, ClaimOrder, ClaimRow};
use crate::{delivered_at, parse_time, rules, select, shopper};

pub(crate) fn claim_not_found(id: &str, next: &'static str) -> ToolError {
    ToolError::new(CLAIM_NOT_FOUND, format!("There is no claim {id}."))
        .detail(json!({"entity": "claim", "id": id}))
        .next(next)
}

/// A claim the caller may see: shoppers only see their own (others look missing).
pub(crate) async fn visible_claim(
    tx: &mut TxCx,
    claim_id: &str,
    next: &'static str,
) -> Result<ClaimRow, ToolError> {
    let me = shopper(&tx.principal);
    let claim = repo::claim(tx, claim_id)
        .await?
        .ok_or_else(|| claim_not_found(claim_id, next))?;
    if me.as_deref().is_some_and(|m| m != claim.customer_id) {
        return Err(claim_not_found(claim_id, next));
    }
    Ok(claim)
}

/// The order lines of each claim with what was paid for them, keyed by claim id.
pub(crate) async fn claim_lines(
    tx: &mut TxCx,
    claim_ids: &[String],
) -> Result<BTreeMap<String, Vec<ClaimLine>>, ToolError> {
    let rows = repo::claim_items(tx, claim_ids).await?;
    let item_ids: Vec<String> = rows.iter().map(|r| r.order_item_id.clone()).collect();
    let items = sales::items(tx, &item_ids).await?;
    let mut out: BTreeMap<String, Vec<ClaimLine>> = BTreeMap::new();
    for r in rows {
        let Some(i) = items.iter().find(|i| i.id == r.order_item_id) else {
            continue;
        };
        out.entry(r.claim_id).or_default().push(ClaimLine {
            order_item_id: i.id.clone(),
            name: i.name.clone(),
            unit_price: i.price,
            ordered: i.order_quantity,
            line_discount: i.discount,
            quantity: r.order_quantity,
        });
    }
    Ok(out)
}

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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluateReturnRequestInput {
    /// The return request to evaluate, from `search_return_requests`.
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_20402")]
    pub claim_id: String,
}

pub struct EvaluateReturnRequest;

impl Operation for EvaluateReturnRequest {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.evaluate_return_request",
        tool: "evaluate_return_request",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/return-requests/{claim_id}/evaluation",
        summary: "Apply the return policy to one return request and suggest refund, ask_for_photos or decline, with the policy rule behind it, the refund amount, whether a person must approve, and flags such as frequent returns. Changes nothing.",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[],
        errors: &[CLAIM_NOT_FOUND, ErrorCode::INVALID_STATE, NOT_A_RETURN],
        audience: Audience::Operator,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = EvaluateReturnRequestInput;
    type Output = Value;
}

impl ReadOp<Cx> for EvaluateReturnRequest {
    async fn read(&self, cx: &Cx, input: EvaluateReturnRequestInput) -> Result<Value, ToolError> {
        let rules = rules(&cx.policy)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let claim = visible_claim(&mut tx, &input.claim_id, "search_return_requests").await?;
        if claim.claim_type != "return" {
            return Err(ToolError::new(
                NOT_A_RETURN,
                format!(
                    "Claim {} is a {}, not a return.",
                    claim.id, claim.claim_type
                ),
            )
            .detail(json!({"entity": "claim", "claim_type": claim.claim_type}))
            .next("get_claim"));
        }
        if !matches!(
            claim.claim_status.as_str(),
            "requested" | "photos_requested"
        ) {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "Return {} is already {}.",
                    claim.id,
                    claim.claim_status.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "claim", "from": claim.claim_status}))
            .next("get_claim"));
        }
        let lines = claim_lines(&mut tx, std::slice::from_ref(&claim.id)).await?;
        let order = sales::order(&mut tx, &claim.order_id).await?;
        let deliveries =
            repo::deliveries_of_orders(&mut tx, std::slice::from_ref(&claim.order_id)).await?;
        let recent = repo::recent_returns(
            &mut tx,
            &claim.customer_id,
            now - Duration::days(rules.flag_days),
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;

        let lines = lines.get(&claim.id).cloned().unwrap_or_default();
        let amount = refund_amount(&lines, rules.coupon_deducted).unwrap_or(claim.amount()?);
        let facts = EvalFacts {
            reason: claim.claim_reason.clone().unwrap_or_default(),
            liability: claim.liability.clone(),
            photos: photo_count(&claim.evidence),
            days_since_delivery: delivered_at(&deliveries, &claim.order_id)
                .map(|d| days_since(d, now, cx.time.zone)),
            refund_amount: amount,
            recent_returns: recent,
        };
        let s = evaluate(&rules, &cx.policy, &facts).ok_or_else(|| {
            ToolError::new(
                ErrorCode::INVALID_STATE,
                "No return rule applies yet: the order has not been delivered.",
            )
            .detail(json!({"entity": "claim", "from": claim.claim_status}))
            .next("get_claim")
        })?;
        let item = json!({
            "claim_id": claim.id,
            "order_number": order.map(|o| o.order_number),
            "suggested": s.suggested,
            "rule": s.rule,
            "rule_text": s.rule_text,
            "liability": s.liability,
            "refund_amount": s.refund_amount,
            "return_fees": s.return_fees,
            "needs_person": s.needs_person,
            "flags": s.flags,
        });
        Ok(json!({"items": [item], "total": 1}))
    }
}

/// How claims are ordered.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
pub enum ClaimSort {
    /// Newest first (the default).
    #[serde(rename = "-created_at")]
    NewestFirst,
    /// Largest amount first.
    #[serde(rename = "-amount")]
    AmountDescending,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchClaimsInput {
    /// Only this kind of claim.
    #[schemars(example = &"exchange")]
    pub claim_type: Option<ClaimType>,
    /// Only claims in these states.
    #[schemars(example = ["requested"])]
    pub status: Option<Vec<ClaimStatus>>,
    /// Only claims with this party at fault.
    #[schemars(example = &"customer_fault")]
    pub liability: Option<Liability>,
    /// Only claims on this order.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20447")]
    pub order_id: Option<String>,
    /// Only claims of this customer (operators; a shopper always sees only their own).
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_alex")]
    pub customer_id: Option<String>,
    /// Only claims sent at or after this time (store time).
    #[schemars(example = &"2026-11-01T00:00")]
    pub created_from: Option<String>,
    /// Only claims sent at or before this time (store time).
    #[schemars(example = &"2026-11-06T23:59")]
    pub created_through: Option<String>,
    /// -created_at: newest first (default); -amount: largest first.
    #[schemars(example = &"-created_at")]
    pub sort: Option<ClaimSort>,
    /// Top-level fields to return for each claim.
    #[schemars(example = ["id", "status", "amount"])]
    pub fields: Option<Vec<String>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchClaims;

impl Operation for SearchClaims {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.search",
        tool: "search_claims",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/claims",
        summary: "Search cancel, return and exchange claims by type, state, fault, order, customer and date. For the return review queue use search_return_requests.",
        consumes: &[],
        produces: &[Slot::many("claim_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchClaimsInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchClaims {
    async fn read(&self, cx: &Cx, input: SearchClaimsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let from = parse_time(&cx.time, "created_from", input.created_from.as_deref())?;
        let through = parse_time(
            &cx.time,
            "created_through",
            input.created_through.as_deref(),
        )?;
        if let (Some(f), Some(t)) = (from, through)
            && f > t
        {
            return Err(ToolError::new(
                INVALID_FILTER,
                "`created_from` is after `created_through`.",
            )
            .detail(json!({"field": "created_from", "reason": "range"}))
            .next("search_claims"));
        }
        let me = shopper(&cx.principal);
        if let (Some(me), Some(asked)) = (&me, &input.customer_id)
            && me != asked
        {
            return Ok(json!({"items": [], "total": 0}));
        }
        let filter = ClaimFilter {
            claim_type: input.claim_type.map(|t| t.as_str().to_owned()),
            statuses: input
                .status
                .unwrap_or_default()
                .iter()
                .map(|s| s.as_str().to_owned())
                .collect(),
            liability: input.liability.map(|l| l.as_str().to_owned()),
            order_id: input.order_id,
            customer_id: me.or(input.customer_id),
            from,
            through,
            ..ClaimFilter::default()
        };
        let order = match input.sort.unwrap_or(ClaimSort::NewestFirst) {
            ClaimSort::NewestFirst => ClaimOrder::Newest,
            ClaimSort::AmountDescending => ClaimOrder::Largest,
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (total, rows) =
            repo::search_claims(&mut tx, &filter, order, page.limit, page.offset).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let fields = input.fields.unwrap_or_default();
        let items = rows
            .iter()
            .map(|c| {
                Ok(select(
                    json!({"id": c.id, "claim_type": c.claim_type, "status": c.claim_status,
                           "order_id": c.order_id, "customer_id": c.customer_id,
                           "reason": c.reason(), "liability": c.liability,
                           "amount": c.amount()?, "created_at": cx.time.format(c.start_time)}),
                    &fields,
                ))
            })
            .collect::<Result<Vec<_>, ToolError>>()?;
        let mut out = json!({"items": items, "total": total});
        if let Some(next) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(next);
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetClaimInput {
    /// The cancel, return or exchange claim.
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_20402")]
    pub claim_id: String,
    /// Top-level fields to return, for example only the status and its history.
    #[schemars(example = ["status", "status_history"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetClaim;

impl Operation for GetClaim {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.get",
        tool: "get_claim",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/claims/{claim_id}",
        summary: "One claim (cancel, return or exchange): items, reason, fault, evidence, inspection, linked refund, and its status history.",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::one("refund_id")],
        checks: &[],
        errors: &[CLAIM_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetClaimInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetClaim {
    async fn read(&self, cx: &Cx, input: GetClaimInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = visible_claim(&mut tx, &input.claim_id, "search_claims").await?;
        let items = repo::claim_items(&mut tx, std::slice::from_ref(&c.id)).await?;
        let inspection = repo::inspection(&mut tx, &c.id).await?;
        let refund = repo::refund_of_claim(&mut tx, &c.id).await?;
        let events = repo::claim_events(&mut tx, &c.id).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let evidence: Vec<Value> = c
            .evidence
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|e| json!({"id": e.get("id"), "evidence_type": e.get("evidence_type")}))
                    .collect()
            })
            .unwrap_or_default();
        let history: Vec<Value> = events
            .iter()
            .filter_map(|e| {
                e.to_state
                    .as_ref()
                    .map(|s| json!({"status": s, "at": cx.time.format(e.start_time)}))
            })
            .collect();
        let refund_estimate = match (c.refund_estimate_value, &c.refund_estimate_currency) {
            (Some(v), Some(cur)) => Some(repo::money(v, cur)?),
            _ => None,
        };
        let out = json!({
            "id": c.id,
            "claim_type": c.claim_type,
            "status": c.claim_status,
            "order_id": c.order_id,
            "customer_id": c.customer_id,
            "items": items.iter().map(|i| {
                let mut v = json!({"order_item_id": i.order_item_id, "quantity": i.order_quantity});
                if let Some(r) = &i.replacement_product_id {
                    v["replacement_variant_id"] = json!(r);
                }
                v
            }).collect::<Vec<_>>(),
            "reason": c.reason(),
            "reason_text": c.reason_text,
            "liability": c.liability,
            "claim_path": c.claim_path,
            "return_fees": c.return_fees,
            "amount": c.amount()?,
            "refund_estimate": refund_estimate,
            "send_back_method": c.send_back_method,
            "pickup_window": c.pickup_window,
            "evidence": evidence,
            "inspection": inspection.map(|i| json!({
                "id": i.id, "condition": i.inspection_condition, "liability": i.liability,
                "note": i.description, "at": cx.time.format(i.end_time)})),
            "refund_id": refund.map(|r| r.id),
            "requested_at": cx.time.format(c.start_time),
            "closed_at": c.end_time.map(|t| cx.time.format(t)),
            "status_history": history,
        });
        Ok(select(out, &input.fields.unwrap_or_default()))
    }
}
