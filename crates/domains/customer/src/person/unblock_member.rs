//! `unblock_member`: lift a block (`blocked` → `active`). Same approval rule as blocking.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{StatePlan, account_basis, account_or_not_found, verify_planned, write_planned};
use crate::access::{always, staff_only};
use crate::repo::{self, StateFields};
use crate::{CUSTOMER_NOT_FOUND, NOT_BLOCKED};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnblockMemberInput {
    /// The blocked member (`search_members` with `state: blocked`).
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_7710")]
    pub customer_id: String,
    /// Why the block ends, in a sentence (`member.block_reason_required`).
    #[schemars(length(min = 1, max = 500), example = &"Chargebacks resolved with the bank.")]
    pub reason: String,
}

pub struct UnblockMember;

impl Operation for UnblockMember {
    const META: OpMeta = OpMeta {
        address: "customer.person.unblock",
        tool: "unblock_member",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/customer/people/{customer_id}/unblock",
        summary: "Lift a member's block so they can use the store again, with a reason. A person always approves.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::one("customer_id")],
        checks: &["member.block_reason_required"],
        errors: &[NOT_BLOCKED, CUSTOMER_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UnblockMemberInput;
    type Output = Value;
}

impl WriteOp<Cx> for UnblockMember {
    type Plan = StatePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UnblockMemberInput,
    ) -> Result<Planned<StatePlan>, ToolError> {
        staff_only(cx)?;
        let reason = input.reason.trim().to_owned();
        if reason.is_empty() {
            return Err(ToolError::invalid_input("Lifting a block needs a reason.")
                .detail(json!({"field": "reason", "reason": "missing",
                               "rule": "member.block_reason_required"}))
                .next("unblock_member"));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = account_or_not_found(&mut tx, &input.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if a.member_state != "blocked" {
            return Err(ToolError::new(
                NOT_BLOCKED,
                format!(
                    "{} is {}, not blocked.",
                    a.id,
                    a.member_state.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "person", "from": a.member_state}))
            .next("get_member"));
        }
        let before = StateFields::from(&a);
        let after = StateFields {
            member_state: "active".into(),
            block_reason: None,
            blocked_until: None,
        };
        let facts = json!({"customer_id": a.id});
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "person".into(),
                id: a.id.clone(),
                from: json!("blocked"),
                to: json!("active"),
                note: None,
            }],
            checks: vec![Check {
                rule: "member.block_reason_required".into(),
                passed: true,
                detail: Some(format!("Reason recorded: {reason}")),
                fix_tool: None,
            }],
            basis: vec![account_basis(&a)],
            approval: always(cx, Self::META.tool, &facts, "Unblocks a member".into()),
            plan: StatePlan {
                customer_id: a.id,
                before,
                after,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StatePlan) -> Result<Committed, ToolError> {
        write_planned(tx, plan).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &StatePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        verify_planned(cx, plan).await
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &StatePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        repo::write_state(tx, &plan.customer_id, &plan.before).await
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_always_waits_for_a_person() {
        let s = closed_schema_for::<UnblockMemberInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(UnblockMember::META.kind, Kind::WriteApproval);
    }
}
