//! `refund_return` (money back for one return claim; a person approves over
//! `approval.yml thresholds.refund`) and `get_refund`.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, ReadOp, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_core::Money;
use acom_core::ids::RefundId;
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use chrono::Duration;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::claim::{claim_lines, claim_not_found, visible_claim};
use crate::errors::{
    ALREADY_REFUNDED, AMOUNT_OVER_PAYMENT, CLAIM_NOT_FOUND, COUPON_NOT_DEDUCTED, NOT_A_RETURN,
    NOT_REVOCABLE, PAYMENT_GATEWAY_ERROR, REFUND_NOT_FOUND,
};
use crate::model::returns::{refund_amount, refund_limit};
use crate::model::{money_short, money_text};
use crate::repo::{self, ClaimEvent, NewRefund};
use crate::{broken, original_payment, payment_label, rules, shopper};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefundReturnInput {
    /// The return claim to refund (claim_type return, status requested), from
    /// `search_return_requests` or `evaluate_return_request`. The amount is computed from it.
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_20402")]
    pub claim_id: String,
}

/// Everything the run needs, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefundPlan {
    pub refund_id: String,
    pub claim_id: String,
    pub order_id: String,
    pub payment_id: Option<String>,
    pub amount: Money,
    pub refund_method: String,
    pub refund_type: String,
    /// `card ending 0937`.
    pub to: String,
    /// The claim status the refund moves from, and to.
    pub from_status: String,
    pub to_status: String,
}

pub struct RefundReturn;

impl Operation for RefundReturn {
    const META: OpMeta = OpMeta {
        address: "fulfillment.refund.create",
        tool: "refund_return",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/fulfillment/refunds",
        summary: "Refund one return claim to the original payment. The server computes the amount from the claim (coupon share taken off); over the approval limit a person approves it in a change set. A refund cannot be undone.",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::one("refund_id")],
        checks: &["refund.to", "refund.coupon_deducted"],
        errors: &[
            CLAIM_NOT_FOUND,
            NOT_A_RETURN,
            ErrorCode::INVALID_STATE,
            ALREADY_REFUNDED,
            AMOUNT_OVER_PAYMENT,
            COUPON_NOT_DEDUCTED,
            PAYMENT_GATEWAY_ERROR,
        ],
        audience: Audience::Operator,
        action_type: ActionType::PAY,
        person_scope: "",
    };
    type Input = RefundReturnInput;
    type Output = Value;
}

/// The claim status a refund starts from and moves to (`refund.timing.return`): on approval
/// it is a waiting request that becomes approved; after inspection the inspected claim keeps
/// its status until the payment event completes it.
fn timing(policy: &acom_policy::PolicySet) -> (&'static str, &'static str) {
    match policy
        .lookup("fulfillment.refund.timing.return")
        .and_then(Value::as_str)
    {
        Some("after_inspection") => ("inspected", "inspected"),
        _ => ("requested", "approved"),
    }
}

impl WriteOp<Cx> for RefundReturn {
    type Plan = RefundPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RefundReturnInput,
    ) -> Result<Planned<RefundPlan>, ToolError> {
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
        if let Some(r) = repo::refund_of_claim(&mut tx, &claim.id).await? {
            return Err(ToolError::new(
                ALREADY_REFUNDED,
                format!("Claim {} was already refunded ({}).", claim.id, r.id),
            )
            .detail(json!({"entity": "refund", "existing_id": r.id}))
            .next("get_refund"));
        }
        let (from, to) = timing(&cx.policy);
        if claim.claim_status != from {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "Return {} is {}; only a {} return can be refunded.",
                    claim.id,
                    claim.claim_status.replace('_', " "),
                    from
                ),
            )
            .detail(json!({"entity": "claim", "from": claim.claim_status, "allowed_to": [to]}))
            .next("get_claim"));
        }
        let lines = claim_lines(&mut tx, std::slice::from_ref(&claim.id)).await?;
        let lines = lines.get(&claim.id).cloned().unwrap_or_default();
        let order = sales::order(&mut tx, &claim.order_id)
            .await?
            .ok_or_else(|| broken(format!("claim {} has no order", claim.id)))?;
        let payments = sales::payments(&mut tx, &claim.order_id).await?;
        let earlier = repo::refunds_on_order(&mut tx, &claim.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let amount = refund_amount(&lines, rules.coupon_deducted)
            .ok_or_else(|| broken(format!("claim {} has no lines", claim.id)))?;
        let paid: Decimal = payments
            .iter()
            .filter(|p| p.payment_status == "payment_complete")
            .map(|p| p.amount.value)
            .sum();
        let mut refunded = Decimal::ZERO;
        for r in &earlier {
            refunded += r.amount()?.value;
        }
        if amount.value > paid - refunded {
            return Err(ToolError::new(
                AMOUNT_OVER_PAYMENT,
                format!(
                    "{} is more than what is left of the payment on order {}.",
                    money_text(amount),
                    order.order_number
                ),
            )
            .detail(
                json!({"limit": Money::new(paid - refunded, amount.currency), "actual": amount}),
            )
            .next("evaluate_return_request"));
        }
        let payment = original_payment(&payments);
        let to_text = payment
            .map(payment_label)
            .unwrap_or_else(|| "the original payment".to_owned());

        let mut checks = Vec::new();
        let reversible = payment.is_none_or(|p| {
            match (rules.reversal_window_days.get(&p.payment_method), p.paid_at) {
                (Some(Some(days)), Some(at)) => at + Duration::days(*days) >= now,
                _ => true,
            }
        });
        checks.push(Check {
            rule: "refund.to".into(),
            passed: reversible,
            detail: Some(if reversible {
                format!("{} back to {}", money_text(amount), to_text)
            } else {
                format!(
                    "The payment to {to_text} is too old to reverse; a person must pick where the money goes"
                )
            }),
            fix_tool: None,
        });
        let share: Decimal = lines.iter().map(|l| l.discount_share().value).sum();
        let coupon = order.discount_code.clone().or(order.coupon_id.clone());
        checks.push(Check {
            rule: "refund.coupon_deducted".into(),
            passed: true,
            detail: Some(match (&coupon, rules.coupon_deducted, share.is_zero()) {
                (None, _, _) => format!("No coupon on order {}", order.order_number),
                (Some(code), true, false) => format!(
                    "Coupon {code}: {} taken off the refund",
                    money_text(Money::new(share, amount.currency))
                ),
                (Some(code), true, true) => format!(
                    "Coupon {code} on order {} applies to items that stay; nothing to take off",
                    order.order_number
                ),
                (Some(code), false, _) => {
                    format!("Coupon {code} is refunded too (the policy does not take it off)")
                }
            }),
            fix_tool: None,
        });

        let approval = cx
            .policy
            .approval_for(Self::META.tool, &json!({"amount": amount}));
        let approval = if approval.needed {
            ApprovalNeed::Needed {
                reason: match refund_limit(&cx.policy) {
                    Some(limit) => format!(
                        "Refund {} is over the {} limit for refunds without a person",
                        money_text(amount),
                        money_text(limit)
                    ),
                    None => approval
                        .reason
                        .unwrap_or_else(|| format!("Refund {}", money_short(amount))),
                },
            }
        } else {
            ApprovalNeed::NotNeeded
        };
        let plan = RefundPlan {
            refund_id: RefundId::generate().to_string(),
            claim_id: claim.id.clone(),
            order_id: claim.order_id.clone(),
            payment_id: payment.map(|p| p.id.clone()),
            amount,
            refund_method: rules.refund_to.clone(),
            refund_type: rules.defect_refund_type.clone(),
            to: to_text.clone(),
            from_status: from.to_owned(),
            to_status: to.to_owned(),
        };
        let mut changes = vec![Change {
            op: ChangeOp::Create,
            entity: "refund".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"claim_id": claim.id, "amount": amount,
                       "refund_method": plan.refund_method, "to": to_text}),
            note: None,
        }];
        if from != to {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "claim".into(),
                id: claim.id.clone(),
                from: json!(from),
                to: json!(to),
                note: None,
            });
        }
        Ok(Planned {
            plan,
            changes,
            checks,
            basis: vec![Basis {
                entity: "fulfillment.claim".into(),
                id: claim.id,
                version: i64::from(claim.version),
            }],
            approval,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &RefundPlan) -> Result<Committed, ToolError> {
        let claim = repo::claim_for_update(tx, &plan.claim_id)
            .await?
            .ok_or_else(|| claim_not_found(&plan.claim_id, "search_return_requests"))?;
        let done = || Committed {
            id: plan.refund_id.clone(),
            produced: Map::from_iter([
                ("refund_id".to_owned(), json!(plan.refund_id)),
                ("claim_id".to_owned(), json!(plan.claim_id)),
            ]),
        };
        if let Some(r) = repo::refund_of_claim(tx, &plan.claim_id).await? {
            if r.id == plan.refund_id {
                return Ok(done());
            }
            return Err(ToolError::new(
                ALREADY_REFUNDED,
                format!("Claim {} was already refunded ({}).", plan.claim_id, r.id),
            )
            .detail(json!({"entity": "refund", "existing_id": r.id}))
            .next("get_refund"));
        }
        if claim.claim_status != plan.from_status {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("Return {} is now {}.", claim.id, claim.claim_status),
            )
            .detail(json!({"entity": "claim", "from": claim.claim_status}))
            .next("get_claim"));
        }
        repo::insert_refund(
            tx,
            &NewRefund {
                id: &plan.refund_id,
                claim_id: &plan.claim_id,
                order_id: &plan.order_id,
                payment_id: plan.payment_id.as_deref(),
                amount: plan.amount,
                refund_method: &plan.refund_method,
                refund_type: &plan.refund_type,
            },
        )
        .await?;
        if plan.from_status != plan.to_status
            && !repo::move_claim(tx, &plan.claim_id, &plan.from_status, &plan.to_status).await?
        {
            return Err(broken(format!("claim {} did not move", plan.claim_id)));
        }
        repo::append_event(
            tx,
            &plan.claim_id,
            &ClaimEvent {
                action_type: "RefundAction",
                action_status: "completed",
                instrument: Self::META.tool,
                from_state: Some(&plan.from_status),
                to_state: Some(&plan.to_status),
                result: Some(json!({"refund_id": plan.refund_id, "amount": plan.amount})),
            },
        )
        .await?;
        Ok(done())
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RefundPlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let refund = repo::refund(&mut tx, &done.id).await?;
        let claim = repo::claim(&mut tx, &plan.claim_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match refund {
            None => mismatches.push(format!("refund {} is missing", done.id)),
            Some(r) => {
                if r.amount()? != plan.amount {
                    mismatches.push(format!(
                        "refund {} is {}, planned {}",
                        r.id,
                        money_text(r.amount()?),
                        money_text(plan.amount)
                    ));
                }
                if r.claim_id != plan.claim_id {
                    mismatches.push(format!("refund {} is for {}", r.id, r.claim_id));
                }
            }
        }
        match claim {
            Some(c) if c.claim_status == plan.to_status || !c.is_open() => {}
            Some(c) => mismatches.push(format!("claim {} is {}", c.id, c.claim_status)),
            None => mismatches.push(format!("claim {} is missing", plan.claim_id)),
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, _: &mut TxCx, plan: &RefundPlan, _: &Committed) -> Result<(), ToolError> {
        Err(ToolError::new(
            NOT_REVOCABLE,
            format!(
                "Refund {} has been sent and cannot be taken back.",
                plan.refund_id
            ),
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetRefundInput {
    /// The refund.
    #[schemars(regex(pattern = r"^rfd_[a-z0-9_]+$"), example = &"rfd_20402")]
    pub refund_id: String,
    /// Top-level fields to return, for example only the status.
    #[schemars(example = ["refund_status", "events"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetRefund;

impl Operation for GetRefund {
    const META: OpMeta = OpMeta {
        address: "fulfillment.refund.get",
        tool: "get_refund",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/refunds/{refund_id}",
        summary: "One refund: amount, where the money goes, and its status as the payment provider confirms it (requested, completed, failed).",
        consumes: &[Slot::one("refund_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[],
        errors: &[REFUND_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetRefundInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetRefund {
    async fn read(&self, cx: &Cx, input: GetRefundInput) -> Result<Value, ToolError> {
        let not_found = || {
            ToolError::new(
                REFUND_NOT_FOUND,
                format!("There is no refund {}.", input.refund_id),
            )
            .detail(json!({"entity": "refund", "id": input.refund_id}))
            .next("search_claims")
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::refund(&mut tx, &input.refund_id)
            .await?
            .ok_or_else(not_found)?;
        if let Some(me) = shopper(&cx.principal) {
            let owner = repo::claim(&mut tx, &r.claim_id)
                .await?
                .map(|c| c.customer_id);
            if owner.as_deref() != Some(me.as_str()) {
                return Err(not_found());
            }
        }
        let payments = sales::payments(&mut tx, &r.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let to = match r.refund_method.as_str() {
            "original_payment" => r
                .payment_id
                .as_ref()
                .and_then(|id| payments.iter().find(|p| &p.id == id))
                .map(payment_label)
                .unwrap_or_else(|| "the original payment".to_owned()),
            other => other.replace('_', " "),
        };
        let mut events =
            vec![json!({"refund_status": "requested", "at": cx.time.format(r.start_time)})];
        if let Some(end) = r.end_time
            && r.refund_status != "requested"
        {
            events.push(json!({"refund_status": r.refund_status, "at": cx.time.format(end)}));
        }
        let out = json!({
            "id": r.id,
            "claim_id": r.claim_id,
            "order_id": r.order_id,
            "amount": r.amount()?,
            "refund_method": r.refund_method,
            "refund_type": r.refund_type,
            "to": to,
            "refund_status": r.refund_status,
            "events": events,
        });
        Ok(crate::select(out, &input.fields.unwrap_or_default()))
    }
}
