//! `close_inquiry`: mark an inquiry resolved (`in_progress` → `resolved`). With
//! `customer_service.notify_member_on_resolve`, a transactional "resolved" notice is recorded
//! for the customer (delivery is not connected here; it says so).

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    InquiryState, NewComment, add_comment, inquiry, inquiry_not_found, new_comment_id, write_state,
};
use crate::INQUIRY_NOT_FOUND;
use crate::access::{approval, policy, staff_only};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseInquiryInput {
    /// The inquiry (`search_inquiries`).
    #[schemars(regex(pattern = r"^inq_[a-z0-9_]+$"), example = &"inq_771")]
    pub inquiry_id: String,
    /// How it was resolved, in a sentence the customer may read.
    #[schemars(length(max = 1000), example = &"Refunded in full; the boots need not be sent back.")]
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosePlan {
    pub inquiry_id: String,
    pub before: InquiryState,
    pub after: InquiryState,
    /// The resolved notice for the customer, when the policy asks for one.
    pub notice: Option<NewComment>,
}

pub struct CloseInquiry;

impl Operation for CloseInquiry {
    const META: OpMeta = OpMeta {
        address: "customer.inquiry.close",
        tool: "close_inquiry",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/inquiries/{inquiry_id}/close",
        summary: "Close an inquiry as resolved, with how it was resolved; the customer gets a resolved notice when the store's policy says so.",
        consumes: &[Slot::one("inquiry_id")],
        produces: &[Slot::one("inquiry_id")],
        checks: &["customer_service.notify_member_on_resolve"],
        errors: &[ErrorCode::INVALID_STATE, INQUIRY_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = CloseInquiryInput;
    type Output = Value;
}

impl WriteOp<Cx> for CloseInquiry {
    type Plan = ClosePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CloseInquiryInput,
    ) -> Result<Planned<ClosePlan>, ToolError> {
        staff_only(cx)?;
        let notify = policy(cx, "customer.customer_service.notify_member_on_resolve")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = inquiry(&mut tx, &input.inquiry_id)
            .await?
            .ok_or_else(|| inquiry_not_found(&input.inquiry_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        let before = InquiryState::from(&row);
        let resolution = input
            .resolution
            .map(|r| r.trim().to_owned())
            .filter(|r| !r.is_empty());
        let (changes, after, notice, detail) = match row.inquiry_status.as_str() {
            // Closing again converges: nothing changes.
            "resolved" => (
                vec![],
                before.clone(),
                None,
                "Already resolved; nothing is sent again".to_owned(),
            ),
            "in_progress" => {
                let after = InquiryState {
                    status: "resolved".into(),
                    resolution: resolution.clone().or_else(|| before.resolution.clone()),
                    resolved_at: Some(cx.clock.now()),
                    ..before.clone()
                };
                let notice = notify.then(|| NewComment {
                    id: new_comment_id(),
                    author_type: "staff".into(),
                    agent_id: cx.principal.subject.clone(),
                    text: match &resolution {
                        Some(r) => format!("Your inquiry is resolved: {r}"),
                        None => "Your inquiry is resolved.".to_owned(),
                    },
                    message_channel: Some("email".into()),
                });
                let detail = if notify {
                    format!("{} gets a resolved notice by email", row.customer_id)
                } else {
                    "No notice: the store does not notify on resolve".to_owned()
                };
                (
                    vec![Change {
                        op: ChangeOp::Update,
                        entity: "customer_inquiry".into(),
                        id: row.id.clone(),
                        from: json!("in_progress"),
                        to: json!("resolved"),
                        note: resolution.clone(),
                    }],
                    after,
                    notice,
                    detail,
                )
            }
            other => {
                return Err(ToolError::new(
                    ErrorCode::INVALID_STATE,
                    format!(
                        "{} is {other}; assign it to an operator before closing it.",
                        row.id
                    ),
                )
                .detail(json!({"entity": "inquiry", "from": other, "allowed_to": ["in_progress"]}))
                .next("get_inquiry"));
            }
        };
        let facts = json!({"inquiry_id": row.id});
        Ok(Planned {
            changes,
            checks: vec![Check {
                rule: "customer_service.notify_member_on_resolve".into(),
                passed: true,
                detail: Some(detail),
                fix_tool: None,
            }],
            basis: vec![Basis {
                entity: "customer.customer_inquiry".into(),
                id: row.id.clone(),
                version: i64::from(row.version),
            }],
            approval: approval(cx, Self::META.tool, &facts),
            plan: ClosePlan {
                inquiry_id: row.id,
                before,
                after,
                notice,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ClosePlan) -> Result<Committed, ToolError> {
        if inquiry(tx, &plan.inquiry_id).await?.is_none() {
            return Err(inquiry_not_found(&plan.inquiry_id));
        }
        write_state(tx, &plan.inquiry_id, &plan.after).await?;
        let mut produced = Map::new();
        produced.insert("inquiry_id".into(), json!(plan.inquiry_id));
        if let Some(n) = &plan.notice {
            add_comment(tx, &plan.inquiry_id, n).await?;
            produced.insert(
                "notice".into(),
                json!({"comment_id": n.id, "delivered": false,
                       "note": "Recorded in the thread; delivery to the customer is not connected yet, so nothing was sent."}),
            );
        }
        Ok(Committed {
            id: plan.inquiry_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ClosePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = inquiry(&mut tx, &plan.inquiry_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            Some(r) if r.inquiry_status == plan.after.status => vec![],
            Some(r) => vec![format!("{} is {}", r.id, r.inquiry_status)],
            None => vec![format!("{} is gone", plan.inquiry_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ClosePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        // The thread is append-only: a recorded notice stays; the inquiry reopens.
        write_state(tx, &plan.inquiry_id, &plan.before).await
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<CloseInquiryInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(CloseInquiry::META.kind, Kind::Write);
    }
}
