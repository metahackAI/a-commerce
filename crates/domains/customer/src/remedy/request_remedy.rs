//! `request_remedy`: ask for compensation (`requested`). No money or points move yet.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::{Liability, RemedyType};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{COLUMNS, RemedyRow, remedy};
use crate::access::{approval, db, policy_words, staff_only};
use crate::inquiry::{inquiry, inquiry_not_found};
use crate::progress::cents;
use crate::{INQUIRY_NOT_FOUND, INVALID_LIABILITY};

/// What the remedy answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RemedyTarget {
    /// A customer inquiry (`inquiry_id`); the customer and order come from it.
    Inquiry,
    /// An order (`order_id`), when there is no inquiry.
    Order,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestRemedyInput {
    /// inquiry: needs `inquiry_id`; order: needs `order_id`.
    #[schemars(example = &"inquiry")]
    pub target: RemedyTarget,
    /// The inquiry it answers. With `target: inquiry`.
    #[schemars(regex(pattern = r"^inq_[a-z0-9_]+$"), example = &"inq_771")]
    pub inquiry_id: Option<String>,
    /// The order it is about. With `target: order`.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20402")]
    pub order_id: Option<String>,
    /// What the customer gets: loyalty_points; coupon; cash.
    #[schemars(example = &"loyalty_points")]
    pub remedy_type: RemedyType,
    /// Whose fault it was: store_fault; seller_fault; customer_fault; carrier_fault; none.
    #[schemars(example = &"store_fault")]
    pub liability: Liability,
    /// How much the remedy is worth.
    #[schemars(example = amount_example())]
    pub amount: Money,
    /// Why, in a sentence.
    #[schemars(length(min = 1, max = 500), example = &"Late delivery: arrived two days after the promised date.")]
    pub reason: String,
}

fn amount_example() -> Value {
    json!({"value": 15.00, "currency": "USD"})
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestRemedyPlan {
    pub remedy_id: String,
    pub customer_id: Option<String>,
    pub inquiry_id: Option<String>,
    pub order_id: Option<String>,
    pub remedy_type: String,
    pub liability: String,
    pub amount: Money,
    pub reason: String,
    pub requested_by: String,
    /// `false` when the same open request already exists (it is returned instead).
    pub changes_anything: bool,
}

pub struct RequestRemedy;

impl Operation for RequestRemedy {
    const META: OpMeta = OpMeta {
        address: "customer.remedy.request",
        tool: "request_remedy",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/remedies",
        summary: "Request compensation (points, coupon or cash) for an inquiry or an order, with whose fault it was. Nothing is paid until it is approved and paid.",
        consumes: &[Slot::one("inquiry_id"), Slot::one("order_id")],
        produces: &[Slot::one("remedy_id")],
        checks: &["remedy.liability"],
        errors: &[INVALID_LIABILITY, INQUIRY_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = RequestRemedyInput;
    type Output = Value;
}

/// An open request for the same thing, which a repeated request returns.
async fn same_open(tx: &mut TxCx, p: &RequestRemedyPlan) -> Result<Option<RemedyRow>, ToolError> {
    sqlx::query_as::<_, RemedyRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM customer.remedy
         WHERE tenant_id = $1 AND inquiry_id IS NOT DISTINCT FROM $2
           AND order_id IS NOT DISTINCT FROM $3 AND remedy_type = $4 AND amount_value = $5
           AND trim(amount_currency) = $6
           AND remedy_status IN ('requested', 'approved', 'payout_pending')
         ORDER BY requested_at, id LIMIT 1"
    )))
    .bind(tx.tenant.as_str())
    .bind(p.inquiry_id.as_deref())
    .bind(p.order_id.as_deref())
    .bind(&p.remedy_type)
    .bind(p.amount.value)
    .bind(p.amount.currency.as_str())
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The customer of an order, from an inquiry about it (orders belong to sales, which this domain
/// cannot read).
async fn customer_of_order(tx: &mut TxCx, order_id: &str) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT customer_id FROM customer.customer_inquiry
         WHERE tenant_id = $1 AND order_id = $2 ORDER BY date_received DESC LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

impl WriteOp<Cx> for RequestRemedy {
    type Plan = RequestRemedyPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RequestRemedyInput,
    ) -> Result<Planned<RequestRemedyPlan>, ToolError> {
        staff_only(cx)?;
        let liability = input.liability.as_str();
        let allowed = policy_words(cx, "customer.remedy.liability");
        if !allowed.iter().any(|l| l == liability) {
            return Err(ToolError::new(
                INVALID_LIABILITY,
                format!("{liability} is not a liability this store uses."),
            )
            .detail(json!({"field": "liability", "reason": "enum", "allowed": allowed}))
            .next("list_enums"));
        }
        let kinds = policy_words(cx, "customer.remedy.types");
        if !kinds.iter().any(|k| k == input.remedy_type.as_str()) {
            return Err(ToolError::invalid_input(format!(
                "This store does not give {} remedies.",
                input.remedy_type
            ))
            .detail(json!({"field": "remedy_type", "reason": "enum", "allowed": kinds})));
        }
        if input.amount.value <= rust_decimal::Decimal::ZERO {
            return Err(
                ToolError::invalid_input("A remedy is worth more than zero.")
                    .detail(json!({"field": "amount", "reason": "range"})),
            );
        }
        let reason = input.reason.trim().to_owned();
        if reason.is_empty() {
            return Err(ToolError::invalid_input("Say why the remedy is requested.")
                .detail(json!({"field": "reason", "reason": "missing"})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (customer_id, inquiry_id, order_id, basis) = match input.target {
            RemedyTarget::Inquiry => {
                let id = input.inquiry_id.as_deref().ok_or_else(|| {
                    ToolError::invalid_input("`target: inquiry` needs `inquiry_id`.")
                        .detail(json!({"field": "inquiry_id", "reason": "missing"}))
                        .next("request_remedy")
                })?;
                let i = inquiry(&mut tx, id)
                    .await?
                    .ok_or_else(|| inquiry_not_found(id))?;
                let basis = Basis {
                    entity: "customer.customer_inquiry".into(),
                    id: i.id.clone(),
                    version: i64::from(i.version),
                };
                (Some(i.customer_id), Some(i.id), i.order_id, vec![basis])
            }
            RemedyTarget::Order => {
                let id = input.order_id.clone().ok_or_else(|| {
                    ToolError::invalid_input("`target: order` needs `order_id`.")
                        .detail(json!({"field": "order_id", "reason": "missing"}))
                        .next("request_remedy")
                })?;
                (
                    customer_of_order(&mut tx, &id).await?,
                    None,
                    Some(id),
                    vec![],
                )
            }
        };
        let mut plan = RequestRemedyPlan {
            remedy_id: format!("rmd_{}", acom_core::id::new_suffix()),
            customer_id,
            inquiry_id,
            order_id,
            remedy_type: input.remedy_type.as_str().to_owned(),
            liability: liability.to_owned(),
            amount: cents(input.amount.value, input.amount.currency),
            reason,
            requested_by: cx.principal.subject.clone(),
            changes_anything: true,
        };
        let existing = same_open(&mut tx, &plan).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let changes = match &existing {
            Some(e) => {
                plan.remedy_id = e.id.clone();
                plan.changes_anything = false;
                vec![]
            }
            None => vec![Change {
                op: ChangeOp::Create,
                entity: "remedy".into(),
                id: String::new(),
                from: Value::Null,
                to: json!("requested"),
                note: Some(format!(
                    "{} {} in {}",
                    plan.amount.value,
                    plan.amount.currency,
                    plan.remedy_type.replace('_', " ")
                )),
            }],
        };
        let fault = liability.replace('_', " ");
        let facts = json!({"remedy_type": plan.remedy_type, "amount": plan.amount});
        Ok(Planned {
            changes,
            checks: vec![Check {
                rule: "remedy.liability".into(),
                passed: true,
                detail: Some(match &existing {
                    Some(e) => format!("Already requested as {} ({})", e.id, e.remedy_status),
                    None => format!("{}: {}", crate::wording::capitalize(&fault), plan.reason),
                }),
                fix_tool: None,
            }],
            basis,
            approval: approval(cx, Self::META.tool, &facts),
            plan,
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &RequestRemedyPlan,
    ) -> Result<Committed, ToolError> {
        if plan.changes_anything {
            sqlx::query(
                "INSERT INTO customer.remedy (tenant_id, id, customer_id, inquiry_id, order_id,
                   remedy_type, liability, amount_value, amount_currency, reason, remedy_status,
                   requested_by_id, requested_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'requested', $11, $12)
                 ON CONFLICT (tenant_id, id) DO NOTHING",
            )
            .bind(tx.tenant.as_str())
            .bind(&plan.remedy_id)
            .bind(plan.customer_id.as_deref())
            .bind(plan.inquiry_id.as_deref())
            .bind(plan.order_id.as_deref())
            .bind(&plan.remedy_type)
            .bind(&plan.liability)
            .bind(plan.amount.value)
            .bind(plan.amount.currency.as_str())
            .bind(&plan.reason)
            .bind(&plan.requested_by)
            .bind(tx.clock.now())
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        }
        let mut produced = Map::new();
        produced.insert("remedy_id".into(), json!(plan.remedy_id));
        Ok(Committed {
            id: plan.remedy_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RequestRemedyPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = remedy(&mut tx, &plan.remedy_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            Some(r) if r.amount() == plan.amount && r.remedy_type == plan.remedy_type => vec![],
            Some(r) => vec![format!("{} differs from the plan", r.id)],
            None => vec![format!("{} was not saved", plan.remedy_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &RequestRemedyPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        if !plan.changes_anything {
            return Ok(());
        }
        match remedy(tx, &plan.remedy_id).await? {
            None => Ok(()),
            Some(r) if r.remedy_status == "requested" => {
                sqlx::query("DELETE FROM customer.remedy WHERE tenant_id = $1 AND id = $2")
                    .bind(tx.tenant.as_str())
                    .bind(&plan.remedy_id)
                    .execute(&mut *tx.tx)
                    .await
                    .map_err(db)?;
                Ok(())
            }
            Some(r) => Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} was already {}; undo that decision first.",
                    r.id, r.remedy_status
                ),
            )
            .detail(json!({"entity": "remedy", "from": r.remedy_status}))),
        }
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_with_target_and_liability_words() {
        let s = closed_schema_for::<RequestRemedyInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            s["properties"]["target"]["enum"],
            json!(["inquiry", "order"])
        );
        assert_eq!(
            s["properties"]["liability"]["enum"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
    }
}
