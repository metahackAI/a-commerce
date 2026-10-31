//! `assign_role` (`platform.role.assign`): give an operator account a role, or take it away.
//! Rights change, so a person always approves it in a change set.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    ALREADY_ASSIGNED, NOT_ASSIGNED, ROLE_NOT_FOUND, not_found, person_rights, role, scopes_of,
};
use crate::admin_user::{ADMIN_USER_NOT_FOUND, admin, not_found as admin_not_found};
use crate::repo::db;
use crate::rules::{LeastPrivilege, RoleRules};

/// Give or take away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AssignAction {
    /// The account gets the role.
    Grant,
    /// The account loses the role.
    Revoke,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssignRoleInput {
    /// The role.
    #[schemars(regex(pattern = r"^rol_[a-z0-9_]+$"), example = &"rol_operator")]
    pub role_id: String,
    /// The operator account.
    #[schemars(regex(pattern = r"^adm_[a-z0-9_]+$"), example = &"adm_mina")]
    pub admin_user_id: String,
    /// grant: give the role; revoke: take it away.
    #[schemars(example = &"grant")]
    pub action: AssignAction,
}

/// What `assign_role` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssignRolePlan {
    pub role_id: String,
    pub admin_user_id: String,
    pub action: AssignAction,
}

/// Give or take away a role.
pub struct AssignRole;

impl Operation for AssignRole {
    const META: OpMeta = OpMeta {
        address: "platform.role.assign",
        tool: "assign_role",
        group: Group::Operate,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/platform/roles/{role_id}/assignments",
        summary: "Give an operator account a role or take it away. Rights change, so a platform administrator always approves it.",
        consumes: &[Slot::one("role_id"), Slot::one("admin_user_id")],
        produces: &[Slot::one("role_id")],
        checks: &["roles.least_privilege"],
        errors: &[
            ALREADY_ASSIGNED,
            NOT_ASSIGNED,
            ROLE_NOT_FOUND,
            ADMIN_USER_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::AUTHORIZE,
        person_scope: "",
    };
    type Input = AssignRoleInput;
    type Output = Value;
}

async fn holds(tx: &mut TxCx, admin_user_id: &str, role_id: &str) -> Result<bool, ToolError> {
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform.admin_user_role
         WHERE tenant_id = $1 AND admin_user_id = $2 AND role_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(admin_user_id)
    .bind(role_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(n > 0)
}

async fn apply(
    tx: &mut TxCx,
    plan: &AssignRolePlan,
    action: AssignAction,
) -> Result<(), ToolError> {
    let sql = match action {
        AssignAction::Grant => {
            "INSERT INTO platform.admin_user_role (tenant_id, admin_user_id, role_id)
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"
        }
        AssignAction::Revoke => {
            "DELETE FROM platform.admin_user_role
             WHERE tenant_id = $1 AND admin_user_id = $2 AND role_id = $3"
        }
    };
    sqlx::query(sql)
        .bind(tx.tenant.as_str())
        .bind(&plan.admin_user_id)
        .bind(&plan.role_id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}

impl WriteOp<Cx> for AssignRole {
    type Plan = AssignRolePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: AssignRoleInput,
    ) -> Result<Planned<AssignRolePlan>, ToolError> {
        crate::require_admin(cx, Some("list_roles"))?;
        let rules = RoleRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = role(&mut tx, &input.role_id)
            .await?
            .ok_or_else(|| not_found(&input.role_id))?;
        let a = admin(&mut tx, &input.admin_user_id)
            .await?
            .ok_or_else(|| admin_not_found(&input.admin_user_id))?;
        let scopes = scopes_of(&mut tx, &r.id).await?;
        let has = holds(&mut tx, &a.id, &r.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        match (input.action, has) {
            (AssignAction::Grant, true) => {
                return Err(ToolError::new(
                    ALREADY_ASSIGNED,
                    format!("{} already has the {} role.", a.name, r.name),
                )
                .detail(json!({"entity": "role", "id": r.id, "admin_user_id": a.id}))
                .next("get_role"));
            }
            (AssignAction::Revoke, false) => {
                return Err(ToolError::new(
                    NOT_ASSIGNED,
                    format!("{} does not have the {} role.", a.name, r.name),
                )
                .detail(json!({"entity": "role", "id": r.id, "admin_user_id": a.id,
                               "from": "not_assigned"}))
                .next("get_role"));
            }
            _ => {}
        }
        // roles.least_privilege: person-only rights are named where the approver reads.
        let rights = person_rights(&scopes);
        let warn = rules.least_privilege == LeastPrivilege::WarnOnHumanScopes && !rights.is_empty();
        let rights_text = if warn {
            format!(" (person-only rights: {})", rights.join(", "))
        } else {
            String::new()
        };
        let reason = match input.action {
            AssignAction::Grant => {
                format!(
                    "Gives {} rights to 1 person: {}{rights_text}",
                    r.name, a.name
                )
            }
            AssignAction::Revoke => {
                format!(
                    "Takes {} rights from 1 person: {}{rights_text}",
                    r.name, a.name
                )
            }
        };
        let check = Check {
            rule: "roles.least_privilege".into(),
            passed: true,
            detail: Some(if rights.is_empty() {
                format!("{} carries no person-only rights", r.name)
            } else {
                format!(
                    "{} carries person-only rights ({}); named in the approval reason",
                    r.name,
                    rights.join(", ")
                )
            }),
            fix_tool: None,
        };
        let plan = AssignRolePlan {
            role_id: r.id.clone(),
            admin_user_id: a.id.clone(),
            action: input.action,
        };
        Ok(Planned {
            changes: vec![Change {
                op: match input.action {
                    AssignAction::Grant => ChangeOp::Grant,
                    AssignAction::Revoke => ChangeOp::Revoke,
                },
                entity: "role".into(),
                id: r.id.clone(),
                from: Value::Null,
                to: json!({"admin_user_id": a.id, "name": a.name, "role": r.name,
                           "scopes": scopes}),
                note: None,
            }],
            checks: vec![check],
            basis: vec![
                Basis {
                    entity: "platform.role".into(),
                    id: r.id,
                    version: i64::from(r.version),
                },
                Basis {
                    entity: "platform.admin_user".into(),
                    id: a.id,
                    version: i64::from(a.version),
                },
            ],
            approval: ApprovalNeed::Needed { reason },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &AssignRolePlan) -> Result<Committed, ToolError> {
        apply(tx, plan, plan.action).await?;
        let mut produced = Map::new();
        produced.insert("role_id".into(), json!(plan.role_id));
        produced.insert("admin_user_id".into(), json!(plan.admin_user_id));
        produced.insert("action".into(), json!(plan.action));
        Ok(Committed {
            id: plan.role_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &AssignRolePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let has = holds(&mut tx, &plan.admin_user_id, &plan.role_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let want = plan.action == AssignAction::Grant;
        Ok(Verification {
            matched: has == want,
            mismatches: if has == want {
                Vec::new()
            } else {
                vec![format!(
                    "{} {} {}",
                    plan.admin_user_id,
                    if has { "still has" } else { "does not have" },
                    plan.role_id
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &AssignRolePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        let back = match plan.action {
            AssignAction::Grant => AssignAction::Revoke,
            AssignAction::Revoke => AssignAction::Grant,
        };
        apply(tx, plan, back).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn input(v: Value) -> AssignRoleInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_input_is_closed_with_a_word_for_the_action() {
        let s = closed_schema_for::<AssignRoleInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["properties"]["action"]["enum"],
            json!(["grant", "revoke"])
        );
        assert_eq!(AssignRole::META.kind, Kind::WriteApproval);
    }

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn grants_with_a_reason_naming_person_only_rights_and_takes_back() {
        let cx = crate::testdb::admin().await;
        let p = AssignRole
            .preview(
                &cx,
                input(json!({"role_id": "rol_operator", "admin_user_id": "adm_sj",
                             "action": "grant"})),
            )
            .await
            .unwrap();
        let ApprovalNeed::Needed { reason } = &p.approval else {
            panic!("always approved")
        };
        assert!(
            reason.starts_with("Gives Operator rights to 1 person"),
            "{reason}"
        );
        assert!(reason.contains("approve"), "{reason}");
        assert_eq!(p.changes[0].op, ChangeOp::Grant);
        let e = AssignRole
            .preview(
                &cx,
                input(
                    json!({"role_id": "rol_merchandiser", "admin_user_id": "adm_sj",
                             "action": "grant"}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("already_assigned", Some("get_role"))
        );
        let e = AssignRole
            .preview(
                &cx,
                input(json!({"role_id": "rol_operator", "admin_user_id": "adm_sj",
                             "action": "revoke"})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "not_assigned");
        let e = AssignRole
            .preview(
                &cx,
                input(
                    json!({"role_id": "rol_operator", "admin_user_id": "adm_ghost",
                             "action": "grant"}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("admin_user_not_found", Some("search_admin_users"))
        );
        // Run, read back, undo: the account ends as it was.
        let mut tx = cx.begin().await.unwrap();
        let done = AssignRole.commit(&mut tx, &p.plan).await.unwrap();
        tx.commit().await.unwrap();
        assert!(
            AssignRole
                .verify(&cx, &p.plan, &done)
                .await
                .unwrap()
                .matched
        );
        let mut tx = cx.begin().await.unwrap();
        AssignRole.undo(&mut tx, &p.plan, &done).await.unwrap();
        tx.commit().await.unwrap();
        assert!(
            !AssignRole
                .verify(&cx, &p.plan, &done)
                .await
                .unwrap()
                .matched
        );
    }
}
