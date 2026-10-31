//! `create_inquiry`: open a customer service inquiry (`received`) with the customer's message.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::InquiryType;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{NewComment, add_comment, inquiry, new_comment_id};
use crate::access::{Caller, approval, caller, customer_for, db, policy_words};
use crate::person::customer_not_found;
use crate::{CUSTOMER_NOT_FOUND, INVALID_INQUIRY_TYPE, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInquiryInput {
    /// What it is about: order_status; return; product_question; account; callback; other.
    #[schemars(example = &"return")]
    pub inquiry_type: InquiryType,
    /// The customer's message, in their words.
    #[schemars(length(min = 1, max = 4000), example = &"The sole is coming apart.")]
    pub text: String,
    /// The order it is about, when there is one.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20402")]
    pub order_id: Option<String>,
    /// The customer; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInquiryPlan {
    pub inquiry_id: String,
    pub customer_id: String,
    pub inquiry_type: String,
    pub order_id: Option<String>,
    pub text: String,
    pub first: NewComment,
}

pub struct CreateInquiry;

impl Operation for CreateInquiry {
    const META: OpMeta = OpMeta {
        address: "customer.inquiry.create",
        tool: "create_inquiry",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/inquiries",
        summary: "Open a customer service inquiry (received) with the customer's message, optionally about an order.",
        consumes: &[],
        produces: &[Slot::one("inquiry_id")],
        checks: &["customer_service.inquiry_types"],
        errors: &[CUSTOMER_NOT_FOUND, INVALID_INQUIRY_TYPE],
        audience: Audience::Both,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateInquiryInput;
    type Output = Value;
}

impl WriteOp<Cx> for CreateInquiry {
    type Plan = CreateInquiryPlan;
    /// An inquiry stays on record; it is closed with `close_inquiry`, not taken back.
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateInquiryInput,
    ) -> Result<Planned<CreateInquiryPlan>, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let kind = input.inquiry_type.as_str();
        let kinds = policy_words(cx, "customer.customer_service.inquiry_types");
        if !kinds.iter().any(|k| k == kind) {
            return Err(ToolError::new(
                INVALID_INQUIRY_TYPE,
                format!("This store does not take `{kind}` inquiries."),
            )
            .detail(json!({"field": "inquiry_type", "reason": "enum", "allowed": kinds}))
            .next("list_enums"));
        }
        let text = input.text.trim().to_owned();
        if text.is_empty() {
            return Err(
                ToolError::invalid_input("The inquiry needs the customer's message.")
                    .detail(json!({"field": "text", "reason": "missing"})),
            );
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let person = repo::person(&mut tx, &customer_id)
            .await?
            .ok_or_else(|| customer_not_found(&customer_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        // The customer's own words; an operator who logs a call writes them down as staff.
        let author = match caller(&cx.principal) {
            Caller::Staff => "staff",
            _ => "customer",
        };
        let facts = json!({"customer_id": customer_id, "inquiry_type": kind});
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "customer_inquiry".into(),
                id: String::new(),
                from: Value::Null,
                to: json!("received"),
                note: input.order_id.as_ref().map(|o| format!("About order {o}")),
            }],
            checks: vec![Check {
                rule: "customer_service.inquiry_types".into(),
                passed: true,
                detail: Some(format!("{kind} is an inquiry type")),
                fix_tool: None,
            }],
            basis: vec![Basis {
                entity: "customer.person".into(),
                id: person.id.clone(),
                version: i64::from(person.version),
            }],
            approval: approval(cx, Self::META.tool, &facts),
            plan: CreateInquiryPlan {
                inquiry_id: format!("inq_{}", acom_core::id::new_suffix()),
                customer_id: person.id,
                inquiry_type: kind.to_owned(),
                order_id: input.order_id,
                first: NewComment {
                    id: new_comment_id(),
                    author_type: author.into(),
                    agent_id: cx.principal.subject.clone(),
                    text: text.clone(),
                    message_channel: None,
                },
                text,
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &CreateInquiryPlan,
    ) -> Result<Committed, ToolError> {
        sqlx::query(
            "INSERT INTO customer.customer_inquiry (tenant_id, id, customer_id, inquiry_type,
               inquiry_status, order_id, text, date_received)
             VALUES ($1, $2, $3, $4, 'received', $5, $6, $7) ON CONFLICT (tenant_id, id) DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.inquiry_id)
        .bind(&plan.customer_id)
        .bind(&plan.inquiry_type)
        .bind(plan.order_id.as_deref())
        .bind(&plan.text)
        .bind(tx.clock.now())
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        add_comment(tx, &plan.inquiry_id, &plan.first).await?;
        let mut produced = Map::new();
        produced.insert("inquiry_id".into(), json!(plan.inquiry_id));
        Ok(Committed {
            id: plan.inquiry_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CreateInquiryPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = inquiry(&mut tx, &plan.inquiry_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            Some(r) if r.customer_id == plan.customer_id => vec![],
            Some(r) => vec![format!("{} belongs to {}", r.id, r.customer_id)],
            None => vec![format!("{} was not saved", plan.inquiry_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        _plan: &CreateInquiryPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        // Not reached: UNDO is NotSupported.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_with_inquiry_type_words() {
        let s = closed_schema_for::<CreateInquiryInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        let words = s["properties"]["inquiry_type"]["enum"].as_array().unwrap();
        assert_eq!(words.len(), 6);
    }
}
