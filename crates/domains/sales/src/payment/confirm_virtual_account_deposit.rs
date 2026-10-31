//! `confirm_virtual_account_deposit` (`sales.payment.confirm_deposit`): an operator confirms
//! that the money for a virtual-account order arrived. The payment completes and the order moves
//! from `payment_due` to `processing`, ready to ship. Money moves, so a person always approves
//! (W$). The bank is outside this system: nothing asks it; the operator's word is recorded, and
//! the output says so.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::Money;
use acom_core::enums::{OrderStatus, PaymentStatus};
use acom_store::{Cx, TxCx};
use chrono::Duration;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::PAYMENT_NOT_FOUND;
use crate::order::visible_order;
use crate::owner::{self, Viewer};
use crate::repo::{self, OrderRow, PaymentRow};
use crate::rules;
use crate::{AMOUNT_MISMATCH, DEADLINE_PASSED, ORDER_NOT_FOUND, approval, outside_system};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmVirtualAccountDepositInput {
    /// The order paid by virtual account (`search_orders`).
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20519")]
    pub order_id: String,
    /// The amount that arrived, as the bank statement shows it; it must be the payment's amount.
    #[schemars(example = json!({"value": 71.20, "currency": "USD"}))]
    pub amount: Money,
}

/// What confirming the deposit changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DepositPlan {
    order_id: String,
    payment_id: String,
    amount: Money,
}

pub(crate) struct ConfirmVirtualAccountDeposit;

impl Operation for ConfirmVirtualAccountDeposit {
    const META: OpMeta = OpMeta {
        address: "sales.payment.confirm_deposit",
        tool: "confirm_virtual_account_deposit",
        group: Group::Transact,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/sales/orders/{order_id}/payment/confirm-deposit",
        summary: "Record that the money for a virtual-account order arrived (operators): the payment completes and the order moves to processing. Always needs a person's approval; the bank is not asked.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("payment_id")],
        checks: &["payment.virtual_account_hold_days"],
        errors: &[
            DEADLINE_PASSED,
            AMOUNT_MISMATCH,
            ErrorCode::INVALID_STATE,
            ORDER_NOT_FOUND,
            PAYMENT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::PAY,
        person_scope: "",
    };
    type Input = ConfirmVirtualAccountDepositInput;
    type Output = Value;
}

fn invalid_state(message: String, from: &str, reason: &str) -> ToolError {
    ToolError::new(ErrorCode::INVALID_STATE, message)
        .detail(json!({"entity": "payment", "from": from, "reason": reason}))
        .next("get_payment")
}

/// Refuses a payment that is not a virtual account waiting for its money.
fn waiting_deposit(p: &PaymentRow) -> Result<(), ToolError> {
    if p.payment_method != "virtual_account" {
        return Err(invalid_state(
            format!(
                "Order {} is paid by {}, not by virtual account.",
                p.order_id,
                p.payment_method.replace('_', " ")
            ),
            &p.payment_status,
            "not_virtual_account",
        ));
    }
    if p.payment_status == PaymentStatus::PaymentComplete.as_str() {
        return Err(invalid_state(
            format!("The payment for order {} is complete already.", p.order_id),
            &p.payment_status,
            "already_complete",
        ));
    }
    if p.payment_status != PaymentStatus::PaymentDue.as_str() {
        return Err(invalid_state(
            format!(
                "The payment for order {} is {}.",
                p.order_id,
                p.payment_status.replace('_', " ")
            ),
            &p.payment_status,
            "not_due",
        ));
    }
    Ok(())
}

fn same(a: Money, b: Money) -> bool {
    a.currency == b.currency && a.value == b.value
}

impl WriteOp<Cx> for ConfirmVirtualAccountDeposit {
    type Plan = DepositPlan;
    // A confirmed payment releases the order to the warehouse; it is not taken back.
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ConfirmVirtualAccountDepositInput,
    ) -> Result<Planned<DepositPlan>, ToolError> {
        if owner::viewer(&cx.principal) != Viewer::Staff {
            return Err(
                ToolError::new(ErrorCode::FORBIDDEN, "Only the store confirms a deposit.")
                    .detail(json!({"reason": "role"}))
                    .next("get_payment"),
            );
        }
        let hold_days = rules::virtual_account_hold_days(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let o: OrderRow = visible_order(cx, &mut tx, &input.order_id).await?;
        let p = repo::payment(&mut tx, &o.id).await?.ok_or_else(|| {
            ToolError::new(PAYMENT_NOT_FOUND, format!("Order {} has no payment.", o.id))
                .detail(json!({"entity": "payment", "order_id": o.id}))
                .next("get_order")
        })?;
        let waits_for_recipient = repo::gift_order_of(&mut tx, &o.id)
            .await?
            .is_some_and(|g| g.action_status == "potential");
        tx.commit().await.map_err(ToolError::from)?;
        waiting_deposit(&p)?;
        if waits_for_recipient {
            return Err(invalid_state(
                "This gift waits for its recipient to accept it; its payment waits too.".into(),
                &p.payment_status,
                "waits_for_recipient",
            ));
        }
        let deadline = o.order_date + Duration::days(i64::from(hold_days));
        if cx.clock.now() > deadline {
            return Err(ToolError::new(
                DEADLINE_PASSED,
                format!(
                    "The deposit for order #{} was due by {}.",
                    o.order_number,
                    cx.time.format(deadline)
                ),
            )
            .detail(json!({"deadline": cx.time.format(deadline),
                           "policy_key": "payment.virtual_account_hold_days"}))
            .next("get_order"));
        }
        let due = repo::money(p.amount_value, &p.amount_currency)?;
        if !same(input.amount, due) {
            return Err(ToolError::new(
                AMOUNT_MISMATCH,
                format!(
                    "Order #{} is for {due}; the deposit of {} does not match it.",
                    o.order_number, input.amount
                ),
            )
            .detail(json!({"expected": due, "given": input.amount}))
            .next("get_payment"));
        }
        let facts = json!({"amount": due, "order_id": o.id});
        let need = match approval(cx, Self::META.tool, &facts) {
            ApprovalNeed::NotNeeded => ApprovalNeed::Needed {
                reason: format!("Confirms a {} {} payment", due.value, due.currency),
            },
            needed => needed,
        };
        Ok(Planned {
            changes: vec![
                Change {
                    op: ChangeOp::Update,
                    entity: "payment".into(),
                    id: p.id.clone(),
                    from: json!(PaymentStatus::PaymentDue),
                    to: json!(PaymentStatus::PaymentComplete),
                    note: Some(format!("{} {} arrived", due.value, due.currency)),
                },
                Change {
                    op: ChangeOp::Update,
                    entity: "order".into(),
                    id: o.id.clone(),
                    from: json!(o.order_status),
                    to: json!(OrderStatus::Processing),
                    note: Some("Paid, so the warehouse can ship it".into()),
                },
            ],
            checks: vec![Check {
                rule: "payment.virtual_account_hold_days".into(),
                passed: true,
                detail: Some(format!(
                    "Deposit within {hold_days} days (due by {})",
                    cx.time.format(deadline)
                )),
                fix_tool: None,
            }],
            basis: vec![
                Basis {
                    entity: "sales.payment".into(),
                    id: p.id.clone(),
                    version: i64::from(p.version),
                },
                Basis {
                    entity: "sales.sales_order".into(),
                    id: o.id.clone(),
                    version: i64::from(o.version),
                },
            ],
            approval: need,
            plan: DepositPlan {
                order_id: o.id,
                payment_id: p.id,
                amount: due,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &DepositPlan) -> Result<Committed, ToolError> {
        let o = repo::order_for_update(tx, &plan.order_id)
            .await?
            .ok_or_else(|| ToolError::new(ORDER_NOT_FOUND, "The order is gone."))?;
        let p = repo::payment(tx, &o.id)
            .await?
            .ok_or_else(|| ToolError::new(PAYMENT_NOT_FOUND, "The payment is gone."))?;
        let bank = outside_system(
            "bank",
            "The bank is not connected: the deposit is recorded as the operator confirmed it, without asking the bank.",
        );
        if p.payment_status != PaymentStatus::PaymentComplete.as_str() {
            waiting_deposit(&p)?;
            let now = tx.clock.now();
            repo::complete_payment(tx, &p.id, now).await?;
            if o.order_status == OrderStatus::PaymentDue.as_str() {
                repo::set_order_and_items_status(tx, &o.id, OrderStatus::Processing.as_str())
                    .await?;
            }
            repo::append_event(
                tx,
                &o.id,
                ActionType::PAY.0,
                Self::META.tool,
                Some(&o.order_status),
                Some(OrderStatus::Processing.as_str()),
                &json!({"payment_id": p.id, "amount": plan.amount, "outside_system": bank}),
            )
            .await?;
        }
        let mut produced = Map::new();
        produced.insert("payment_id".into(), json!(p.id));
        produced.insert("order_id".into(), json!(o.id));
        produced.insert(
            "payment_status".into(),
            json!(PaymentStatus::PaymentComplete),
        );
        produced.insert("order_status".into(), json!(OrderStatus::Processing));
        produced.insert("outside_system".into(), bank);
        Ok(Committed { id: p.id, produced })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &DepositPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let p = repo::payment(&mut tx, &plan.order_id).await?;
        let o = repo::order(&mut tx, &plan.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        let paid = p.as_ref().map(|p| p.payment_status.as_str());
        if paid != Some(PaymentStatus::PaymentComplete.as_str()) {
            mismatches.push(format!("payment {} is {paid:?}", plan.payment_id));
        }
        let status = o.as_ref().map(|o| o.order_status.as_str());
        if status == Some(OrderStatus::PaymentDue.as_str()) {
            mismatches.push(format!("order {} still waits for payment", plan.order_id));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, _: &mut TxCx, plan: &DepositPlan, _: &Committed) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "The payment {} is confirmed and the order released to ship; that is not taken back.",
                plan.payment_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<ConfirmVirtualAccountDepositInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["order_id", "amount"]));
        assert_eq!(ConfirmVirtualAccountDeposit::META.kind, Kind::WriteApproval);
    }

    #[tokio::test]
    async fn shoppers_cannot_confirm_their_own_deposit() {
        let cx = crate::testdb::offline("cus_priya");
        let e = ConfirmVirtualAccountDeposit
            .preview(
                &cx,
                serde_json::from_value(json!({"order_id": "ord_29001",
                    "amount": {"value": 128.00, "currency": "USD"}}))
                .unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    /// Integration (`DATABASE_URL`, demo seed): Priya's virtual-account order #29001 waits for
    /// $128.00; a wrong amount is refused, the right one is previewed for approval and confirmed
    /// in a transaction that is rolled back.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn sj_confirms_priyas_deposit() {
        let sj = crate::testdb::cx_at("adm_sj", "2026-11-06T11:00").await;
        let input = |value: f64| {
            serde_json::from_value::<ConfirmVirtualAccountDepositInput>(
                json!({"order_id": "ord_29001", "amount": {"value": value, "currency": "USD"}}),
            )
            .unwrap()
        };
        let e = ConfirmVirtualAccountDeposit
            .preview(&sj, input(100.00))
            .await
            .unwrap_err();
        assert_eq!(e.code, "amount_mismatch");
        let planned = ConfirmVirtualAccountDeposit
            .preview(&sj, input(128.00))
            .await
            .unwrap();
        assert!(matches!(planned.approval, ApprovalNeed::Needed { .. }));
        assert_eq!(planned.changes[1].to, "processing");
        let mut tx = sj.begin().await.unwrap();
        let done = ConfirmVirtualAccountDeposit
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap();
        assert_eq!(done.produced["outside_system"]["connected"], false);
        let status: String = sqlx::query_scalar(
            "SELECT order_status FROM sales.sales_order WHERE tenant_id = $1 AND id = $2",
        )
        .bind("store_northwind")
        .bind("ord_29001")
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        assert_eq!(status, "processing");
        drop(tx); // rolled back
        // Too late: four days after the order.
        let late = crate::testdb::cx_at("adm_sj", "2026-11-09T17:00").await;
        let e = ConfirmVirtualAccountDeposit
            .preview(&late, input(128.00))
            .await
            .unwrap_err();
        assert_eq!(e.code, "deadline_passed");
    }
}
