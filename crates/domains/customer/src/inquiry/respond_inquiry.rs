//! `respond_inquiry`: reply to the customer in an inquiry's thread. It reaches the customer, so
//! it always waits for a person (like `send_customer_message`). The reply is recorded in the
//! thread; delivering it is messaging's job, which this domain cannot call, so the result says
//! it was recorded and not delivered.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::ActorKind;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{NewComment, add_comment, inquiry, inquiry_not_found, new_comment_id, thread};
use crate::access::{always, staff_only};
use crate::{INQUIRY_CLOSED, INQUIRY_NOT_FOUND};

/// How the reply reaches the customer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReplyChannel {
    /// By email to the customer's address.
    Email,
    /// In the customer's account, under the inquiry.
    InApp,
}

impl ReplyChannel {
    fn word(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::InApp => "in_app",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RespondInquiryInput {
    /// The inquiry (`search_inquiries`).
    #[schemars(regex(pattern = r"^inq_[a-z0-9_]+$"), example = &"inq_771")]
    pub inquiry_id: String,
    /// The reply exactly as the customer will read it.
    #[schemars(
        length(min = 1, max = 4000),
        example = &"Sorry about the sole. A refund is being reviewed today."
    )]
    pub text: String,
    /// email or in_app: where the customer reads it.
    #[schemars(example = &"email")]
    pub channel: ReplyChannel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespondPlan {
    pub inquiry_id: String,
    pub comment: NewComment,
}

pub struct RespondInquiry;

impl Operation for RespondInquiry {
    const META: OpMeta = OpMeta {
        address: "customer.inquiry.respond",
        tool: "respond_inquiry",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/customer/inquiries/{inquiry_id}/respond",
        summary: "Reply to the customer in an inquiry (email or in app). Reaches the customer, so a person always approves it; a sent reply cannot be taken back.",
        consumes: &[Slot::one("inquiry_id")],
        produces: &[Slot::one("comment_id")],
        checks: &["customer_messages"],
        errors: &[INQUIRY_CLOSED, INQUIRY_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEND,
        person_scope: "",
    };
    type Input = RespondInquiryInput;
    type Output = Value;
}

impl WriteOp<Cx> for RespondInquiry {
    type Plan = RespondPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RespondInquiryInput,
    ) -> Result<Planned<RespondPlan>, ToolError> {
        staff_only(cx)?;
        let text = input.text.trim().to_owned();
        if text.is_empty() {
            return Err(ToolError::invalid_input("The reply is empty.")
                .detail(json!({"field": "text", "reason": "missing"})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = inquiry(&mut tx, &input.inquiry_id)
            .await?
            .ok_or_else(|| inquiry_not_found(&input.inquiry_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        if row.inquiry_status == "resolved" {
            return Err(ToolError::new(
                INQUIRY_CLOSED,
                format!("{} is resolved; open a new inquiry to write again.", row.id),
            )
            .detail(json!({"entity": "inquiry", "from": "resolved"}))
            .next("create_inquiry"));
        }
        let author = if cx.principal.kind == ActorKind::AgentOnBehalf {
            "agent"
        } else {
            "staff"
        };
        let channel = input.channel.word();
        let facts = json!({"inquiry_id": row.id, "reaches_customers": true});
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Send,
                entity: "comment".into(),
                id: String::new(),
                from: Value::Null,
                to: json!(text),
                note: Some(format!("By {channel} to {}", row.customer_id)),
            }],
            checks: vec![Check {
                rule: "customer_messages".into(),
                passed: true,
                detail: Some("Shown as the customer will read it".into()),
                fix_tool: None,
            }],
            basis: vec![Basis {
                entity: "customer.customer_inquiry".into(),
                id: row.id.clone(),
                version: i64::from(row.version),
            }],
            approval: always(
                cx,
                Self::META.tool,
                &facts,
                "Sends a reply to a customer".into(),
            ),
            plan: RespondPlan {
                inquiry_id: row.id,
                comment: NewComment {
                    id: new_comment_id(),
                    author_type: author.into(),
                    agent_id: cx.principal.subject.clone(),
                    text,
                    message_channel: Some(channel.into()),
                },
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &RespondPlan) -> Result<Committed, ToolError> {
        let row = inquiry(tx, &plan.inquiry_id)
            .await?
            .ok_or_else(|| inquiry_not_found(&plan.inquiry_id))?;
        if row.inquiry_status == "resolved" {
            return Err(ToolError::new(
                INQUIRY_CLOSED,
                format!("{} was resolved before the reply went out.", row.id),
            )
            .next("create_inquiry"));
        }
        add_comment(tx, &plan.inquiry_id, &plan.comment).await?;
        let mut produced = Map::new();
        produced.insert("comment_id".into(), json!(plan.comment.id));
        produced.insert(
            "delivery".into(),
            json!({"delivered": false,
                   "note": "Recorded in the inquiry thread. Delivery to the customer is not connected yet, so nothing was sent."}),
        );
        Ok(Committed {
            id: plan.comment.id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RespondPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let comments = thread(&mut tx, &plan.inquiry_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = if comments.iter().any(|c| c.id == plan.comment.id) {
            vec![]
        } else {
            vec![format!("{} is not in the thread", plan.comment.id)]
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        _plan: &RespondPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        // Not reached: a reply to a customer cannot be taken back (UNDO is NotSupported).
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_always_waits_for_a_person_and_cannot_be_undone() {
        let s = closed_schema_for::<RespondInquiryInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(RespondInquiry::META.kind, Kind::WriteApproval);
        assert_eq!(RespondInquiry::UNDO, UndoSupport::NotSupported);
        assert_eq!(
            s["properties"]["channel"]["enum"],
            json!(["email", "in_app"])
        );
    }
}
