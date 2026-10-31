//! `employee`: members who work for the store and their employee discount limit
//! (`customer.employee_role`). The employee discount replaces other member benefits on an order
//! (`promotion.member_benefits_per_order`).

use acom_contract::ToolError;
use acom_core::{Currency, Money};
use acom_store::TxCx;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde_json::json;

use crate::EMPLOYEE_NOT_FOUND;
use crate::access::db;

mod register_employee;
mod set_discount_limit;

pub use register_employee::*;
pub use set_discount_limit::*;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct EmployeeRow {
    pub id: String,
    pub customer_id: String,
    pub employee_number: String,
    pub discount_limit_value: Decimal,
    pub discount_limit_currency: String,
    pub discount_limit_period: String,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub version: i32,
}

impl EmployeeRow {
    pub(crate) fn limit(&self) -> Result<Money, ToolError> {
        let currency = Currency::parse(self.discount_limit_currency.trim())
            .map_err(|e| ToolError::new(acom_contract::ErrorCode::INTERNAL, e.to_string()))?;
        Ok(crate::progress::cents(self.discount_limit_value, currency))
    }
}

const COLUMNS: &str = "id, customer_id, employee_number, discount_limit_value,
    trim(discount_limit_currency) AS discount_limit_currency, discount_limit_period, start_date,
    end_date, version";

fn employee_not_found(id: &str) -> ToolError {
    ToolError::new(EMPLOYEE_NOT_FOUND, format!("There is no employee {id}."))
        .detail(json!({"entity": "employee", "id": id}))
        .next("register_employee")
}

pub(crate) async fn by_id(tx: &mut TxCx, id: &str) -> Result<Option<EmployeeRow>, ToolError> {
    sqlx::query_as::<_, EmployeeRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM customer.employee_role WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The employee record of a member, or the one with this employee number.
pub(crate) async fn by_member_or_number(
    tx: &mut TxCx,
    customer_id: &str,
    employee_number: &str,
) -> Result<Vec<EmployeeRow>, ToolError> {
    sqlx::query_as::<_, EmployeeRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM customer.employee_role
         WHERE tenant_id = $1 AND (customer_id = $2 OR employee_number = $3) ORDER BY id"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(employee_number)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The employee record of a member, if they have one.
pub(crate) async fn of_member(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Option<EmployeeRow>, ToolError> {
    sqlx::query_as::<_, EmployeeRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM customer.employee_role WHERE tenant_id = $1 AND customer_id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Sets the limit and period (a no-op when they already have those values).
pub(crate) async fn write_limit(
    tx: &mut TxCx,
    id: &str,
    limit: Money,
    period: &str,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE customer.employee_role SET discount_limit_value = $3,
           discount_limit_currency = $4, discount_limit_period = $5, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2
           AND (discount_limit_value, trim(discount_limit_currency), discount_limit_period)
               IS DISTINCT FROM ($3, $4, $5)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(limit.value)
    .bind(limit.currency.as_str())
    .bind(period)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use acom_contract::WriteOp;
    use acom_core::enums::DiscountLimitPeriod;

    use super::*;

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn registers_an_employee_and_changes_the_limit() {
        let cx = crate::testdb::cx("adm_sj").await;
        let e = RegisterEmployee
            .preview(
                &cx,
                RegisterEmployeeInput {
                    customer_id: "cus_sam".into(),
                    employee_number: "NW-E-0041".into(),
                    discount_limit: None,
                    period: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "duplicate_employee_number");
        let planned = RegisterEmployee
            .preview(
                &cx,
                RegisterEmployeeInput {
                    customer_id: "cus_sam".into(),
                    employee_number: "NW-E-0099".into(),
                    discount_limit: None,
                    period: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(planned.plan.limit.value.to_string(), "500.00");
        let mut tx = cx.begin().await.unwrap();
        let done = RegisterEmployee
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap();
        let row = by_id(&mut tx, &done.id).await.unwrap().unwrap();
        assert_eq!(row.discount_limit_period, "year");
        let change = SetDiscountLimitPlan {
            employee_id: done.id.clone(),
            before: (row.limit().unwrap(), "year".into()),
            after: (
                Money::new(Decimal::new(75000, 2), Currency::USD),
                DiscountLimitPeriod::Month.as_str().into(),
            ),
        };
        SetDiscountLimit.commit(&mut tx, &change).await.unwrap();
        let after = by_id(&mut tx, &done.id).await.unwrap().unwrap();
        assert_eq!(after.discount_limit_period, "month");
        SetDiscountLimit
            .undo(&mut tx, &change, &done)
            .await
            .unwrap();
        RegisterEmployee
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap();
        assert!(by_id(&mut tx, &done.id).await.unwrap().is_none());
        // Rolled back on drop.
    }
}
