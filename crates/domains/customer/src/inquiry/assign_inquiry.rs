//! `assign_inquiry`: give an inquiry to an operator, by round robin or by name
//! (`received` → `in_progress`).

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{InquiryState, inquiry, inquiry_not_found, operator_load, write_state};
use crate::access::{approval, policy_number, policy_words, staff_only};
use crate::{INQUIRY_CLOSED, INQUIRY_NOT_FOUND, NO_AVAILABLE_AGENT};

/// How the operator is chosen (`customer_service.assignment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssignMode {
    /// The operator with the fewest open inquiries, under the per-operator limit.
    AutoRoundRobin,
    /// The operator named in `admin_user_id`.
    Manual,
}

impl AssignMode {
    fn word(self) -> &'static str {
        match self {
            Self::AutoRoundRobin => "auto_round_robin",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssignInquiryInput {
    /// The inquiry (`search_inquiries`).
    #[schemars(regex(pattern = r"^inq_[a-z0-9_]+$"), example = &"inq_771")]
    pub inquiry_id: String,
    /// auto_round_robin: the least busy operator; manual: the one in `admin_user_id`.
    #[schemars(example = &"manual")]
    pub mode: AssignMode,
    /// The operator to assign. Required with `mode: manual`.
    #[schemars(regex(pattern = r"^adm_[a-z0-9_]+$"), example = &"adm_sj")]
    pub admin_user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignPlan {
    pub inquiry_id: String,
    pub before: InquiryState,
    pub after: InquiryState,
}

pub struct AssignInquiry;

impl Operation for AssignInquiry {
    const META: OpMeta = OpMeta {
        address: "customer.inquiry.assign",
        tool: "assign_inquiry",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/inquiries/{inquiry_id}/assign",
        summary: "Assign an inquiry to an operator, automatically (least busy) or by name; it moves to in_progress.",
        consumes: &[Slot::one("inquiry_id")],
        produces: &[Slot::one("inquiry_id")],
        checks: &["customer_service.max_open_per_agent"],
        errors: &[NO_AVAILABLE_AGENT, INQUIRY_NOT_FOUND, INQUIRY_CLOSED],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = AssignInquiryInput;
    type Output = Value;
}

impl WriteOp<Cx> for AssignInquiry {
    type Plan = AssignPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: AssignInquiryInput,
    ) -> Result<Planned<AssignPlan>, ToolError> {
        staff_only(cx)?;
        let modes = policy_words(cx, "customer.customer_service.assignment");
        if !modes.iter().any(|m| m == input.mode.word()) {
            return Err(ToolError::invalid_input(format!(
                "This store does not assign inquiries by {}.",
                input.mode.word()
            ))
            .detail(json!({"field": "mode", "reason": "enum", "allowed": modes})));
        }
        let max = policy_number(cx, "customer.customer_service.max_open_per_agent")?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = inquiry(&mut tx, &input.inquiry_id)
            .await?
            .ok_or_else(|| inquiry_not_found(&input.inquiry_id))?;
        let load = operator_load(&mut tx).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if row.inquiry_status == "resolved" {
            return Err(
                ToolError::new(INQUIRY_CLOSED, format!("{} is already resolved.", row.id))
                    .detail(json!({"entity": "inquiry", "from": "resolved"}))
                    .next("create_inquiry"),
            );
        }
        let open = |adm: &str| load.iter().find(|(a, _)| a == adm).map_or(0, |(_, n)| *n);
        let chosen = match input.mode {
            AssignMode::Manual => input.admin_user_id.clone().ok_or_else(|| {
                ToolError::invalid_input("`mode: manual` needs `admin_user_id`.")
                    .detail(json!({"field": "admin_user_id", "reason": "missing"}))
                    .next("assign_inquiry")
            })?,
            AssignMode::AutoRoundRobin => load
                .iter()
                .find(|(a, n)| *n < max && row.admin_user_id.as_deref() != Some(a.as_str()))
                .map(|(a, _)| a.clone())
                .ok_or_else(|| {
                    ToolError::new(
                        NO_AVAILABLE_AGENT,
                        "No operator can take another inquiry by round robin; assign one by name.",
                    )
                    .detail(json!({"rule": "customer_service.max_open_per_agent", "limit": max}))
                    .next("assign_inquiry")
                })?,
        };
        // Taking over an inquiry already counted for the same operator adds nothing.
        let already = row.admin_user_id.as_deref() == Some(chosen.as_str())
            && row.inquiry_status == "in_progress";
        let would_have = open(&chosen) + i64::from(!already);
        let before = InquiryState::from(&row);
        let after = InquiryState {
            status: "in_progress".into(),
            admin_user_id: Some(chosen.clone()),
            assigned_at: if already {
                row.assigned_at
            } else {
                Some(cx.clock.now())
            },
            ..before.clone()
        };
        let changes = if already {
            vec![]
        } else {
            vec![Change {
                op: ChangeOp::Update,
                entity: "customer_inquiry".into(),
                id: row.id.clone(),
                from: json!({"status": row.inquiry_status, "admin_user_id": row.admin_user_id}),
                to: json!({"status": "in_progress", "admin_user_id": chosen}),
                note: None,
            }]
        };
        let facts = json!({"inquiry_id": row.id});
        Ok(Planned {
            changes,
            checks: vec![Check {
                rule: "customer_service.max_open_per_agent".into(),
                passed: would_have <= max,
                detail: Some(format!("{chosen} has {would_have} of {max} open inquiries")),
                fix_tool: (would_have > max).then(|| "assign_inquiry".to_owned()),
            }],
            basis: vec![Basis {
                entity: "customer.customer_inquiry".into(),
                id: row.id.clone(),
                version: i64::from(row.version),
            }],
            approval: approval(cx, Self::META.tool, &facts),
            plan: AssignPlan {
                inquiry_id: row.id,
                before,
                after,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &AssignPlan) -> Result<Committed, ToolError> {
        if inquiry(tx, &plan.inquiry_id).await?.is_none() {
            return Err(inquiry_not_found(&plan.inquiry_id));
        }
        write_state(tx, &plan.inquiry_id, &plan.after).await?;
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
        plan: &AssignPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = inquiry(&mut tx, &plan.inquiry_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            Some(r) if r.admin_user_id == plan.after.admin_user_id => vec![],
            Some(r) => vec![format!("{} is with {:?}", r.id, r.admin_user_id)],
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
        plan: &AssignPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        write_state(tx, &plan.inquiry_id, &plan.before).await
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_with_a_mode_word() {
        let s = closed_schema_for::<AssignInquiryInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            s["properties"]["mode"]["enum"],
            json!(["auto_round_robin", "manual"])
        );
    }
}
