//! `block_member`: stop a member from using the store (`active`/`dormant` → `blocked`). A
//! sanction that reaches the customer, so a person always approves it.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{StatePlan, account_basis, account_or_not_found, verify_planned, write_planned};
use crate::access::{always, day, parse_time, staff_only};
use crate::repo::{self, StateFields};
use crate::{ALREADY_BLOCKED, CUSTOMER_NOT_FOUND, MEMBER_NOT_ACTIVE};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlockMemberInput {
    /// The member (`search_members`).
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_7710")]
    pub customer_id: String,
    /// Why, in a sentence (`member.block_reason_required`).
    #[schemars(length(min = 1, max = 500), example = &"Repeated chargebacks on three orders.")]
    pub reason: String,
    /// When the block ends, store time. No end when left out.
    #[schemars(example = &"2026-12-31T23:59")]
    pub block_until: Option<String>,
}

pub struct BlockMember;

impl Operation for BlockMember {
    const META: OpMeta = OpMeta {
        address: "customer.person.block",
        tool: "block_member",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/customer/people/{customer_id}/block",
        summary: "Block a member from using the store, with a reason and an optional end. A person always approves; unblock_member lifts it.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::one("customer_id")],
        checks: &["member.block_reason_required"],
        errors: &[ALREADY_BLOCKED, CUSTOMER_NOT_FOUND, MEMBER_NOT_ACTIVE],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = BlockMemberInput;
    type Output = Value;
}

impl WriteOp<Cx> for BlockMember {
    type Plan = StatePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: BlockMemberInput,
    ) -> Result<Planned<StatePlan>, ToolError> {
        staff_only(cx)?;
        let reason = input.reason.trim().to_owned();
        if reason.is_empty() {
            return Err(ToolError::invalid_input("A block needs a reason.")
                .detail(json!({"field": "reason", "reason": "missing",
                               "rule": "member.block_reason_required"}))
                .next("block_member"));
        }
        let until = parse_time(cx, "block_until", input.block_until.as_deref())?;
        if until.is_some_and(|u| u <= cx.clock.now()) {
            return Err(ToolError::invalid_input("`block_until` is in the past.")
                .detail(json!({"field": "block_until", "reason": "past"})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = account_or_not_found(&mut tx, &input.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        match a.member_state.as_str() {
            "active" | "dormant" => {}
            "blocked" => {
                return Err(ToolError::new(
                    ALREADY_BLOCKED,
                    format!("{} is already blocked.", a.id),
                )
                .detail(json!({"entity": "person", "from": "blocked"}))
                .next("get_member"));
            }
            other => {
                return Err(ToolError::new(
                    MEMBER_NOT_ACTIVE,
                    format!(
                        "{} is {}; only active or dormant members can be blocked.",
                        a.id,
                        other.replace('_', " ")
                    ),
                )
                .detail(json!({"entity": "person", "from": other}))
                .next("get_member"));
            }
        }
        let before = StateFields::from(&a);
        let after = StateFields {
            member_state: "blocked".into(),
            block_reason: Some(reason.clone()),
            blocked_until: until,
        };
        let why = match until {
            Some(u) => format!("Blocks a member until {}", day(cx.time, u)),
            None => "Blocks a member with no end date".to_owned(),
        };
        let facts = json!({"customer_id": a.id});
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "person".into(),
                id: a.id.clone(),
                from: json!(before.member_state),
                to: json!("blocked"),
                note: until.map(|u| format!("Until {}", day(cx.time, u))),
            }],
            checks: vec![Check {
                rule: "member.block_reason_required".into(),
                passed: true,
                detail: Some(format!("Reason recorded: {reason}")),
                fix_tool: None,
            }],
            basis: vec![account_basis(&a)],
            approval: always(cx, Self::META.tool, &facts, why),
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
        let s = closed_schema_for::<BlockMemberInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(BlockMember::META.kind, Kind::WriteApproval);
    }
}
