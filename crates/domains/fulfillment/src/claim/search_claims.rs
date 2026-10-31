//! `search_claims`: cancel, return and exchange claims by type, state, fault, order and date.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::{ClaimStatus, ClaimType, Liability};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::errors::INVALID_FILTER;
use crate::repo::{self, ClaimFilter, ClaimOrder};
use crate::{parse_time, select, shopper};

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
