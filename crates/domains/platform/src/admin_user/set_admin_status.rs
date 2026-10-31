//! `set_admin_status` (`platform.admin_user.set_status`): suspend, unlock or restore an operator
//! account. A person always approves it.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::AdminStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{ADMIN_USER_NOT_FOUND, ALREADY_IN_STATE, admin, not_found};
use crate::repo::db;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetAdminStatusInput {
    /// The operator account.
    #[schemars(regex(pattern = r"^adm_[a-z0-9_]+$"), example = &"adm_jordan")]
    pub admin_user_id: String,
    /// The new state (active: can sign in; suspended: stopped by an administrator; locked:
    /// stopped as after too many wrong passwords).
    #[schemars(example = &"active")]
    pub status: AdminStatus,
    /// Why, in a sentence; kept in the action log.
    #[schemars(length(min = 1, max = 500), example = &"Back from leave.")]
    pub reason: String,
}

/// What `set_admin_status` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetAdminStatusPlan {
    pub admin_user_id: String,
    pub from: AdminStatus,
    pub to: AdminStatus,
    pub reason: String,
}

/// Change whether an account can sign in.
pub struct SetAdminStatus;

impl Operation for SetAdminStatus {
    const META: OpMeta = OpMeta {
        address: "platform.admin_user.set_status",
        tool: "set_admin_status",
        group: Group::Operate,
        kind: Kind::WriteApproval,
        method: Method::Put,
        path: "/v1/platform/admin-users/{admin_user_id}/status",
        summary: "Suspend, unlock or restore an operator account, with a reason. A platform administrator always approves it.",
        consumes: &[Slot::one("admin_user_id")],
        produces: &[Slot::one("admin_user_id")],
        checks: &[],
        errors: &[ALREADY_IN_STATE, ADMIN_USER_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetAdminStatusInput;
    type Output = Value;
}

fn status_of(s: &str) -> Result<AdminStatus, ToolError> {
    s.parse().map_err(|e: acom_core::enums::UnknownWord| {
        ToolError::new(acom_contract::ErrorCode::INTERNAL, e.to_string())
    })
}

async fn set(tx: &mut TxCx, id: &str, status: AdminStatus) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE platform.admin_user SET admin_status = $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND admin_status <> $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(status.as_str())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

impl WriteOp<Cx> for SetAdminStatus {
    type Plan = SetAdminStatusPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetAdminStatusInput,
    ) -> Result<Planned<SetAdminStatusPlan>, ToolError> {
        crate::require_admin(cx, Some("list_roles"))?;
        let reason = crate::required_text("reason", &input.reason)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = admin(&mut tx, &input.admin_user_id)
            .await?
            .ok_or_else(|| not_found(&input.admin_user_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        let from = status_of(&a.admin_status)?;
        if from == input.status {
            return Err(ToolError::new(
                ALREADY_IN_STATE,
                format!("{}'s account is already {}.", a.name, from),
            )
            .detail(json!({"entity": "admin_user", "id": a.id, "from": from}))
            .next("search_admin_users"));
        }
        let effect = match input.status {
            AdminStatus::Active => "can sign in again",
            AdminStatus::Suspended | AdminStatus::Locked => "can no longer sign in",
        };
        let plan = SetAdminStatusPlan {
            admin_user_id: a.id.clone(),
            from,
            to: input.status,
            reason,
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "admin_user".into(),
                id: a.id.clone(),
                from: json!({"status": from}),
                to: json!({"status": input.status}),
                note: Some(plan.reason.clone()),
            }],
            checks: Vec::new(),
            basis: vec![Basis {
                entity: "platform.admin_user".into(),
                id: a.id,
                version: i64::from(a.version),
            }],
            approval: ApprovalNeed::Needed {
                reason: format!("Makes {}'s account {}: they {effect}", a.name, input.status),
            },
            plan,
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &SetAdminStatusPlan,
    ) -> Result<Committed, ToolError> {
        // Asking again for the state the account is already in succeeds.
        set(tx, &plan.admin_user_id, plan.to).await?;
        let mut produced = Map::new();
        produced.insert("admin_user_id".into(), json!(plan.admin_user_id));
        produced.insert("status".into(), json!(plan.to));
        produced.insert("reason".into(), json!(plan.reason));
        Ok(Committed {
            id: plan.admin_user_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SetAdminStatusPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = admin(&mut tx, &plan.admin_user_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let now = a.map(|a| a.admin_status);
        let matched = now.as_deref() == Some(plan.to.as_str());
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} is {} instead of {}",
                    plan.admin_user_id,
                    now.unwrap_or_else(|| "missing".into()),
                    plan.to
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SetAdminStatusPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        set(tx, &plan.admin_user_id, plan.from).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn input(v: Value) -> SetAdminStatusInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_input_is_closed_with_status_words() {
        let s = closed_schema_for::<SetAdminStatusInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["properties"]["status"]["enum"],
            json!(["active", "suspended", "locked"])
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn restores_a_suspended_account_and_puts_it_back() {
        let cx = crate::testdb::admin().await;
        let e = SetAdminStatus
            .preview(
                &cx,
                input(json!({"admin_user_id": "adm_sj", "status": "active", "reason": "x"})),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("already_in_state", Some("search_admin_users"))
        );
        let p = SetAdminStatus
            .preview(
                &cx,
                input(json!({"admin_user_id": "adm_jordan", "status": "active",
                             "reason": "Back for the holiday season."})),
            )
            .await
            .unwrap();
        assert_eq!(p.changes[0].from, json!({"status": "suspended"}));
        let ApprovalNeed::Needed { reason } = &p.approval else {
            panic!("always approved")
        };
        assert!(reason.contains("can sign in again"), "{reason}");
        let mut tx = cx.begin().await.unwrap();
        let done = SetAdminStatus.commit(&mut tx, &p.plan).await.unwrap();
        tx.commit().await.unwrap();
        assert!(
            SetAdminStatus
                .verify(&cx, &p.plan, &done)
                .await
                .unwrap()
                .matched
        );
        let mut tx = cx.begin().await.unwrap();
        SetAdminStatus.undo(&mut tx, &p.plan, &done).await.unwrap();
        tx.commit().await.unwrap();
        assert!(
            !SetAdminStatus
                .verify(&cx, &p.plan, &done)
                .await
                .unwrap()
                .matched
        );
    }
}
