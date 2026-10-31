//! `pay_remedy`: pay an approved remedy. Cash goes to the original payment method; points and
//! coupons are granted by promotion's `grant_loyalty` and `issue_coupon`. Both are stubs from
//! here (no payment provider; promotion cannot be called from customer), so the payout record
//! says what was meant to happen and that nothing left the store, and names the tool that grants
//! points or coupons. A payment cannot be taken back.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{RemedyRow, RemedyState, dollars, remedy, remedy_not_found, write_state};
use crate::access::{always, policy_words, staff_only};
use crate::{PAYOUT_NOT_APPROVED, REMEDY_NOT_FOUND, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PayRemedyInput {
    /// The approved remedy (`approve_remedy`).
    #[schemars(regex(pattern = r"^rmd_[a-z0-9_]+$"), example = &"rmd_310")]
    pub remedy_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayPlan {
    pub remedy_id: String,
    pub before: RemedyState,
    /// What the payout record will say.
    pub payout: Value,
}

pub struct PayRemedy;

impl Operation for PayRemedy {
    const META: OpMeta = OpMeta {
        address: "customer.remedy.pay",
        tool: "pay_remedy",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/customer/remedies/{remedy_id}/pay",
        summary: "Pay an approved remedy: cash to the original payment method, points or a coupon to the customer. Moves money, so a person always approves; it cannot be undone.",
        consumes: &[Slot::one("remedy_id")],
        produces: &[Slot::one("remedy_id")],
        checks: &["remedy.approval_levels", "remedy.cash_requires"],
        errors: &[PAYOUT_NOT_APPROVED, REMEDY_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::PAY,
        person_scope: "",
    };
    type Input = PayRemedyInput;
    type Output = Value;
}

/// The payout record of a remedy: where it goes and that it was a stub.
pub(crate) fn payout_for(r: &RemedyRow, card_last4: Option<&str>) -> Value {
    let amount = r.amount();
    match r.remedy_type.as_str() {
        "cash" => json!({
            "method": "original_payment",
            "to": card_last4.map(|l| format!("card ending {l}")),
            "amount": amount,
            "reference": format!("stub_{}", r.id),
            "delivered": false,
            "note": "The payment provider is a stub: no money moved."}),
        "coupon" => json!({
            "method": "coupon",
            "amount": amount,
            "delivered": false,
            "next_tool": "issue_coupon",
            "note": "Coupons are issued by promotion (issue_coupon), which customer cannot call; recorded here, not issued."}),
        _ => json!({
            "method": "loyalty_points",
            "amount": amount,
            "delivered": false,
            "next_tool": "grant_loyalty",
            "note": "Points are granted by promotion (grant_loyalty), which customer cannot call; recorded here, not granted."}),
    }
}

impl WriteOp<Cx> for PayRemedy {
    type Plan = PayPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(&self, cx: &Cx, input: PayRemedyInput) -> Result<Planned<PayPlan>, ToolError> {
        staff_only(cx)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = remedy(&mut tx, &input.remedy_id)
            .await?
            .ok_or_else(|| remedy_not_found(&input.remedy_id))?;
        let person = match &r.customer_id {
            Some(c) => repo::person(&mut tx, c).await?,
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;
        if r.remedy_status != "approved" {
            return Err(ToolError::new(
                PAYOUT_NOT_APPROVED,
                format!(
                    "{} is {}; only an approved remedy is paid.",
                    r.id, r.remedy_status
                ),
            )
            .detail(
                json!({"entity": "remedy", "from": r.remedy_status, "allowed_from": ["approved"]}),
            )
            .next("approve_remedy"));
        }
        let amount = r.amount();
        let card = person.as_ref().and_then(|p| p.card_last4.clone());
        let payout = payout_for(&r, card.as_deref());
        let cash = r.remedy_type == "cash";
        let mut changes = Vec::new();
        let mut checks = vec![Check {
            rule: "remedy.approval_levels".into(),
            passed: true,
            detail: Some(format!(
                "Approved by {}{}",
                r.decided_by_id.as_deref().unwrap_or("a person"),
                if cash {
                    "; the payout approver signs off on this payment"
                } else {
                    ""
                }
            )),
            fix_tool: Some("approve_remedy".into()),
        }];
        if cash {
            let requires = policy_words(cx, "customer.remedy.cash_requires");
            let account_needed = requires.iter().any(|x| x == "verified_refund_account");
            let has_account = card.is_some();
            checks.push(Check {
                rule: "remedy.cash_requires".into(),
                passed: has_account || !account_needed,
                detail: Some(match &card {
                    Some(l) => format!("Card ending {l} on file; the payout approver signs off"),
                    None => "No payment method on file to pay back to".to_owned(),
                }),
                fix_tool: None,
            });
            changes.push(Change {
                op: ChangeOp::Create,
                entity: "money_transfer".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"amount": amount, "to": payout["to"]}),
                note: Some("Payment provider stub: no money moves".into()),
            });
        } else {
            let entity = if r.remedy_type == "coupon" {
                "member_coupon"
            } else {
                "loyalty_transaction"
            };
            changes.push(Change {
                op: ChangeOp::Grant,
                entity: entity.into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"amount": amount, "customer_id": r.customer_id}),
                note: payout["note"].as_str().map(str::to_owned),
            });
        }
        changes.push(Change {
            op: ChangeOp::Update,
            entity: "remedy".into(),
            id: r.id.clone(),
            from: json!("approved"),
            to: json!("paid"),
            note: cash.then(|| "Through payout_pending; the stub confirms at once".to_owned()),
        });
        let reason = match (&card, cash) {
            (Some(l), true) => format!("Pays {} in cash to card ending {l}", dollars(amount)),
            (None, true) => format!("Pays {} in cash", dollars(amount)),
            (_, false) => format!(
                "Gives {} in {}",
                dollars(amount),
                r.remedy_type.replace('_', " ")
            ),
        };
        let facts = json!({"remedy_type": r.remedy_type, "amount": amount});
        Ok(Planned {
            changes,
            checks,
            basis: vec![Basis {
                entity: "customer.remedy".into(),
                id: r.id.clone(),
                version: i64::from(r.version),
            }],
            approval: always(cx, Self::META.tool, &facts, reason),
            plan: PayPlan {
                remedy_id: r.id.clone(),
                before: RemedyState::from(&r),
                payout,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &PayPlan) -> Result<Committed, ToolError> {
        let r = remedy(tx, &plan.remedy_id)
            .await?
            .ok_or_else(|| remedy_not_found(&plan.remedy_id))?;
        let mut produced = Map::new();
        produced.insert("remedy_id".into(), json!(plan.remedy_id));
        match r.remedy_status.as_str() {
            // Paid already: the first result, never a second payment.
            "paid" => {
                produced.insert("payout".into(), r.payout.unwrap_or(Value::Null));
            }
            "approved" => {
                let after = RemedyState {
                    status: "paid".into(),
                    paid_at: Some(tx.clock.now()),
                    payout: Some(plan.payout.clone()),
                    ..plan.before.clone()
                };
                write_state(tx, &plan.remedy_id, &after).await?;
                produced.insert("payout".into(), plan.payout.clone());
            }
            other => {
                return Err(ToolError::new(
                    PAYOUT_NOT_APPROVED,
                    format!("{} is {other}; only an approved remedy is paid.", r.id),
                )
                .next("approve_remedy"));
            }
        }
        Ok(Committed {
            id: plan.remedy_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &PayPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = remedy(&mut tx, &plan.remedy_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            Some(r) if r.remedy_status == "paid" => vec![],
            Some(r) => vec![format!("{} is {}", r.id, r.remedy_status)],
            None => vec![format!("{} is gone", plan.remedy_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        _plan: &PayPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        // Not reached: a payment cannot be taken back (UNDO is NotSupported).
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_moves_money_so_it_cannot_be_undone() {
        let s = closed_schema_for::<PayRemedyInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(PayRemedy::META.kind, Kind::WriteApproval);
        assert_eq!(PayRemedy::UNDO, UndoSupport::NotSupported);
    }
}
