//! `create_role` (`platform.role.create`): a store-defined role, a named bundle of scopes.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, Group, Kind, Method, OpMeta, Operation,
    Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{DUPLICATE_ROLE, INVALID_SCOPE, known_scopes, person_rights, role, scopes_of, tidy};
use crate::repo::db;
use crate::rules::RoleRules;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateRoleInput {
    /// The role's name, unique in the store.
    #[schemars(length(min = 1, max = 60), example = &"Returns desk")]
    pub name: String,
    /// The scopes the role grants, from list_permissions.
    #[schemars(length(min = 1), example = ["tools:read:fulfillment", "tools:write:fulfillment", "human:approve"])]
    pub scopes: Vec<String>,
    /// The approver roles (approval.yml `approver_role`) people with this role may approve as.
    #[schemars(example = ["operator"])]
    pub approver_roles: Option<Vec<String>>,
    /// What people with this role do, in a sentence.
    #[schemars(length(max = 300), example = &"Reviews return requests and refunds under the threshold.")]
    pub description: Option<String>,
}

/// What `create_role` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateRolePlan {
    pub role_id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub approver_roles: Vec<String>,
    pub description: Option<String>,
}

/// Make a role.
pub struct CreateRole;

impl Operation for CreateRole {
    const META: OpMeta = OpMeta {
        address: "platform.role.create",
        tool: "create_role",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/platform/roles",
        summary: "Make a store-defined role from scopes in list_permissions. Giving it to people is assign_role, which a person approves.",
        consumes: &[],
        produces: &[Slot::one("role_id")],
        checks: &["roles.known_scopes"],
        errors: &[DUPLICATE_ROLE, INVALID_SCOPE],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateRoleInput;
    type Output = Value;
}

fn duplicate(name: &str, existing: &str) -> ToolError {
    ToolError::new(
        DUPLICATE_ROLE,
        format!("A role named {name} already exists ({existing})."),
    )
    .detail(json!({"entity": "role", "field": "name", "existing_id": existing}))
    .next("list_roles")
}

async fn named(tx: &mut TxCx, name: &str) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM platform.role WHERE tenant_id = $1 AND lower(name) = lower($2)",
    )
    .bind(tx.tenant.as_str())
    .bind(name)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

impl WriteOp<Cx> for CreateRole {
    type Plan = CreateRolePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateRoleInput,
    ) -> Result<Planned<CreateRolePlan>, ToolError> {
        crate::require_admin(cx, Some("list_roles"))?;
        let name = crate::required_text("name", &input.name)?;
        let scopes = tidy(&input.scopes);
        if scopes.is_empty() {
            return Err(ToolError::invalid_input("A role needs at least one scope.")
                .detail(json!({"field": "scopes", "reason": "missing"}))
                .next("list_permissions"));
        }
        let known = known_scopes(cx, &scopes)?;
        let rules = RoleRules::from_policy(&cx.policy)?;
        let approver_roles = tidy(&input.approver_roles.unwrap_or_default());
        if let Some(bad) = approver_roles
            .iter()
            .find(|r| !rules.known_roles.contains(r))
        {
            return Err(ToolError::invalid_input(format!(
                "`{bad}` is not an approver role; use one of {}.",
                rules.known_roles.join(", ")
            ))
            .detail(json!({"field": "approver_roles", "reason": "enum",
                           "rule": "roles.known_roles", "allowed": rules.known_roles})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if let Some(existing) = named(&mut tx, &name).await? {
            return Err(duplicate(&name, &existing));
        }
        let base = format!("rol_{}", crate::slug(&name));
        let mut role_id = base.clone();
        for n in 2..50 {
            if role(&mut tx, &role_id).await?.is_none() {
                break;
            }
            role_id = format!("{base}_{n}");
        }
        tx.commit().await.map_err(ToolError::from)?;
        let description = input
            .description
            .map(|d| d.trim().to_owned())
            .filter(|d| !d.is_empty());
        let rights = person_rights(&scopes);
        let mut checks = vec![known];
        if !rights.is_empty() {
            checks[0].detail = Some(format!(
                "{}; the person-only rights ({}) work only on the person's own clicks",
                checks[0].detail.clone().unwrap_or_default(),
                rights.join(", ")
            ));
        }
        let plan = CreateRolePlan {
            role_id,
            name,
            scopes,
            approver_roles,
            description,
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "role".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"role_id": plan.role_id, "name": plan.name, "scopes": plan.scopes,
                           "approver_roles": plan.approver_roles, "description": plan.description}),
                note: None,
            }],
            checks,
            basis: Vec::new(),
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &CreateRolePlan) -> Result<Committed, ToolError> {
        match named(tx, &plan.name).await? {
            Some(existing) if existing != plan.role_id => {
                return Err(duplicate(&plan.name, &existing));
            }
            Some(_) => {}
            None => {
                sqlx::query(
                    "INSERT INTO platform.role (tenant_id, id, name, description, approver_roles,
                       is_system) VALUES ($1, $2, $3, $4, $5, false)",
                )
                .bind(tx.tenant.as_str())
                .bind(&plan.role_id)
                .bind(&plan.name)
                .bind(plan.description.as_deref())
                .bind(&plan.approver_roles)
                .execute(&mut *tx.tx)
                .await
                .map_err(db)?;
                sqlx::query(
                    "INSERT INTO platform.role_scope (tenant_id, role_id, scope)
                     SELECT $1, $2, unnest($3::text[])",
                )
                .bind(tx.tenant.as_str())
                .bind(&plan.role_id)
                .bind(&plan.scopes)
                .execute(&mut *tx.tx)
                .await
                .map_err(db)?;
            }
        }
        let mut produced = Map::new();
        produced.insert("role_id".into(), json!(plan.role_id));
        Ok(Committed {
            id: plan.role_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CreateRolePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = role(&mut tx, &plan.role_id).await?;
        let mut scopes = scopes_of(&mut tx, &plan.role_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        scopes.sort();
        let mut mismatches = Vec::new();
        match r {
            None => mismatches.push(format!("{} does not exist", plan.role_id)),
            Some(r) if r.name != plan.name => {
                mismatches.push(format!("{} is named {}", plan.role_id, r.name));
            }
            Some(_) => {}
        }
        if scopes != plan.scopes {
            mismatches.push(format!(
                "{} grants {} instead of {}",
                plan.role_id,
                scopes.join(", "),
                plan.scopes.join(", ")
            ));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &CreateRolePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        let held: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.admin_user_role WHERE tenant_id = $1 AND role_id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.role_id)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        if held > 0 {
            return Err(ToolError::new(
                acom_contract::ErrorCode::INVALID_STATE,
                format!(
                    "{} is held by {held} account(s) now; take it from them first.",
                    plan.role_id
                ),
            )
            .detail(json!({"entity": "role", "id": plan.role_id}))
            .next("assign_role"));
        }
        for sql in [
            "DELETE FROM platform.role_scope WHERE tenant_id = $1 AND role_id = $2",
            "DELETE FROM platform.role WHERE tenant_id = $1 AND id = $2 AND NOT is_system",
        ] {
            sqlx::query(sql)
                .bind(tx.tenant.as_str())
                .bind(&plan.role_id)
                .execute(&mut *tx.tx)
                .await
                .map_err(db)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn input(v: Value) -> CreateRoleInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_input_is_closed() {
        let s = closed_schema_for::<CreateRoleInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["name", "scopes"]));
    }

    #[tokio::test]
    async fn unknown_scopes_and_approver_roles_are_refused_before_any_read() {
        let cx = crate::testdb::offline("adm_admin", acom_auth::scopes::admin());
        let e = CreateRole
            .preview(
                &cx,
                input(json!({"name": "Root", "scopes": ["root:everything"]})),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("invalid_scope", Some("list_permissions"))
        );
        let e = CreateRole
            .preview(
                &cx,
                input(json!({"name": "Desk", "scopes": ["tools:read:sales"],
                             "approver_roles": ["superuser"]})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["field"], "approver_roles");
        let sj = crate::testdb::offline("adm_sj", ["tools:read:platform".to_owned()].into());
        let e = CreateRole
            .preview(
                &sj,
                input(json!({"name": "Desk", "scopes": ["tools:read:sales"]})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn makes_a_role_reads_it_back_and_takes_it_back() {
        let cx = crate::testdb::admin().await;
        let p = CreateRole
            .preview(
                &cx,
                input(json!({"name": "Returns desk test",
                    "scopes": ["tools:write:fulfillment", "tools:read:fulfillment", "human:approve"],
                    "approver_roles": ["operator"]})),
            )
            .await
            .unwrap();
        assert_eq!(p.plan.role_id, "rol_returns_desk_test");
        assert_eq!(p.approval, acom_contract::ApprovalNeed::NotNeeded);
        assert!(p.checks[0].passed);
        assert!(p.checks[0].detail.as_deref().unwrap().contains("approve"));
        let mut tx = cx.begin().await.unwrap();
        let done = CreateRole.commit(&mut tx, &p.plan).await.unwrap();
        tx.commit().await.unwrap();
        let v = CreateRole.verify(&cx, &p.plan, &done).await.unwrap();
        assert!(v.matched, "{:?}", v.mismatches);
        let e = CreateRole
            .preview(
                &cx,
                input(json!({"name": "returns DESK test", "scopes": ["tools:read:sales"]})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "duplicate_role");
        let mut tx = cx.begin().await.unwrap();
        CreateRole.undo(&mut tx, &p.plan, &done).await.unwrap();
        tx.commit().await.unwrap();
        assert!(
            !CreateRole
                .verify(&cx, &p.plan, &done)
                .await
                .unwrap()
                .matched
        );
    }
}
