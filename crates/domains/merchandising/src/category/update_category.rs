//! `update_category` (docs/api/merchandising.md#category).

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::*;
use crate::check_id;
use crate::repo::{self, db};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCategoryInput {
    /// The category to change.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub category_id: String,
    /// New name.
    #[schemars(length(min = 1, max = 100), example = &"Light outerwear")]
    pub name: Option<String>,
    /// New position among its siblings, from 1.
    #[schemars(range(min = 1), example = 2)]
    pub display_order: Option<i32>,
    /// New parent in the same tree.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_outerwear")]
    pub parent_id: Option<String>,
    /// false switches it off (only when nothing is under it, mapped to it or placed in it);
    /// true switches it back on.
    #[schemars(example = false)]
    pub active: Option<bool>,
}

/// One category's fields, as `update_category` sets them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermFields {
    pub name: String,
    pub position: i32,
    pub parent_id: Option<String>,
    pub active: bool,
}

/// What `update_category` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCategoryPlan {
    pub category_id: String,
    pub to: TermFields,
    pub from: TermFields,
}

/// Rename, reorder, move or switch a category off.
pub struct UpdateCategory;

impl Operation for UpdateCategory {
    const META: OpMeta = OpMeta {
        address: "merchandising.category.update",
        tool: "update_category",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/merchandising/categories/{category_id}",
        summary: "Rename a category, change its position or parent, or switch it off (only when nothing is under it, mapped to it or placed in it).",
        consumes: &[Slot::one("category_id")],
        produces: &[Slot::one("category_id")],
        checks: &[
            "category.delete_only_if",
            "category.no_cycle",
            "category.max_depth",
        ],
        errors: &[
            CATEGORY_NOT_FOUND,
            HAS_CHILDREN,
            HAS_MAPPED_PRODUCTS,
            CYCLE_DETECTED,
            DEPTH_EXCEEDED,
            PARENT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateCategoryInput;
    type Output = Value;
}

async fn write_term(tx: &mut TxCx, id: &str, f: &TermFields) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE merchandising.defined_term SET name = $3, position = $4, broader_id = $5,
           is_active = $6, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&f.name)
    .bind(f.position)
    .bind(f.parent_id.as_deref())
    .bind(f.active)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

impl WriteOp<Cx> for UpdateCategory {
    type Plan = UpdateCategoryPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateCategoryInput,
    ) -> Result<Planned<UpdateCategoryPlan>, ToolError> {
        check_id("category_id", "cat", &input.category_id)?;
        let rules = CategoryRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let t = term(&mut tx, &input.category_id)
            .await?
            .ok_or_else(|| not_found(&input.category_id))?;
        let from = TermFields {
            name: t.name.clone(),
            position: t.position,
            parent_id: t.broader_id.clone(),
            active: t.is_active,
        };
        let mut to = from.clone();
        if let Some(n) = input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            to.name = n.to_owned();
        }
        if let Some(o) = input.display_order {
            to.position = o.max(1);
        }
        if let Some(a) = input.active {
            to.active = a;
        }
        let mut checks = Vec::new();
        if let Some(p) = input
            .parent_id
            .as_deref()
            .filter(|p| Some(*p) != t.broader_id.as_deref())
        {
            check_id("parent_id", "cat", p)?;
            let parent: CategoryRow = repo::category(&mut tx, p).await?.ok_or_else(|| {
                ToolError::new(PARENT_NOT_FOUND, format!("There is no category {p}."))
                    .detail(json!({"entity": "category", "id": p}))
                    .next("list_categories")
            })?;
            if parent.term_set != t.in_defined_term_set {
                return Err(ToolError::new(
                    CROSS_SET_VIOLATION,
                    format!("{p} is in the {} tree.", parent.term_set),
                )
                .detail(json!({"field": "parent_id", "term_set": parent.term_set}))
                .next("list_categories"));
            }
            let cycle = parent.id == t.id || parent.ancestors.contains(&t.id);
            checks.push(Check {
                rule: "category.no_cycle".into(),
                passed: !cycle,
                detail: Some(if cycle {
                    format!("{p} is under {}; a category cannot go under itself", t.id)
                } else {
                    format!("{p} is not under {}", t.id)
                }),
                fix_tool: cycle.then(|| "update_category".into()),
            });
            let height = subtree_height(&mut tx, &t.id).await?;
            let depth = parent.ancestors.len() + 1 + height;
            checks.push(Check {
                rule: "category.max_depth".into(),
                passed: depth <= rules.max_depth,
                detail: Some(format!(
                    "Its deepest level would be {depth}; the store allows {}",
                    rules.max_depth
                )),
                fix_tool: (depth > rules.max_depth).then(|| "update_category".into()),
            });
            to.parent_id = Some(parent.id);
        }
        if !to.active && from.active {
            checks.push(delete_check(&rules, attached(&mut tx, &t.id).await?));
        }
        tx.commit().await.map_err(ToolError::from)?;
        if to == from {
            return Err(ToolError::invalid_input(format!(
                "Nothing to change: {} already has these values.",
                t.id
            ))
            .detail(json!({"field": "category_id", "reason": "missing"})));
        }
        let (mut cf, mut ct) = (Map::new(), Map::new());
        if to.name != from.name {
            cf.insert("name".into(), json!(from.name));
            ct.insert("name".into(), json!(to.name));
        }
        if to.position != from.position {
            cf.insert("display_order".into(), json!(from.position));
            ct.insert("display_order".into(), json!(to.position));
        }
        if to.parent_id != from.parent_id {
            cf.insert("parent_id".into(), json!(from.parent_id));
            ct.insert("parent_id".into(), json!(to.parent_id));
        }
        if to.active != from.active {
            cf.insert("active".into(), json!(from.active));
            ct.insert("active".into(), json!(to.active));
        }
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "category".into(),
                id: t.id.clone(),
                from: Value::Object(cf),
                to: Value::Object(ct),
                note: (to.active && !from.active).then(|| {
                    "Categories under it show again unless they are off themselves".into()
                }),
            }],
            checks,
            basis: vec![Basis {
                entity: "merchandising.defined_term".into(),
                id: t.id.clone(),
                version: i64::from(t.version),
            }],
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan: UpdateCategoryPlan {
                category_id: t.id,
                to,
                from,
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &UpdateCategoryPlan,
    ) -> Result<Committed, ToolError> {
        if write_term(tx, &plan.category_id, &plan.to).await? == 0 {
            return Err(not_found(&plan.category_id));
        }
        let mut produced = Map::new();
        produced.insert("category_id".into(), json!(plan.category_id));
        Ok(Committed {
            id: plan.category_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &UpdateCategoryPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let t = term(&mut tx, &plan.category_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = t.is_some_and(|t| {
            t.name == plan.to.name
                && t.position == plan.to.position
                && t.broader_id == plan.to.parent_id
                && t.is_active == plan.to.active
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} does not read back as changed",
                    plan.category_id
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &UpdateCategoryPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        write_term(tx, &plan.category_id, &plan.from)
            .await
            .map(|_| ())
    }
}
