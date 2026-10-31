//! `set_installment` (`sales.installment.set`): how many monthly installments a card payment is
//! split into (`order.payment.installment`). The card provider settles the charge when the order
//! ships, so the plan can change until then (`order.change.allowed_before`). The provider is
//! outside this system: the plan is recorded here and nothing is sent, as the output says.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::PaymentStatus;
use acom_promotion::service::for_sales as promotion;
use acom_store::{Cx, TxCx};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::order::{before_shipping, visible_order};
use crate::payment::PAYMENT_NOT_FOUND;
use crate::repo;
use crate::rules::{self, PaymentRules};
use crate::{
    ALREADY_SHIPPED, BELOW_MIN_AMOUNT, MONTHS_NOT_AVAILABLE, NOT_CARD_PAYMENT, ORDER_NOT_FOUND,
    approval, outside_system,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetInstallmentInput {
    /// The order paid by card (`get_my_orders`, `search_orders`).
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20521")]
    pub order_id: String,
    /// Monthly installments, one of `payment.installment.months` (`get_payment_methods`
    /// `installment_months`); 0 pays at once.
    #[schemars(example = 3)]
    pub installment_months: u32,
}

/// What `set_installment` changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InstallmentPlan {
    order_id: String,
    payment_id: String,
    from: u32,
    to: u32,
}

pub(crate) struct SetInstallment;

impl Operation for SetInstallment {
    const META: OpMeta = OpMeta {
        address: "sales.installment.set",
        tool: "set_installment",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/sales/orders/{order_id}/installment",
        summary: "Split an order's card payment into monthly installments (or back to paying at once) before the order ships, in the months and above the minimum the store offers. The card provider is not called.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("payment_id")],
        checks: &[
            "payment.installment.methods",
            "payment.installment.min_amount",
            "payment.installment.months",
        ],
        errors: &[
            MONTHS_NOT_AVAILABLE,
            BELOW_MIN_AMOUNT,
            NOT_CARD_PAYMENT,
            ALREADY_SHIPPED,
            ORDER_NOT_FOUND,
            PAYMENT_NOT_FOUND,
            ErrorCode::INVALID_STATE,
        ],
        audience: Audience::Both,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetInstallmentInput;
    type Output = Value;
}

fn methods_error(code: acom_contract::ErrorCode, message: String, detail: Value) -> ToolError {
    ToolError::new(code, message)
        .detail(detail)
        .next("get_payment_methods")
}

/// The policy's word on `months` for a payment of `amount` by `method`; the check's detail
/// when it may.
pub(crate) fn allowed(
    rules: &PaymentRules,
    method: &str,
    amount: Money,
    months: u32,
) -> Result<String, ToolError> {
    if !rules.installment_methods.iter().any(|m| m == method) {
        return Err(methods_error(
            NOT_CARD_PAYMENT,
            format!(
                "Installments are for {} payments; this one is {}.",
                rules.installment_methods.join(", ").replace('_', " "),
                method.replace('_', " ")
            ),
            json!({"payment_method": method, "policy_key": "payment.installment.methods"}),
        ));
    }
    if !rules.installment_months.contains(&months) {
        return Err(methods_error(
            MONTHS_NOT_AVAILABLE,
            format!(
                "{months} installments are not offered; choose one of {:?} (0 pays at once).",
                rules.installment_months
            ),
            json!({"field": "installment_months", "allowed": rules.installment_months,
                   "policy_key": "payment.installment.months"}),
        ));
    }
    if months == 0 {
        return Ok(format!("Paid at once: {amount}"));
    }
    // In cents, like every amount an order shows ("50.00", not "50.0").
    let minimum = rules
        .installment_min
        .map(|m| promotion::money(m.value, m.currency));
    if let Some(min) = minimum
        && (min.currency != amount.currency || amount.value < min.value)
    {
        return Err(methods_error(
            BELOW_MIN_AMOUNT,
            format!("Installments start at {min}; this order is {amount}."),
            json!({"min_amount": min, "amount": amount,
                   "policy_key": "payment.installment.min_amount"}),
        ));
    }
    let each = promotion::money(amount.value / Decimal::from(months), amount.currency);
    Ok(match minimum {
        Some(min) => format!(
            "Order {amount} is over the {min} minimum; {months} × {}",
            each.value
        ),
        None => format!("{months} × {}", each.value),
    })
}

impl WriteOp<Cx> for SetInstallment {
    type Plan = InstallmentPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetInstallmentInput,
    ) -> Result<Planned<InstallmentPlan>, ToolError> {
        let rules = rules::payment_rules(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let o = visible_order(cx, &mut tx, &input.order_id).await?;
        let p = repo::payment(&mut tx, &o.id).await?.ok_or_else(|| {
            ToolError::new(PAYMENT_NOT_FOUND, format!("Order {} has no payment.", o.id))
                .detail(json!({"entity": "payment", "order_id": o.id}))
                .next("get_order")
        })?;
        let stage = before_shipping(&mut tx, &o, "get_payment").await?;
        tx.commit().await.map_err(ToolError::from)?;
        if ![PaymentStatus::PaymentDue, PaymentStatus::PaymentComplete]
            .iter()
            .any(|s| s.as_str() == p.payment_status)
        {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "The payment for order #{} is {}.",
                    o.order_number,
                    p.payment_status.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "payment", "from": p.payment_status}))
            .next("get_payment"));
        }
        let amount = repo::money(p.amount_value, &p.amount_currency)?;
        let detail = allowed(&rules, &p.payment_method, amount, input.installment_months)?;
        let from = u32::try_from(p.installment_months).unwrap_or(0);
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "payment".into(),
                id: p.id.clone(),
                from: json!({"installment_months": from}),
                to: json!({"installment_months": input.installment_months}),
                note: Some(format!(
                    "{stage}; the card provider is not connected, so the plan is recorded here"
                )),
            }],
            checks: vec![Check {
                rule: "payment.installment.min_amount".into(),
                passed: true,
                detail: Some(detail),
                fix_tool: None,
            }],
            basis: vec![Basis {
                entity: "sales.payment".into(),
                id: p.id.clone(),
                version: i64::from(p.version),
            }],
            approval: approval(
                cx,
                Self::META.tool,
                &json!({"amount": amount, "installment_months": input.installment_months}),
            ),
            plan: InstallmentPlan {
                order_id: o.id,
                payment_id: p.id,
                from,
                to: input.installment_months,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &InstallmentPlan) -> Result<Committed, ToolError> {
        let o = repo::order_for_update(tx, &plan.order_id)
            .await?
            .ok_or_else(|| ToolError::new(ORDER_NOT_FOUND, "The order is gone."))?;
        before_shipping(tx, &o, "get_payment").await?;
        let provider = outside_system(
            "card_provider",
            "The card provider is not connected: the installment plan is recorded here and was not sent.",
        );
        repo::set_installment_months(tx, &plan.payment_id, plan.to).await?;
        repo::append_event(
            tx,
            &o.id,
            ActionType::UPDATE.0,
            Self::META.tool,
            None,
            None,
            &json!({"payment_id": plan.payment_id, "installment_months": plan.to,
                    "previous_installment_months": plan.from, "outside_system": provider}),
        )
        .await?;
        let mut produced = Map::new();
        produced.insert("payment_id".into(), json!(plan.payment_id));
        produced.insert("installment_months".into(), json!(plan.to));
        produced.insert("outside_system".into(), provider);
        Ok(Committed {
            id: plan.payment_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &InstallmentPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let p = repo::payment(&mut tx, &plan.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let now = p.map(|p| p.installment_months);
        let matched = now == i32::try_from(plan.to).ok();
        Ok(Verification {
            matched,
            mismatches: if matched {
                vec![]
            } else {
                vec![format!(
                    "payment {} has {now:?} installments",
                    plan.payment_id
                )]
            },
        })
    }

    /// Back to the installments before, while the order has not shipped.
    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &InstallmentPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        let o = repo::order_for_update(tx, &plan.order_id)
            .await?
            .ok_or_else(|| ToolError::new(ORDER_NOT_FOUND, "The order is gone."))?;
        before_shipping(tx, &o, "get_payment").await?;
        repo::set_installment_months(tx, &plan.payment_id, plan.from).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};
    use acom_core::Currency;
    use std::str::FromStr;

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<SetInstallmentInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["order_id", "installment_months"]));
    }

    #[test]
    fn installments_follow_the_policy() {
        let rules = rules::payment_rules(&crate::rules::tests::defaults()).unwrap();
        assert_eq!(
            allowed(&rules, "credit_card", usd("213.60"), 3).unwrap(),
            "Order 213.60 USD is over the 50.00 USD minimum; 3 × 71.20"
        );
        assert_eq!(
            allowed(&rules, "credit_card", usd("40.00"), 0).unwrap(),
            "Paid at once: 40.00 USD"
        );
        let code = |e: ToolError| e.code.to_string();
        assert_eq!(
            code(allowed(&rules, "credit_card", usd("213.60"), 4).unwrap_err()),
            "months_not_available"
        );
        assert_eq!(
            code(allowed(&rules, "credit_card", usd("40.00"), 3).unwrap_err()),
            "below_min_amount"
        );
        assert_eq!(
            code(allowed(&rules, "virtual_account", usd("213.60"), 3).unwrap_err()),
            "not_card_payment"
        );
    }

    /// Integration (`DATABASE_URL`, demo seed): Owen's card order #29002 waits for his dad to
    /// accept the gift; he splits it into three installments in a transaction that is rolled
    /// back. Alex's #20402 has been delivered, so its plan can no longer change.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn owen_splits_his_payment_before_it_ships() {
        let owen = crate::testdb::cx("cus_owen").await;
        let planned = SetInstallment
            .preview(
                &owen,
                SetInstallmentInput {
                    order_id: "ord_29002".into(),
                    installment_months: 3,
                },
            )
            .await
            .unwrap();
        assert_eq!(planned.changes[0].to["installment_months"], 3);
        let mut tx = owen.begin().await.unwrap();
        let done = SetInstallment.commit(&mut tx, &planned.plan).await.unwrap();
        assert_eq!(done.produced["outside_system"]["connected"], false);
        drop(tx); // rolled back
        let alex = crate::testdb::cx("cus_alex").await;
        let e = SetInstallment
            .preview(
                &alex,
                SetInstallmentInput {
                    order_id: "ord_20402".into(),
                    installment_months: 3,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "already_shipped");
    }
}
