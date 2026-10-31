//! `approve_remedy`: approve or reject a requested remedy (`requested` → `approved` |
//! `rejected`). A rejected remedy is not edited; a new request is made instead
//! (`remedy.resubmit_on_reject`).

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{RemedyState, decider, dollars, remedy, remedy_not_found, write_state};
use crate::access::{always, policy_words, staff_only};
use crate::{ALREADY_DECIDED, REMEDY_NOT_FOUND};

/// The decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RemedyDecision {
    /// Approve it; cash still needs the payout step (`pay_remedy`).
    Approve,
    /// Reject it; needs `reason`.
    Reject,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApproveRemedyInput {
    /// The remedy (`request_remedy`).
    #[schemars(regex(pattern = r"^rmd_[a-z0-9_]+$"), example = &"rmd_310")]
    pub remedy_id: String,
    /// approve or reject.
    #[schemars(example = &"approve")]
    pub decision: RemedyDecision,
    /// Why it is rejected. Required with `decision: reject`.
    #[schemars(length(max = 500), example = &"Delivery was on time per the carrier scan.")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovePlan {
    pub remedy_id: String,
    pub before: RemedyState,
    /// `approved` or `rejected`.
    pub decision: String,
    pub reason: Option<String>,
}

pub struct ApproveRemedy;

impl Operation for ApproveRemedy {
    const META: OpMeta = OpMeta {
        address: "customer.remedy.approve",
        tool: "approve_remedy",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/customer/remedies/{remedy_id}/approve",
        summary: "Approve or reject a requested remedy. Always a person's decision; cash also needs the payout approver when it is paid.",
        consumes: &[Slot::one("remedy_id")],
        produces: &[Slot::one("remedy_id")],
        checks: &["remedy.approval_levels"],
        errors: &[ALREADY_DECIDED, REMEDY_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::AUTHORIZE,
        person_scope: "",
    };
    type Input = ApproveRemedyInput;
    type Output = Value;
}

impl WriteOp<Cx> for ApproveRemedy {
    type Plan = ApprovePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ApproveRemedyInput,
    ) -> Result<Planned<ApprovePlan>, ToolError> {
        staff_only(cx)?;
        let reason = input
            .reason
            .map(|r| r.trim().to_owned())
            .filter(|r| !r.is_empty());
        if input.decision == RemedyDecision::Reject && reason.is_none() {
            return Err(ToolError::invalid_input("Say why the remedy is rejected.")
                .detail(json!({"field": "reason", "reason": "missing"}))
                .next("approve_remedy"));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = remedy(&mut tx, &input.remedy_id)
            .await?
            .ok_or_else(|| remedy_not_found(&input.remedy_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        if r.remedy_status != "requested" {
            return Err(ToolError::new(
                ALREADY_DECIDED,
                format!("{} is already {}.", r.id, r.remedy_status),
            )
            .detail(json!({"entity": "remedy", "from": r.remedy_status}))
            .next("approve_remedy"));
        }
        let decision = match input.decision {
            RemedyDecision::Approve => "approved",
            RemedyDecision::Reject => "rejected",
        };
        let amount = r.amount();
        let levels = policy_words(cx, "customer.remedy.approval_levels");
        let cash = r.remedy_type == "cash";
        let detail = match (cash, input.decision) {
            (_, RemedyDecision::Reject) => {
                "Rejected; a new request is the way to ask again".to_owned()
            }
            (true, _) if levels.iter().any(|l| l == "payout_approver") => {
                "Cash: the payout approver signs off when it is paid (pay_remedy)".to_owned()
            }
            (false, _) => format!(
                "{}, no payout step",
                crate::wording::capitalize(&r.remedy_type.replace('_', " "))
            ),
            (true, _) => "Cash, paid with pay_remedy".to_owned(),
        };
        let verb = if decision == "approved" {
            "Approves"
        } else {
            "Rejects"
        };
        let facts = json!({"remedy_type": r.remedy_type, "amount": amount});
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "remedy".into(),
                id: r.id.clone(),
                from: json!("requested"),
                to: json!(decision),
                note: reason.clone(),
            }],
            checks: vec![Check {
                rule: "remedy.approval_levels".into(),
                passed: true,
                detail: Some(detail),
                fix_tool: None,
            }],
            basis: vec![Basis {
                entity: "customer.remedy".into(),
                id: r.id.clone(),
                version: i64::from(r.version),
            }],
            approval: always(
                cx,
                Self::META.tool,
                &facts,
                format!(
                    "{verb} a {} remedy in {}",
                    dollars(amount),
                    r.remedy_type.replace('_', " ")
                ),
            ),
            plan: ApprovePlan {
                remedy_id: r.id.clone(),
                before: RemedyState::from(&r),
                decision: decision.into(),
                reason,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ApprovePlan) -> Result<Committed, ToolError> {
        let r = remedy(tx, &plan.remedy_id)
            .await?
            .ok_or_else(|| remedy_not_found(&plan.remedy_id))?;
        let mut produced = Map::new();
        produced.insert("remedy_id".into(), json!(plan.remedy_id));
        // A retry of the same decision returns the first one.
        if r.remedy_status == plan.decision {
            return Ok(Committed {
                id: plan.remedy_id.clone(),
                produced,
            });
        }
        if r.remedy_status != "requested" {
            return Err(ToolError::new(
                ALREADY_DECIDED,
                format!("{} was {} in the meantime.", r.id, r.remedy_status),
            )
            .next("approve_remedy"));
        }
        let after = RemedyState {
            status: plan.decision.clone(),
            decided_by_id: Some(decider(tx)),
            decided_at: Some(tx.clock.now()),
            decision_reason: plan.reason.clone(),
            ..plan.before.clone()
        };
        write_state(tx, &plan.remedy_id, &after).await?;
        Ok(Committed {
            id: plan.remedy_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ApprovePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = remedy(&mut tx, &plan.remedy_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            Some(r) if r.remedy_status == plan.decision => vec![],
            Some(r) => vec![format!("{} is {}", r.id, r.remedy_status)],
            None => vec![format!("{} is gone", plan.remedy_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ApprovePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        let r = remedy(tx, &plan.remedy_id)
            .await?
            .ok_or_else(|| remedy_not_found(&plan.remedy_id))?;
        if r.remedy_status != plan.decision {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} is {} now; it can no longer go back to requested.",
                    r.id, r.remedy_status
                ),
            )
            .detail(json!({"entity": "remedy", "from": r.remedy_status})));
        }
        write_state(tx, &plan.remedy_id, &plan.before).await
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_always_waits_for_a_person() {
        let s = closed_schema_for::<ApproveRemedyInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(ApproveRemedy::META.kind, Kind::WriteApproval);
        assert_eq!(
            s["properties"]["decision"]["enum"],
            json!(["approve", "reject"])
        );
    }
}
