//! `retry_refund`: send a failed refund to the payment provider again. Money moves, so a person
//! always approves it.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::errors::{MAX_RETRIES_REACHED, NOT_FAILED, NOT_REVOCABLE, REFUND_NOT_FOUND};
use crate::model::money_text;
use crate::repo::{self, ClaimEvent, RefundRow};
use crate::{broken, payment_label};

/// What the payment provider is in this system: a stub. Said in every retry's record.
pub(crate) const PROVIDER_NOTE: &str = "No payment provider is connected in this demo: the refund is recorded as sent again, and a payment event confirms or fails it.";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RetryRefundInput {
    /// The failed refund, from `get_claim` or `get_refund`.
    #[schemars(regex(pattern = r"^rfd_[a-z0-9_]+$"), example = &"rfd_20377")]
    pub refund_id: String,
}

/// The retry, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryPlan {
    pub refund_id: String,
    pub claim_id: String,
    /// This attempt's number; the first sending was attempt 1.
    pub attempt: i64,
    pub max_retries: i64,
    pub amount: Money,
    pub to: String,
}

pub struct RetryRefund;

impl Operation for RetryRefund {
    const META: OpMeta = OpMeta {
        address: "fulfillment.refund.retry",
        tool: "retry_refund",
        group: Group::Transact,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/fulfillment/refunds/{refund_id}/retry",
        summary: "Send a failed refund to the payment provider again (failed → requested), up to the policy's number of attempts. Money moves, so a person approves it; it cannot be undone.",
        consumes: &[Slot::one("refund_id")],
        produces: &[Slot::one("refund_id")],
        checks: &["refund.to", "refund.max_retries"],
        errors: &[REFUND_NOT_FOUND, NOT_FAILED, MAX_RETRIES_REACHED],
        audience: Audience::Operator,
        action_type: ActionType::PAY,
        person_scope: "",
    };
    type Input = RetryRefundInput;
    type Output = Value;
}

fn not_failed(r: &RefundRow) -> ToolError {
    ToolError::new(
        NOT_FAILED,
        format!(
            "Refund {} is {}, not failed; only a failed refund is sent again.",
            r.id, r.refund_status
        ),
    )
    .detail(json!({"entity": "refund", "from": r.refund_status, "allowed_to": []}))
    .next("get_refund")
}

/// Attempts so far: the first sending and every retry recorded.
fn attempts_so_far(events: &[repo::EventRow]) -> i64 {
    1 + events
        .iter()
        .filter(|e| e.instrument.as_deref() == Some(RetryRefund::META.tool))
        .count() as i64
}

impl WriteOp<Cx> for RetryRefund {
    type Plan = RetryPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RetryRefundInput,
    ) -> Result<Planned<RetryPlan>, ToolError> {
        let max_retries = cx
            .policy
            .lookup("fulfillment.refund.max_retries")
            .and_then(Value::as_i64)
            .ok_or_else(|| broken("The store's policy has no usable `refund.max_retries`."))?;
        let sla = cx
            .policy
            .lookup("fulfillment.refund.manual_queue_sla_hours")
            .and_then(Value::as_i64);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::refund(&mut tx, &input.refund_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    REFUND_NOT_FOUND,
                    format!("There is no refund {}.", input.refund_id),
                )
                .detail(json!({"entity": "refund", "id": input.refund_id}))
                .next("search_claims")
            })?;
        let events = repo::refund_events(&mut tx, &r.id).await?;
        let payments = sales::payments(&mut tx, &r.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if r.refund_status != "failed" {
            return Err(not_failed(&r));
        }
        let done = attempts_so_far(&events);
        if done >= max_retries {
            return Err(ToolError::new(
                MAX_RETRIES_REACHED,
                format!(
                    "Refund {} was tried {done} times, the most the policy allows; a person handles it from the manual queue{}.",
                    r.id,
                    sla.map(|h| format!(" within {h} hours")).unwrap_or_default()
                ),
            )
            .detail(json!({"limit": max_retries, "actual": done,
                           "policy_key": "refund.max_retries"}))
            .next("get_refund"));
        }
        let amount = r.amount()?;
        let to = match r.refund_method.as_str() {
            "original_payment" => r
                .payment_id
                .as_ref()
                .and_then(|id| payments.iter().find(|p| &p.id == id))
                .map(payment_label)
                .unwrap_or_else(|| "the original payment".to_owned()),
            other => other.replace('_', " "),
        };
        let attempt = done + 1;
        Ok(Planned {
            plan: RetryPlan {
                refund_id: r.id.clone(),
                claim_id: r.claim_id.clone(),
                attempt,
                max_retries,
                amount,
                to: to.clone(),
            },
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "refund".into(),
                id: r.id.clone(),
                from: json!("failed"),
                to: json!("requested"),
                note: Some(PROVIDER_NOTE.into()),
            }],
            checks: vec![
                Check {
                    rule: "refund.to".into(),
                    passed: true,
                    detail: Some(format!("{} back to {to}", money_text(amount))),
                    fix_tool: None,
                },
                Check {
                    rule: "refund.max_retries".into(),
                    passed: true,
                    detail: Some(format!("Attempt {attempt} of {max_retries}")),
                    fix_tool: None,
                },
            ],
            basis: Vec::new(),
            approval: ApprovalNeed::Needed {
                reason: format!("Retries a {} refund", money_text(amount)),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &RetryPlan) -> Result<Committed, ToolError> {
        let done = Committed {
            id: plan.refund_id.clone(),
            produced: Map::from_iter([
                ("refund_id".to_owned(), json!(plan.refund_id)),
                ("claim_id".to_owned(), json!(plan.claim_id)),
            ]),
        };
        let r = repo::refund_for_update(tx, &plan.refund_id)
            .await?
            .ok_or_else(|| broken(format!("refund {} is missing", plan.refund_id)))?;
        let events = repo::refund_events(tx, &r.id).await?;
        let already = events.iter().any(|e| {
            e.instrument.as_deref() == Some(Self::META.tool)
                && e.result
                    .as_ref()
                    .and_then(|v| v.get("attempt"))
                    .and_then(Value::as_i64)
                    == Some(plan.attempt)
        });
        if already {
            return Ok(done);
        }
        if r.refund_status != "failed" {
            return Err(not_failed(&r));
        }
        let failed_at = r.end_time.map(|t| tx.time.format(t));
        repo::move_refund(tx, &r.id, "failed", "requested").await?;
        repo::append_refund_event(
            tx,
            &r.id,
            &ClaimEvent {
                action_type: "PayAction",
                action_status: "active",
                instrument: Self::META.tool,
                from_state: Some("failed"),
                to_state: Some("requested"),
                result: Some(
                    json!({"attempt": plan.attempt, "max_retries": plan.max_retries,
                                    "failed_at": failed_at, "payment_provider": PROVIDER_NOTE}),
                ),
            },
        )
        .await?;
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RetryPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::refund(&mut tx, &plan.refund_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match r {
            None => vec![format!("refund {} is missing", plan.refund_id)],
            Some(r) if r.refund_status != "failed" => Vec::new(),
            Some(r) => vec![format!("refund {} is still failed", r.id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, _: &mut TxCx, plan: &RetryPlan, _: &Committed) -> Result<(), ToolError> {
        Err(ToolError::new(
            NOT_REVOCABLE,
            format!(
                "Refund {} was sent again and cannot be taken back.",
                plan.refund_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retry() -> repo::EventRow {
        repo::EventRow {
            instrument: Some("retry_refund".into()),
            to_state: Some("requested".into()),
            result: None,
            start_time: chrono::Utc::now(),
        }
    }

    #[test]
    fn the_first_sending_counts_as_an_attempt() {
        assert_eq!(attempts_so_far(&[]), 1);
        assert_eq!(attempts_so_far(&[retry(), retry()]), 3);
    }
}
