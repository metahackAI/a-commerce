//! `issue_receipt` (`sales.receipt.issue`): a receipt or tax document for a paid order
//! (`order.receipt`). One receipt of each type stands per order; when the order's amount changed
//! after a claim, the earlier one is cancelled and issued again (`receipt.after_claim`). A
//! receipt that was issued is not taken back.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::{PaymentStatus, ReceiptType};
use acom_store::{Cx, TxCx};
use chrono::Datelike;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::tax_filing;
use crate::order::visible_order;
use crate::repo;
use crate::rules;
use crate::{ALREADY_ISSUED, BUSINESS_ID_REQUIRED, ORDER_NOT_FOUND, approval};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueReceiptInput {
    /// The paid order (`get_my_orders`, `search_orders`).
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20402")]
    pub order_id: String,
    /// `deduction`: for income deduction; `proof`: proof of expense; `tax_invoice`: made out to
    /// a business, needs `tax_id`.
    #[schemars(example = &"proof")]
    pub receipt_type: ReceiptType,
    /// The buyer's business tax id. Needed with `tax_invoice`.
    #[schemars(length(min = 2, max = 40), example = &"12-3456789")]
    pub tax_id: Option<String>,
}

/// What `issue_receipt` records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReceiptPlan {
    receipt_id: String,
    order_id: String,
    receipt_type: ReceiptType,
    tax_id: Option<String>,
    amount: Money,
    /// The standing receipt this one replaces, cancelled first (`receipt.after_claim`).
    replaces: Option<String>,
}

pub(crate) struct IssueReceipt;

impl Operation for IssueReceipt {
    const META: OpMeta = OpMeta {
        address: "sales.receipt.issue",
        tool: "issue_receipt",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/sales/orders/{order_id}/receipt",
        summary: "Issue a receipt for a paid order: for income deduction, as proof of expense, or as a tax invoice for a business tax id. One of each type per order, in the month of the order; not taken back once issued.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("receipt_id")],
        checks: &[
            "receipt.types",
            "receipt.same_month_only",
            "receipt.after_claim",
        ],
        errors: &[
            BUSINESS_ID_REQUIRED,
            ALREADY_ISSUED,
            ORDER_NOT_FOUND,
            ErrorCode::INVALID_STATE,
            ErrorCode::POLICY_VIOLATION,
        ],
        audience: Audience::Both,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = IssueReceiptInput;
    type Output = Value;
}

fn words(t: ReceiptType) -> String {
    t.as_str().replace('_', " ")
}

impl WriteOp<Cx> for IssueReceipt {
    type Plan = ReceiptPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: IssueReceiptInput,
    ) -> Result<Planned<ReceiptPlan>, ToolError> {
        let rules = rules::receipt_rules(&cx.policy)?;
        let kind = input.receipt_type;
        if !rules.types.iter().any(|t| t == kind.as_str()) {
            return Err(ToolError::invalid_input(format!(
                "This store does not issue {} receipts.",
                words(kind)
            ))
            .detail(
                json!({"field": "receipt_type", "reason": "enum", "allowed": rules.types,
                           "policy_key": "receipt.types"}),
            )
            .next("issue_receipt"));
        }
        let tax_id = input
            .tax_id
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned);
        if kind == ReceiptType::TaxInvoice && tax_id.is_none() {
            return Err(ToolError::new(
                BUSINESS_ID_REQUIRED,
                "A tax invoice is made out to a business: give its tax id (`tax_id`).",
            )
            .detail(json!({"field": "tax_id", "reason": "missing"}))
            .next("issue_receipt"));
        }

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let o = visible_order(cx, &mut tx, &input.order_id).await?;
        let payment = repo::payment(&mut tx, &o.id).await?;
        let standing = repo::receipts(&mut tx, &o.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if payment.as_ref().map(|p| p.payment_status.as_str())
            != Some(PaymentStatus::PaymentComplete.as_str())
        {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "Order #{} is not paid yet; a receipt follows the payment.",
                    o.order_number
                ),
            )
            .detail(json!({"entity": "payment", "reason": "not_paid",
                           "from": payment.map(|p| p.payment_status)}))
            .next("get_payment"));
        }
        let ordered = o.order_date.with_timezone(&cx.time.zone).date_naive();
        let today = crate::today(cx.clock.as_ref(), &cx.time);
        let month = |d: chrono::NaiveDate| d.format("%B %Y").to_string();
        let same_month = (ordered.year(), ordered.month()) == (today.year(), today.month());
        if rules.same_month_only && !same_month {
            return Err(ToolError::new(
                ErrorCode::POLICY_VIOLATION,
                format!(
                    "Receipts are issued in the month of the order only; #{} was ordered in {}.",
                    o.order_number,
                    month(ordered)
                ),
            )
            .detail(
                json!({"rule": "receipt.same_month_only", "ordered": ordered.to_string(),
                           "today": today.to_string()}),
            )
            .next("get_receipt"));
        }
        let amount = repo::money(o.total_payment_due_value, &o.total_payment_due_currency)?;
        let receipt_id = format!("rct_{}", acom_core::id::new_suffix());
        let mut changes = Vec::new();
        let mut checks = vec![
            Check {
                rule: "receipt.types".into(),
                passed: true,
                detail: Some(format!("A {} receipt", words(kind))),
                fix_tool: None,
            },
            Check {
                rule: "receipt.same_month_only".into(),
                passed: true,
                detail: Some(if same_month {
                    format!("Ordered this month ({})", month(ordered))
                } else {
                    format!("Ordered in {}; any month allowed", month(ordered))
                }),
                fix_tool: None,
            },
        ];
        let mut replaces = None;
        if let Some(r) = standing.iter().find(|r| r.receipt_type == kind.as_str()) {
            let stated = repo::money(r.total_payment_due_value, &r.total_payment_due_currency)?;
            let changed = stated.currency != amount.currency || stated.value != amount.value;
            if !changed || rules.after_claim != "cancel_and_reissue" {
                return Err(ToolError::new(
                    ALREADY_ISSUED,
                    format!(
                        "Order #{} already has a {} receipt ({}).",
                        o.order_number,
                        words(kind),
                        r.id
                    ),
                )
                .detail(json!({"entity": "receipt", "field": "receipt_type",
                               "existing_id": r.id}))
                .next("get_receipt"));
            }
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "receipt".into(),
                id: r.id.clone(),
                from: json!({"amount": stated}),
                to: json!({"cancelled": true}),
                note: Some("Cancelled to be issued again for the new amount".into()),
            });
            checks.push(Check {
                rule: "receipt.after_claim".into(),
                passed: true,
                detail: Some(format!(
                    "The order's amount changed from {stated} to {amount}; the receipt is issued again"
                )),
                fix_tool: None,
            });
            replaces = Some(r.id.clone());
        }
        changes.push(Change {
            op: ChangeOp::Create,
            entity: "receipt".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"order_id": o.id, "receipt_type": kind, "amount": amount,
                       "tax_id": tax_id}),
            note: (kind != ReceiptType::Proof).then(|| {
                "Recorded here; the tax service is not connected, so it is not filed".to_owned()
            }),
        });
        let basis = standing
            .iter()
            .filter(|r| Some(&r.id) == replaces.as_ref())
            .map(|r| Basis {
                entity: "sales.receipt".into(),
                id: r.id.clone(),
                version: i64::from(r.version),
            })
            .collect();
        Ok(Planned {
            changes,
            checks,
            basis,
            approval: approval(cx, Self::META.tool, &json!({"amount": amount})),
            plan: ReceiptPlan {
                receipt_id,
                order_id: o.id,
                receipt_type: kind,
                tax_id,
                amount,
                replaces,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ReceiptPlan) -> Result<Committed, ToolError> {
        let now = tx.clock.now();
        if let Some(old) = &plan.replaces {
            repo::cancel_receipt(tx, old, now).await?;
        }
        repo::insert_receipt(
            tx,
            &plan.receipt_id,
            &plan.order_id,
            plan.receipt_type.as_str(),
            plan.tax_id.as_deref(),
            plan.amount,
            now,
        )
        .await?;
        let mut produced = Map::new();
        produced.insert("receipt_id".into(), json!(plan.receipt_id));
        produced.insert("order_id".into(), json!(plan.order_id));
        produced.insert("receipt_type".into(), json!(plan.receipt_type));
        produced.insert("amount".into(), json!(plan.amount));
        produced.insert("tax_filing".into(), tax_filing(plan.receipt_type.as_str()));
        Ok(Committed {
            id: plan.receipt_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ReceiptPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::receipt(&mut tx, &plan.receipt_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = r.as_ref().is_some_and(|r| {
            r.cancelled_at.is_none()
                && r.receipt_type == plan.receipt_type.as_str()
                && r.total_payment_due_value == plan.amount.value
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                vec![]
            } else {
                vec![format!("receipt {} is not as issued", plan.receipt_id)]
            },
        })
    }

    async fn undo(&self, _: &mut TxCx, plan: &ReceiptPlan, _: &Committed) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "Receipt {} was issued and is not taken back; a changed amount is issued again.",
                plan.receipt_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed_with_types_as_words() {
        let s = closed_schema_for::<IssueReceiptInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["order_id", "receipt_type"]));
        assert_eq!(
            s["properties"]["receipt_type"]["enum"],
            json!(["deduction", "proof", "tax_invoice"])
        );
    }

    #[tokio::test]
    async fn a_tax_invoice_needs_the_business_tax_id() {
        let cx = crate::testdb::offline("cus_alex");
        let e = IssueReceipt
            .preview(
                &cx,
                serde_json::from_value(json!({"order_id": "ord_20402",
                    "receipt_type": "tax_invoice", "tax_id": "  "}))
                .unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("business_id_required", Some("issue_receipt"))
        );
    }

    /// Integration (`DATABASE_URL`, demo seed): Alex's #20402 was ordered in October, so in
    /// November no receipt can be issued for it (`receipt.same_month_only`); Jamie's #20381
    /// already has its proof receipt.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn receipts_follow_the_month_and_stand_once() {
        let alex = crate::testdb::cx("cus_alex").await;
        let e = IssueReceipt
            .preview(
                &alex,
                serde_json::from_value(json!({"order_id": "ord_20402", "receipt_type": "proof"}))
                    .unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["rule"], "receipt.same_month_only");
        // In October the same receipt can be issued.
        let october = crate::testdb::cx_at("cus_alex", "2026-10-30T12:00").await;
        let planned = IssueReceipt
            .preview(
                &october,
                serde_json::from_value(
                    json!({"order_id": "ord_20402", "receipt_type": "deduction"}),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            planned.changes[0].to["amount"]["value"].to_string(),
            "312.00"
        );
        let mut tx = october.begin().await.unwrap();
        let done = IssueReceipt.commit(&mut tx, &planned.plan).await.unwrap();
        assert_eq!(done.produced["tax_filing"]["connected"], false);
        drop(tx); // rolled back
        let jamie = crate::testdb::cx_at("cus_jamie", "2026-10-30T12:00").await;
        let e = IssueReceipt
            .preview(
                &jamie,
                serde_json::from_value(json!({"order_id": "ord_20381", "receipt_type": "proof"}))
                    .unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "already_issued");
    }
}
