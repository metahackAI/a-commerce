//! `remedy`: compensation for a customer (`customer.remedy`), classified by liability and
//! approved step by step: an agent or operator requests it, a remedy approver approves it, and
//! cash also needs the payout approver when it is paid (`remedy.approval_levels`; the change-set
//! engine holds each approval step to the role `approval.yml` names). States: `requested` →
//! `approved` | `rejected`; paying moves `approved` → `paid` (cash through `payout_pending`).
//!
//! Paying is a stub here: cash goes through no real payment provider, and points and coupons
//! belong to promotion, which this domain cannot call. The payout record and the tool output say
//! so, and name the promotion tool that grants points or coupons.

use acom_contract::ToolError;
use acom_core::{Currency, Money};
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};

use crate::REMEDY_NOT_FOUND;
use crate::access::db;
use crate::progress::cents;

mod approve_remedy;
mod pay_remedy;
mod request_remedy;

pub use approve_remedy::*;
pub use pay_remedy::*;
pub use request_remedy::*;

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct RemedyRow {
    pub id: String,
    pub customer_id: Option<String>,
    pub inquiry_id: Option<String>,
    pub order_id: Option<String>,
    pub remedy_type: String,
    pub liability: String,
    pub amount_value: Decimal,
    pub amount_currency: String,
    pub reason: String,
    pub remedy_status: String,
    pub requested_by_id: String,
    pub decided_by_id: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_reason: Option<String>,
    pub paid_at: Option<DateTime<Utc>>,
    pub payout: Option<Value>,
    pub version: i32,
}

impl RemedyRow {
    pub(crate) fn amount(&self) -> Money {
        let currency = Currency::parse(self.amount_currency.trim()).unwrap_or(Currency::USD);
        cents(self.amount_value, currency)
    }
}

const COLUMNS: &str = "id, customer_id, inquiry_id, order_id, remedy_type, liability,
    amount_value, trim(amount_currency) AS amount_currency, reason, remedy_status,
    requested_by_id, decided_by_id, decided_at, decision_reason, paid_at, payout, version";

fn remedy_not_found(id: &str) -> ToolError {
    ToolError::new(REMEDY_NOT_FOUND, format!("There is no remedy {id}."))
        .detail(json!({"entity": "remedy", "id": id}))
        .next("search_inquiries")
}

pub(crate) async fn remedy(tx: &mut TxCx, id: &str) -> Result<Option<RemedyRow>, ToolError> {
    sqlx::query_as::<_, RemedyRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM customer.remedy WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Open (not yet paid or rejected) remedies of a member, for withdrawal eligibility.
pub(crate) async fn open_of_member(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Vec<RemedyRow>, ToolError> {
    sqlx::query_as::<_, RemedyRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM customer.remedy
         WHERE tenant_id = $1 AND customer_id = $2
           AND remedy_status IN ('requested', 'approved', 'payout_pending')
         ORDER BY requested_at, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The decision fields of a remedy.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RemedyState {
    pub status: String,
    pub decided_by_id: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub decision_reason: Option<String>,
    pub paid_at: Option<DateTime<Utc>>,
    pub payout: Option<Value>,
}

impl From<&RemedyRow> for RemedyState {
    fn from(r: &RemedyRow) -> Self {
        Self {
            status: r.remedy_status.clone(),
            decided_by_id: r.decided_by_id.clone(),
            decided_at: r.decided_at,
            decision_reason: r.decision_reason.clone(),
            paid_at: r.paid_at,
            payout: r.payout.clone(),
        }
    }
}

async fn write_state(tx: &mut TxCx, id: &str, s: &RemedyState) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE customer.remedy SET remedy_status = $3, decided_by_id = $4, decided_at = $5,
           decision_reason = $6, paid_at = $7, payout = $8, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2
           AND (remedy_status, decided_by_id, decided_at, decision_reason, paid_at, payout)
               IS DISTINCT FROM ($3, $4, $5, $6, $7, $8)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&s.status)
    .bind(s.decided_by_id.as_deref())
    .bind(s.decided_at)
    .bind(s.decision_reason.as_deref())
    .bind(s.paid_at)
    .bind(s.payout.as_ref())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Who decided: the change set that ran the decision (it records the approving person), or the
/// caller when it ran outside a change set.
fn decider(tx: &TxCx) -> String {
    tx.change_set_id
        .clone()
        .unwrap_or_else(|| tx.principal.subject.clone())
}

/// `$15.00` for an approval reason.
fn dollars(m: Money) -> String {
    format!("{} {}", m.value, m.currency)
}

#[cfg(test)]
mod tests {
    use acom_contract::WriteOp;
    use acom_core::enums::{Liability, RemedyType};

    use super::*;

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn a_remedy_is_requested_approved_and_paid() {
        let cx = crate::testdb::cx("adm_sj").await;
        let planned = RequestRemedy
            .preview(
                &cx,
                RequestRemedyInput {
                    target: RemedyTarget::Inquiry,
                    inquiry_id: Some("inq_771".into()),
                    order_id: None,
                    remedy_type: RemedyType::Cash,
                    liability: Liability::StoreFault,
                    amount: Money::new(Decimal::new(1500, 2), Currency::USD),
                    reason: "Sole came apart after a week.".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(planned.plan.customer_id.as_deref(), Some("cus_alex"));
        assert_eq!(planned.plan.order_id.as_deref(), Some("ord_20402"));
        let mut tx = cx.begin().await.unwrap();
        let made = RequestRemedy.commit(&mut tx, &planned.plan).await.unwrap();
        let row = remedy(&mut tx, &made.id).await.unwrap().unwrap();
        assert_eq!(row.remedy_status, "requested");
        let approve = ApprovePlan {
            remedy_id: made.id.clone(),
            before: RemedyState::from(&row),
            decision: "approved".into(),
            reason: None,
        };
        ApproveRemedy.commit(&mut tx, &approve).await.unwrap();
        let row = remedy(&mut tx, &made.id).await.unwrap().unwrap();
        assert_eq!(row.remedy_status, "approved");
        let pay = PayPlan {
            remedy_id: made.id.clone(),
            before: RemedyState::from(&row),
            payout: payout_for(&row, Some("0937")),
        };
        let paid = PayRemedy.commit(&mut tx, &pay).await.unwrap();
        let row = remedy(&mut tx, &made.id).await.unwrap().unwrap();
        assert_eq!(row.remedy_status, "paid");
        assert_eq!(row.payout.unwrap()["to"], "card ending 0937");
        assert_eq!(paid.produced["payout"]["delivered"], false);
        // Undoing the request takes it away only while nobody decided it.
        let e = RequestRemedy
            .undo(&mut tx, &planned.plan, &made)
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_state");
        // Rolled back on drop.
    }
}
