//! `set_org` (`platform.org.set`): make an organization or change its name, kind or parent.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::OrganizationType;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{CYCLE_DETECTED, ORG_NOT_FOUND, OrgRow, not_found, org, subtree};
use crate::repo::db;
use crate::rules::OrgRules;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetOrgInput {
    /// The organization to change. Leave it out to make a new one (an organization with the same
    /// name is changed instead).
    #[schemars(regex(pattern = r"^org_[a-z0-9_]+$"), example = &"org_merchandising")]
    pub org_id: Option<String>,
    /// The organization's name, unique in the store.
    #[schemars(length(min = 1, max = 100), example = &"Merchandising")]
    pub name: String,
    /// What it is (department: a team of the store; vendor: an outside company people work for;
    /// branch: a site of the store).
    #[schemars(example = &"department")]
    pub org_type: OrganizationType,
    /// The organization it belongs to; never itself or one of its own sub-organizations.
    #[schemars(regex(pattern = r"^org_[a-z0-9_]+$"), example = &"org_northwind")]
    pub parent_org_id: Option<String>,
}

/// One organization's fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgFields {
    pub name: String,
    pub org_type: OrganizationType,
    pub parent_org_id: Option<String>,
}

/// What `set_org` runs; `before` is `None` for a new organization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetOrgPlan {
    pub org_id: String,
    pub to: OrgFields,
    pub before: Option<OrgFields>,
}

/// Make or change an organization.
pub struct SetOrg;

impl Operation for SetOrg {
    const META: OpMeta = OpMeta {
        address: "platform.org.set",
        tool: "set_org",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Put,
        path: "/v1/platform/orgs/{org_id}",
        summary: "Make an organization (department, vendor, branch) or change its name, kind or parent. Administrators only.",
        consumes: &[Slot::one("org_id")],
        produces: &[Slot::one("org_id")],
        checks: &["orgs.no_cycle"],
        errors: &[CYCLE_DETECTED, ORG_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetOrgInput;
    type Output = Value;
}

fn fields(o: &OrgRow) -> Result<OrgFields, ToolError> {
    Ok(OrgFields {
        name: o.name.clone(),
        org_type: o
            .organization_type
            .parse()
            .map_err(|e: acom_core::enums::UnknownWord| {
                ToolError::new(ErrorCode::INTERNAL, e.to_string())
            })?,
        parent_org_id: o.parent_organization_id.clone(),
    })
}

fn shown(f: &OrgFields) -> Value {
    json!({"name": f.name, "org_type": f.org_type, "parent_org_id": f.parent_org_id})
}

async fn named(tx: &mut TxCx, name: &str) -> Result<Option<OrgRow>, ToolError> {
    sqlx::query_as(
        "SELECT id, name, organization_type, parent_organization_id, version
         FROM platform.organization WHERE tenant_id = $1 AND lower(name) = lower($2)",
    )
    .bind(tx.tenant.as_str())
    .bind(name)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

async fn write(tx: &mut TxCx, id: &str, f: &OrgFields) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO platform.organization (tenant_id, id, name, organization_type,
           parent_organization_id) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (tenant_id, id) DO UPDATE SET name = EXCLUDED.name,
           organization_type = EXCLUDED.organization_type,
           parent_organization_id = EXCLUDED.parent_organization_id,
           updated_at = now(), version = platform.organization.version + 1",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&f.name)
    .bind(f.org_type.as_str())
    .bind(f.parent_org_id.as_deref())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

impl WriteOp<Cx> for SetOrg {
    type Plan = SetOrgPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(&self, cx: &Cx, input: SetOrgInput) -> Result<Planned<SetOrgPlan>, ToolError> {
        crate::require_admin(cx, Some("list_roles"))?;
        let name = crate::required_text("name", &input.name)?;
        let rules = OrgRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let current = match &input.org_id {
            Some(id) => org(&mut tx, id).await?,
            None => named(&mut tx, &name).await?,
        };
        let org_id = match (&current, &input.org_id) {
            (Some(o), _) => o.id.clone(),
            (None, Some(id)) => id.clone(),
            (None, None) => format!("org_{}", crate::slug(&name)),
        };
        if let Some(other) = named(&mut tx, &name).await?
            && other.id != org_id
        {
            return Err(ToolError::invalid_input(format!(
                "{} is already the name of {}; change that one or pick another name.",
                name, other.id
            ))
            .detail(json!({"field": "name", "reason": "taken", "existing_id": other.id}))
            .next("list_orgs"));
        }
        let mut check_detail = "No parent: a top-level organization".to_owned();
        if let Some(parent) = &input.parent_org_id {
            let p = org(&mut tx, parent)
                .await?
                .ok_or_else(|| not_found(parent))?;
            // orgs.no_cycle: the parent may not be the organization or anything under it.
            let under = if current.is_some() {
                subtree(&mut tx, &org_id).await?
            } else {
                vec![org_id.clone()]
            };
            if rules.no_cycle && under.contains(parent) {
                return Err(ToolError::new(
                    CYCLE_DETECTED,
                    format!(
                        "{} cannot belong to {}: that is {} itself or one of its own sub-organizations.",
                        name, p.name, name
                    ),
                )
                .detail(json!({"rule": "orgs.no_cycle", "items": [org_id, parent]}))
                .next("list_orgs"));
            }
            check_detail = format!("{} is not under {}", p.name, name);
        }
        tx.commit().await.map_err(ToolError::from)?;
        let to = OrgFields {
            name,
            org_type: input.org_type,
            parent_org_id: input.parent_org_id,
        };
        let before = current.as_ref().map(fields).transpose()?;
        if before.as_ref() == Some(&to) {
            return Err(ToolError::invalid_input(format!(
                "Nothing to change: {org_id} already has these values."
            ))
            .detail(json!({"field": "org_id", "reason": "unchanged"})));
        }
        let change = Change {
            op: if before.is_some() {
                ChangeOp::Update
            } else {
                ChangeOp::Create
            },
            entity: "org".into(),
            id: if before.is_some() {
                org_id.clone()
            } else {
                String::new()
            },
            from: before.as_ref().map_or(Value::Null, shown),
            to: {
                let mut v = shown(&to);
                v["org_id"] = json!(org_id);
                v
            },
            note: None,
        };
        let basis = current
            .iter()
            .map(|o| Basis {
                entity: "platform.organization".into(),
                id: o.id.clone(),
                version: i64::from(o.version),
            })
            .collect();
        Ok(Planned {
            changes: vec![change],
            checks: vec![Check {
                rule: "orgs.no_cycle".into(),
                passed: true,
                detail: Some(check_detail),
                fix_tool: None,
            }],
            basis,
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan: SetOrgPlan { org_id, to, before },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SetOrgPlan) -> Result<Committed, ToolError> {
        write(tx, &plan.org_id, &plan.to).await?;
        let mut produced = Map::new();
        produced.insert("org_id".into(), json!(plan.org_id));
        Ok(Committed {
            id: plan.org_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SetOrgPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let o = org(&mut tx, &plan.org_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let now = o.as_ref().map(fields).transpose()?;
        let matched = now.as_ref() == Some(&plan.to);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not as previewed", plan.org_id)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SetOrgPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        if let Some(before) = &plan.before {
            return write(tx, &plan.org_id, before).await;
        }
        let used: i64 = sqlx::query_scalar(
            "SELECT (SELECT count(*) FROM platform.admin_user
                     WHERE tenant_id = $1 AND organization_id = $2)
                  + (SELECT count(*) FROM platform.organization
                     WHERE tenant_id = $1 AND parent_organization_id = $2)",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.org_id)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        if used > 0 {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} has people or sub-organizations now; move them first.",
                    plan.org_id
                ),
            )
            .detail(json!({"entity": "org", "id": plan.org_id}))
            .next("list_orgs"));
        }
        sqlx::query("DELETE FROM platform.organization WHERE tenant_id = $1 AND id = $2")
            .bind(tx.tenant.as_str())
            .bind(&plan.org_id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn input(v: Value) -> SetOrgInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_input_is_closed_with_kind_words() {
        let s = closed_schema_for::<SetOrgInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["name", "org_type"]));
        assert_eq!(
            s["properties"]["org_type"]["enum"],
            json!(["department", "vendor", "branch"])
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn refuses_cycles_makes_an_org_and_takes_it_back() {
        let cx = crate::testdb::admin().await;
        let e = SetOrg
            .preview(
                &cx,
                input(
                    json!({"org_id": "org_northwind", "name": "Northwind Apparel",
                             "org_type": "department", "parent_org_id": "org_merchandising"}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("cycle_detected", Some("list_orgs"))
        );
        let e = SetOrg
            .preview(
                &cx,
                input(json!({"name": "Night shift", "org_type": "branch",
                             "parent_org_id": "org_nowhere"})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "org_not_found");
        let p = SetOrg
            .preview(
                &cx,
                input(json!({"name": "Test night shift", "org_type": "branch",
                             "parent_org_id": "org_operations"})),
            )
            .await
            .unwrap();
        assert_eq!(p.plan.org_id, "org_test_night_shift");
        assert_eq!(p.changes[0].op, ChangeOp::Create);
        assert!(p.checks[0].passed);
        let mut tx = cx.begin().await.unwrap();
        let done = SetOrg.commit(&mut tx, &p.plan).await.unwrap();
        tx.commit().await.unwrap();
        assert!(SetOrg.verify(&cx, &p.plan, &done).await.unwrap().matched);
        let mut tx = cx.begin().await.unwrap();
        SetOrg.undo(&mut tx, &p.plan, &done).await.unwrap();
        tx.commit().await.unwrap();
        assert!(!SetOrg.verify(&cx, &p.plan, &done).await.unwrap().matched);
    }
}
