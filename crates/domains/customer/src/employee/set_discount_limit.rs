//! `set_discount_limit`: change an employee's discount limit and the period it renews over.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::DiscountLimitPeriod;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{by_id, employee_not_found, write_limit};
use crate::EMPLOYEE_NOT_FOUND;
use crate::access::{approval, staff_only};
use crate::progress::cents;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetDiscountLimitInput {
    /// The employee (`register_employee`).
    #[schemars(regex(pattern = r"^emp_[a-z0-9_]+$"), example = &"emp_041")]
    pub employee_id: String,
    /// The most employee discount per period.
    #[schemars(example = limit_example())]
    pub discount_limit: Money,
    /// month or year: the period the limit renews over.
    #[schemars(example = &"year")]
    pub period: DiscountLimitPeriod,
}

fn limit_example() -> Value {
    json!({"value": 750.00, "currency": "USD"})
}

/// The limit and period before and after.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetDiscountLimitPlan {
    pub employee_id: String,
    pub before: (Money, String),
    pub after: (Money, String),
}

pub struct SetDiscountLimit;

impl Operation for SetDiscountLimit {
    const META: OpMeta = OpMeta {
        address: "customer.employee.set_discount_limit",
        tool: "set_discount_limit",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/customer/employees/{employee_id}/discount-limit",
        summary: "Change an employee's discount limit (amount per month or per year).",
        consumes: &[Slot::one("employee_id")],
        produces: &[Slot::one("employee_id")],
        checks: &[],
        errors: &[EMPLOYEE_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetDiscountLimitInput;
    type Output = Value;
}

impl WriteOp<Cx> for SetDiscountLimit {
    type Plan = SetDiscountLimitPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetDiscountLimitInput,
    ) -> Result<Planned<SetDiscountLimitPlan>, ToolError> {
        staff_only(cx)?;
        if input.discount_limit.value.is_sign_negative() {
            return Err(
                ToolError::invalid_input("A discount limit cannot be negative.")
                    .detail(json!({"field": "discount_limit", "reason": "range"})),
            );
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let e = by_id(&mut tx, &input.employee_id)
            .await?
            .ok_or_else(|| employee_not_found(&input.employee_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        let before = (e.limit()?, e.discount_limit_period.clone());
        let limit = cents(input.discount_limit.value, input.discount_limit.currency);
        let after = (limit, input.period.as_str().to_owned());
        let changes = if before == after {
            vec![]
        } else {
            vec![Change {
                op: ChangeOp::Update,
                entity: "employee".into(),
                id: e.id.clone(),
                from: json!(before.0),
                to: json!(after.0),
                note: (before.1 != after.1)
                    .then(|| format!("Renews every {} instead of every {}", after.1, before.1)),
            }]
        };
        let facts = json!({"employee_id": e.id});
        Ok(Planned {
            changes,
            checks: vec![],
            basis: vec![Basis {
                entity: "customer.employee_role".into(),
                id: e.id.clone(),
                version: i64::from(e.version),
            }],
            approval: approval(cx, Self::META.tool, &facts),
            plan: SetDiscountLimitPlan {
                employee_id: e.id,
                before,
                after,
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &SetDiscountLimitPlan,
    ) -> Result<Committed, ToolError> {
        if by_id(tx, &plan.employee_id).await?.is_none() {
            return Err(employee_not_found(&plan.employee_id));
        }
        write_limit(tx, &plan.employee_id, plan.after.0, &plan.after.1).await?;
        let mut produced = Map::new();
        produced.insert("employee_id".into(), json!(plan.employee_id));
        Ok(Committed {
            id: plan.employee_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SetDiscountLimitPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = by_id(&mut tx, &plan.employee_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            Some(e) if (e.limit()?, e.discount_limit_period.clone()) == plan.after => vec![],
            Some(e) => vec![format!("{} has another limit", e.id)],
            None => vec![format!("{} is gone", plan.employee_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SetDiscountLimitPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        write_limit(tx, &plan.employee_id, plan.before.0, &plan.before.1).await
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_with_money_and_a_period_word() {
        let s = closed_schema_for::<SetDiscountLimitInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            SetDiscountLimit::META.path,
            "/v1/customer/employees/{employee_id}/discount-limit"
        );
    }
}
