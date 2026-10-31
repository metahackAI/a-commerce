//! `create_admin_user` (`platform.admin_user.create`): a new operator account with roles and an
//! organization. A person with rights appears, so a person always approves it. No tool handles
//! passwords: the account starts without one.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{DUPLICATE_EMAIL, INVALID_ROLE, admin};
use crate::org::{ORG_NOT_FOUND, not_found as org_not_found, org};
use crate::repo::db;
use crate::role::{person_rights, role, scopes_of, tidy};
use crate::rules::AdminUserRules;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAdminUserInput {
    /// The person's full name.
    #[schemars(length(min = 1, max = 100), example = &"Kim Minji")]
    pub name: String,
    /// The email the person signs in and is reached with.
    #[schemars(length(min = 3, max = 254), example = &"minji.kim@northwind.example")]
    pub email: String,
    /// The roles the account starts with.
    #[schemars(length(min = 1), example = ["rol_operator"])]
    pub role_ids: Vec<String>,
    /// The organization the person belongs to.
    #[schemars(regex(pattern = r"^org_[a-z0-9_]+$"), example = &"org_operations")]
    pub org_id: Option<String>,
}

/// What `create_admin_user` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAdminUserPlan {
    pub admin_user_id: String,
    pub name: String,
    pub email: String,
    pub role_ids: Vec<String>,
    pub organization_id: Option<String>,
}

/// Make an operator account.
pub struct CreateAdminUser;

impl Operation for CreateAdminUser {
    const META: OpMeta = OpMeta {
        address: "platform.admin_user.create",
        tool: "create_admin_user",
        group: Group::Operate,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/platform/admin-users",
        summary: "Make an operator account with roles and an organization. A platform administrator always approves it; the person sets a password at first sign-in, never through a tool.",
        consumes: &[Slot::many("role_id"), Slot::one("org_id")],
        produces: &[Slot::one("admin_user_id")],
        checks: &["roles.known_roles", "admin_users.email_unique"],
        errors: &[DUPLICATE_EMAIL, INVALID_ROLE, ORG_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateAdminUserInput;
    type Output = Value;
}

fn duplicate(email: &str, existing: &str) -> ToolError {
    ToolError::new(
        DUPLICATE_EMAIL,
        format!("{email} already signs in as {existing}."),
    )
    .detail(json!({"entity": "admin_user", "field": "email", "existing_id": existing}))
    .next("search_admin_users")
}

async fn with_email(tx: &mut TxCx, email: &str) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM platform.admin_user WHERE tenant_id = $1 AND lower(email) = lower($2)",
    )
    .bind(tx.tenant.as_str())
    .bind(email)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A plausible address: one `@`, something before it, a dotted domain after it.
fn email_ok(email: &str) -> bool {
    match email.split_once('@') {
        Some((name, domain)) => {
            !name.is_empty()
                && !domain.contains('@')
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !email.chars().any(char::is_whitespace)
        }
        None => false,
    }
}

impl WriteOp<Cx> for CreateAdminUser {
    type Plan = CreateAdminUserPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateAdminUserInput,
    ) -> Result<Planned<CreateAdminUserPlan>, ToolError> {
        crate::require_admin(cx, Some("list_roles"))?;
        let name = crate::required_text("name", &input.name)?;
        let email = input.email.trim().to_lowercase();
        if !email_ok(&email) {
            return Err(ToolError::invalid_input(format!(
                "`{}` is not an email address.",
                input.email
            ))
            .detail(json!({"field": "email", "reason": "format"})));
        }
        let role_ids = tidy(&input.role_ids);
        if role_ids.is_empty() {
            return Err(
                ToolError::invalid_input("An account needs at least one role.")
                    .detail(json!({"field": "role_ids", "reason": "missing"}))
                    .next("list_roles"),
            );
        }
        let rules = AdminUserRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let existing = with_email(&mut tx, &email).await?;
        if rules.email_unique
            && let Some(existing) = &existing
        {
            return Err(duplicate(&email, existing));
        }
        let mut role_names = Vec::new();
        let mut scopes = Vec::new();
        for id in &role_ids {
            let Some(r) = role(&mut tx, id).await? else {
                return Err(ToolError::new(
                    INVALID_ROLE,
                    format!("{id} is not one of the store's roles."),
                )
                .detail(json!({"field": "role_ids", "reason": "enum", "id": id,
                               "rule": "roles.known_roles"}))
                .next("list_roles"));
            };
            role_names.push(r.name.clone());
            scopes.extend(scopes_of(&mut tx, id).await?);
        }
        let organization = match &input.org_id {
            Some(o) => Some(org(&mut tx, o).await?.ok_or_else(|| org_not_found(o))?),
            None => None,
        };
        // The id is the email's name part: minji.kim@… → adm_minji_kim.
        let local = email.split('@').next().unwrap_or_default();
        let base = format!("adm_{}", crate::slug(local));
        let mut admin_user_id = base.clone();
        for n in 2..50 {
            if admin(&mut tx, &admin_user_id).await?.is_none() {
                break;
            }
            admin_user_id = format!("{base}_{n}");
        }
        tx.commit().await.map_err(ToolError::from)?;
        let rights = person_rights(&tidy(&scopes));
        let reason = format!(
            "Creates an account for {name} with {}{}",
            role_names.join(", "),
            if rights.is_empty() {
                String::new()
            } else {
                format!(" (person-only rights: {})", rights.join(", "))
            }
        );
        let checks = vec![
            Check {
                rule: "roles.known_roles".into(),
                passed: true,
                detail: Some(format!(
                    "{} existing role(s): {}",
                    role_ids.len(),
                    role_names.join(", ")
                )),
                fix_tool: None,
            },
            Check {
                rule: "admin_users.email_unique".into(),
                passed: true,
                detail: Some(if rules.email_unique {
                    format!("No other account uses {email}")
                } else {
                    "The store allows shared emails".into()
                }),
                fix_tool: None,
            },
        ];
        let plan = CreateAdminUserPlan {
            admin_user_id,
            name,
            email,
            role_ids,
            organization_id: organization.map(|o| o.id),
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "admin_user".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"admin_user_id": plan.admin_user_id, "name": plan.name,
                           "email": plan.email, "roles": plan.role_ids,
                           "org_id": plan.organization_id, "status": "active"}),
                note: None,
            }],
            checks,
            basis: Vec::new(),
            approval: ApprovalNeed::Needed { reason },
            plan,
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &CreateAdminUserPlan,
    ) -> Result<Committed, ToolError> {
        match admin(tx, &plan.admin_user_id).await? {
            Some(a) if a.email != plan.email => {
                return Err(ToolError::new(
                    ErrorCode::INVALID_STATE,
                    format!(
                        "{} was taken by another account; preview again.",
                        plan.admin_user_id
                    ),
                )
                .detail(json!({"entity": "admin_user", "id": plan.admin_user_id}))
                .next("create_admin_user"));
            }
            Some(_) => {}
            None => {
                if let Some(existing) = with_email(tx, &plan.email).await? {
                    return Err(duplicate(&plan.email, &existing));
                }
                sqlx::query(
                    "INSERT INTO platform.admin_user (tenant_id, id, name, email, admin_status,
                       organization_id) VALUES ($1, $2, $3, $4, 'active', $5)",
                )
                .bind(tx.tenant.as_str())
                .bind(&plan.admin_user_id)
                .bind(&plan.name)
                .bind(&plan.email)
                .bind(plan.organization_id.as_deref())
                .execute(&mut *tx.tx)
                .await
                .map_err(db)?;
                sqlx::query(
                    "INSERT INTO platform.admin_user_role (tenant_id, admin_user_id, role_id)
                     SELECT $1, $2, unnest($3::text[]) ON CONFLICT DO NOTHING",
                )
                .bind(tx.tenant.as_str())
                .bind(&plan.admin_user_id)
                .bind(&plan.role_ids)
                .execute(&mut *tx.tx)
                .await
                .map_err(db)?;
            }
        }
        let mut produced = Map::new();
        produced.insert("admin_user_id".into(), json!(plan.admin_user_id));
        // Email delivery is outside this system: the invitation is a stub that says so.
        produced.insert(
            "invitation".into(),
            json!({"channel": "email", "to": plan.email, "sent": false,
                   "note": "Email delivery is a stub in this build: no message was sent. The account has no password until the person sets one at first sign-in."}),
        );
        Ok(Committed {
            id: plan.admin_user_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CreateAdminUserPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = admin(&mut tx, &plan.admin_user_id).await?;
        let mut roles: Vec<String> = sqlx::query_scalar(
            "SELECT role_id FROM platform.admin_user_role
             WHERE tenant_id = $1 AND admin_user_id = $2 ORDER BY role_id",
        )
        .bind(cx.tenant.as_str())
        .bind(&plan.admin_user_id)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        roles.sort();
        let mut mismatches = Vec::new();
        match a {
            None => mismatches.push(format!("{} does not exist", plan.admin_user_id)),
            Some(a) => {
                if a.email != plan.email || a.organization_id != plan.organization_id {
                    mismatches.push(format!("{} is not as previewed", plan.admin_user_id));
                }
            }
        }
        if roles != plan.role_ids {
            mismatches.push(format!(
                "{} has roles {} instead of {}",
                plan.admin_user_id,
                roles.join(", "),
                plan.role_ids.join(", ")
            ));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    /// Takes the account away again while it has never signed in.
    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &CreateAdminUserPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        let Some(a) = admin(tx, &plan.admin_user_id).await? else {
            return Ok(());
        };
        if a.last_login_at.is_some() {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} has signed in since it was made; suspend it instead.",
                    plan.admin_user_id
                ),
            )
            .detail(json!({"entity": "admin_user", "id": plan.admin_user_id}))
            .next("set_admin_status"));
        }
        for sql in [
            "DELETE FROM platform.menu_favorite WHERE tenant_id = $1 AND admin_user_id = $2",
            "DELETE FROM platform.admin_user_role WHERE tenant_id = $1 AND admin_user_id = $2",
            "DELETE FROM platform.admin_user WHERE tenant_id = $1 AND id = $2",
        ] {
            sqlx::query(sql)
                .bind(tx.tenant.as_str())
                .bind(&plan.admin_user_id)
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

    fn input(v: Value) -> CreateAdminUserInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_input_is_closed_and_emails_are_checked() {
        let s = closed_schema_for::<CreateAdminUserInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["name", "email", "role_ids"]));
        assert!(email_ok("minji.kim@northwind.example"));
        assert!(!email_ok("minji@localhost"));
        assert!(!email_ok("min ji@northwind.example"));
        assert!(!email_ok("@northwind.example"));
    }

    #[tokio::test]
    async fn a_bad_email_is_refused_before_any_read() {
        let cx = crate::testdb::offline("adm_admin", acom_auth::scopes::admin());
        let e = CreateAdminUser
            .preview(
                &cx,
                input(json!({"name": "Kim", "email": "kim", "role_ids": ["rol_operator"]})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["field"], "email");
    }

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn makes_an_account_with_roles_and_takes_it_back() {
        let cx = crate::testdb::admin().await;
        let e = CreateAdminUser
            .preview(
                &cx,
                input(json!({"name": "Someone", "email": "SJ@northwind.example",
                             "role_ids": ["rol_operator"]})),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("duplicate_email", Some("search_admin_users"))
        );
        let e = CreateAdminUser
            .preview(
                &cx,
                input(
                    json!({"name": "Someone", "email": "someone@northwind.example",
                             "role_ids": ["rol_nobody"]}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_role");
        let p = CreateAdminUser
            .preview(
                &cx,
                input(
                    json!({"name": "Test Person", "email": "test.person@northwind.example",
                             "role_ids": ["rol_operator"], "org_id": "org_operations"}),
                ),
            )
            .await
            .unwrap();
        assert_eq!(p.plan.admin_user_id, "adm_test_person");
        let ApprovalNeed::Needed { reason } = &p.approval else {
            panic!("always approved")
        };
        assert!(
            reason.contains("Operator") && reason.contains("approve"),
            "{reason}"
        );
        assert!(p.checks.iter().all(|c| c.passed));
        let mut tx = cx.begin().await.unwrap();
        let done = CreateAdminUser.commit(&mut tx, &p.plan).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(done.produced["invitation"]["sent"], false);
        assert!(
            CreateAdminUser
                .verify(&cx, &p.plan, &done)
                .await
                .unwrap()
                .matched
        );
        let mut tx = cx.begin().await.unwrap();
        CreateAdminUser.undo(&mut tx, &p.plan, &done).await.unwrap();
        tx.commit().await.unwrap();
        assert!(
            !CreateAdminUser
                .verify(&cx, &p.plan, &done)
                .await
                .unwrap()
                .matched
        );
    }
}
