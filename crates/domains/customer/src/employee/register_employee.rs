//! `register_employee`: mark a member as a store employee with an employee discount limit.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::DiscountLimitPeriod;
use acom_store::{Cx, TxCx};
use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{by_id, by_member_or_number};
use crate::access::{approval, db, policy_money, policy_number, policy_value, staff_only};
use crate::person::customer_not_found;
use crate::progress::cents;
use crate::{CUSTOMER_NOT_FOUND, DUPLICATE_EMPLOYEE_NUMBER, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterEmployeeInput {
    /// The member who works for the store (`search_members`).
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_owen")]
    pub customer_id: String,
    /// The store's own employee number, unique in the store.
    #[schemars(length(min = 1, max = 40), example = &"NW-E-0042")]
    pub employee_number: String,
    /// The most employee discount per period. The store's default
    /// (`employee.default_discount_limit`) when left out.
    #[schemars(example = limit_example())]
    pub discount_limit: Option<Money>,
    /// month or year: the period the limit renews over. The store's default
    /// (`employee.default_period`) when left out.
    #[schemars(example = &"year")]
    pub period: Option<DiscountLimitPeriod>,
}

fn limit_example() -> Value {
    json!({"value": 500.00, "currency": "USD"})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterEmployeePlan {
    pub employee_id: String,
    pub customer_id: String,
    pub employee_number: String,
    pub limit: Money,
    pub period: String,
    pub start_date: NaiveDate,
    /// `false` when the member already is this employee (a repeated request).
    pub changes_anything: bool,
}

pub struct RegisterEmployee;

impl Operation for RegisterEmployee {
    const META: OpMeta = OpMeta {
        address: "customer.employee.register",
        tool: "register_employee",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/employees",
        summary: "Register a member as a store employee with an employee discount limit per month or year. The employee discount replaces other member benefits on an order.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::one("employee_id")],
        checks: &["member_benefits_per_order"],
        errors: &[DUPLICATE_EMPLOYEE_NUMBER, CUSTOMER_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::REGISTER,
        person_scope: "",
    };
    type Input = RegisterEmployeeInput;
    type Output = Value;
}

impl WriteOp<Cx> for RegisterEmployee {
    type Plan = RegisterEmployeePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RegisterEmployeeInput,
    ) -> Result<Planned<RegisterEmployeePlan>, ToolError> {
        staff_only(cx)?;
        let number = input.employee_number.trim().to_owned();
        let limit = match input.discount_limit {
            Some(m) => m,
            None => policy_money(cx, "customer.employee.default_discount_limit")?,
        };
        if limit.value.is_sign_negative() {
            return Err(
                ToolError::invalid_input("A discount limit cannot be negative.")
                    .detail(json!({"field": "discount_limit", "reason": "range"})),
            );
        }
        let limit = cents(limit.value, limit.currency);
        let period = match input.period {
            Some(p) => p.as_str().to_owned(),
            None => policy_value(cx, "customer.employee.default_period")?
                .as_str()
                .unwrap_or("year")
                .to_owned(),
        };
        let per_order = policy_number(cx, "promotion.member_benefits_per_order")?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let person = repo::person(&mut tx, &input.customer_id)
            .await?
            .ok_or_else(|| customer_not_found(&input.customer_id).next("register_member"))?;
        let existing = by_member_or_number(&mut tx, &person.id, &number).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let same = existing
            .iter()
            .find(|e| e.customer_id == person.id && e.employee_number == number);
        if same.is_none()
            && let Some(other) = existing.first()
        {
            let field = if other.employee_number == number {
                "employee_number"
            } else {
                "customer_id"
            };
            return Err(ToolError::new(
                DUPLICATE_EMPLOYEE_NUMBER,
                format!(
                    "{} already belongs to employee {} ({}).",
                    if field == "employee_number" {
                        &number
                    } else {
                        &person.id
                    },
                    other.id,
                    other.employee_number
                ),
            )
            .detail(json!({"entity": "employee", "field": field, "existing_id": other.id}))
            .next("get_member"));
        }
        let changes = match same {
            Some(_) => vec![],
            None => vec![Change {
                op: ChangeOp::Create,
                entity: "employee".into(),
                id: String::new(),
                from: Value::Null,
                to: json!("active"),
                note: Some(format!("{} {} a {}", limit.value, limit.currency, period)),
            }],
        };
        let facts = json!({"customer_id": person.id});
        Ok(Planned {
            changes,
            checks: vec![Check {
                rule: "member_benefits_per_order".into(),
                passed: true,
                detail: Some(format!(
                    "Employee discount replaces tier coupons ({per_order} member benefit per order)"
                )),
                fix_tool: None,
            }],
            basis: vec![Basis {
                entity: "customer.person".into(),
                id: person.id.clone(),
                version: i64::from(person.version),
            }],
            approval: approval(cx, Self::META.tool, &facts),
            plan: RegisterEmployeePlan {
                employee_id: same.map_or_else(
                    || format!("emp_{}", acom_core::id::new_suffix()),
                    |e| e.id.clone(),
                ),
                customer_id: person.id,
                employee_number: number,
                limit,
                period,
                start_date: cx.clock.now().with_timezone(&cx.time.zone).date_naive(),
                changes_anything: same.is_none(),
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &RegisterEmployeePlan,
    ) -> Result<Committed, ToolError> {
        if plan.changes_anything {
            sqlx::query(
                "INSERT INTO customer.employee_role (tenant_id, id, customer_id, employee_number,
                   discount_limit_value, discount_limit_currency, discount_limit_period, start_date)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (tenant_id, id) DO NOTHING",
            )
            .bind(tx.tenant.as_str())
            .bind(&plan.employee_id)
            .bind(&plan.customer_id)
            .bind(&plan.employee_number)
            .bind(plan.limit.value)
            .bind(plan.limit.currency.as_str())
            .bind(&plan.period)
            .bind(plan.start_date)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        }
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
        plan: &RegisterEmployeePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = by_id(&mut tx, &plan.employee_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            Some(e) if e.customer_id == plan.customer_id && e.limit()? == plan.limit => vec![],
            Some(e) => vec![format!("{} differs from the plan", e.id)],
            None => vec![format!("{} was not saved", plan.employee_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &RegisterEmployeePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        if plan.changes_anything {
            sqlx::query("DELETE FROM customer.employee_role WHERE tenant_id = $1 AND id = $2")
                .bind(tx.tenant.as_str())
                .bind(&plan.employee_id)
                .execute(&mut *tx.tx)
                .await
                .map_err(db)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_with_money_and_period_words() {
        let s = closed_schema_for::<RegisterEmployeeInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(s["properties"]["period"]["enum"], json!(["month", "year"]));
        assert_eq!(RegisterEmployee::META.produces, &[Slot::one("employee_id")]);
    }
}
