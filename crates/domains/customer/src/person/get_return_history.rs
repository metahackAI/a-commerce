//! `get_return_history`.

use super::customer_not_found;
use crate::access::{policy_number, staff_only};
use crate::service::providers;
use crate::{CUSTOMER_NOT_FOUND, repo};
use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::{Cx, Page};
use chrono::Duration;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetReturnHistoryInput {
    /// The customer (`search_return_requests` gives it).
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_4431")]
    pub customer_id: String,
    /// Days back from today (store clock), 1–365. Default: the return history flag's days (90).
    #[schemars(range(min = 1, max = 365), example = 90)]
    pub days: Option<u32>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct GetReturnHistory;

impl Operation for GetReturnHistory {
    const META: OpMeta = OpMeta {
        address: "customer.person.get_return_history",
        tool: "get_return_history",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/{customer_id}/returns",
        summary: "A customer's return requests in the last days, with a summary that flags frequent returners (for example 4 returns in 90 days).",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::many("claim_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetReturnHistoryInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetReturnHistory {
    async fn read(&self, cx: &Cx, input: GetReturnHistoryInput) -> Result<Value, ToolError> {
        staff_only(cx)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let flag_days = policy_number(
            cx,
            "fulfillment.return_suggestions.flag_return_history.days",
        )?;
        let threshold = policy_number(
            cx,
            "fulfillment.return_suggestions.flag_return_history.returns",
        )?;
        let days = input.days.map_or(flag_days, i64::from);
        if !(1..=365).contains(&days) {
            return Err(ToolError::invalid_input("`days` must be 1–365.")
                .detail(json!({"field": "days", "reason": "range"}))
                .next("get_return_history"));
        }
        let reader = providers::return_claims().ok_or_else(|| {
            ToolError::new(
                ErrorCode::UPSTREAM_UNAVAILABLE,
                "Return history is not connected to fulfillment yet.",
            )
            .detail(json!({"upstream": "fulfillment", "retryable": false}))
        })?;
        let since = cx.clock.now() - Duration::days(days);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if repo::person(&mut tx, &input.customer_id).await?.is_none() {
            return Err(customer_not_found(&input.customer_id));
        }
        let claims: Vec<_> = reader(&mut tx, &input.customer_id, since)
            .await?
            .into_iter()
            .filter(|c| c.status != "withdrawn")
            .collect();
        tx.commit().await.map_err(ToolError::from)?;
        let total = i64::try_from(claims.len()).unwrap_or(i64::MAX);
        let start = usize::try_from(page.offset).unwrap_or(usize::MAX);
        let take = usize::try_from(page.limit).unwrap_or(0);
        let items: Vec<Value> = claims
            .iter()
            .skip(start)
            .take(take)
            .map(|c| {
                json!({"id": c.id, "claim_type": c.claim_type, "order_id": c.order_id,
                       "order_number": (!c.order_number.is_empty()).then_some(&c.order_number),
                       "item": (!c.item.is_empty()).then_some(&c.item),
                       "amount": c.amount, "reason": c.reason, "status": c.status,
                       "requested_at": cx.time.format(c.requested_at)})
            })
            .collect();
        let mut out = json!({"items": items, "total": total,
            "summary": {"count": total, "days": days, "threshold": threshold,
                        "flagged": total >= threshold}});
        if let Some(c) = page.next_cursor(items.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_the_path_is_the_spec_path() {
        let s = closed_schema_for::<GetReturnHistoryInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            GetReturnHistory::META.path,
            "/v1/customer/people/{customer_id}/returns"
        );
    }
}
