//! `create_category` (docs/api/merchandising.md#category).

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::TermSet;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::*;
use crate::check_id;
use crate::repo::{self, db};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCategoryInput {
    /// Which tree. standard: the internal classification products belong to; display: the
    /// menu shoppers see.
    #[schemars(example = &"standard")]
    pub term_set: TermSet,
    /// The category's name.
    #[schemars(length(min = 1, max = 100), example = &"Cardigans")]
    pub name: String,
    /// The category above it, in the same tree. Top level when left out.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_knitwear")]
    pub parent_id: Option<String>,
    /// Position among its siblings, from 1. Default after the last one.
    #[schemars(range(min = 1), example = 3)]
    pub display_order: Option<i32>,
    /// A meaningful English word for it, unique in the store. Default made from the name.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"cardigans")]
    pub term_code: Option<String>,
}

/// What `create_category` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCategoryPlan {
    pub category_id: String,
    pub name: String,
    pub term_code: String,
    pub term_set: String,
    pub parent_id: Option<String>,
    pub position: i32,
}

/// Create a category node.
pub struct CreateCategory;

impl Operation for CreateCategory {
    const META: OpMeta = OpMeta {
        address: "merchandising.category.create",
        tool: "create_category",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/merchandising/categories",
        summary: "Create a category in the standard tree (where products are classified) or the display tree (the shopper menu).",
        consumes: &[Slot::one("category_id")],
        produces: &[Slot::one("category_id")],
        checks: &["category.max_depth", "category.term_code_unique"],
        errors: &[PARENT_NOT_FOUND, DEPTH_EXCEEDED, DUPLICATE_TERM_CODE],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateCategoryInput;
    type Output = Value;
}

impl WriteOp<Cx> for CreateCategory {
    type Plan = CreateCategoryPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateCategoryInput,
    ) -> Result<Planned<CreateCategoryPlan>, ToolError> {
        let rules = CategoryRules::from_policy(&cx.policy)?;
        let name = input.name.trim().to_owned();
        if name.is_empty() {
            return Err(ToolError::invalid_input("A category needs a name.")
                .detail(json!({"field": "name", "reason": "missing"})));
        }
        let code = match &input.term_code {
            Some(c) => slug(c),
            None => slug(&name),
        };
        if code.is_empty() || !code.starts_with(|c: char| c.is_ascii_lowercase()) {
            return Err(
                ToolError::invalid_input("`term_code` must start with a letter.")
                    .detail(json!({"field": "term_code", "reason": "format"})),
            );
        }
        let set = input.term_set.as_str().to_owned();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mut depth = 1;
        if let Some(p) = &input.parent_id {
            check_id("parent_id", "cat", p)?;
            let parent = repo::category(&mut tx, p).await?.ok_or_else(|| {
                ToolError::new(
                    PARENT_NOT_FOUND,
                    format!("There is no category {p} to put it under."),
                )
                .detail(json!({"entity": "category", "id": p}))
                .next("list_categories")
            })?;
            if parent.term_set != set {
                return Err(ToolError::new(
                    CROSS_SET_VIOLATION,
                    format!(
                        "{p} is a {} category; a {set} category goes under a {set} one.",
                        parent.term_set
                    ),
                )
                .detail(json!({"field": "parent_id", "term_set": parent.term_set}))
                .next("list_categories"));
            }
            depth = parent.ancestors.len() + 2;
        }
        let taken: Option<String> = sqlx::query_scalar(
            "SELECT id FROM merchandising.defined_term WHERE tenant_id = $1 AND (term_code = $2 OR id = $3)",
        )
        .bind(cx.tenant.as_str())
        .bind(&code)
        .bind(format!("cat_{code}"))
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let last: Option<i32> = sqlx::query_scalar(
            "SELECT max(position) FROM merchandising.defined_term
             WHERE tenant_id = $1 AND in_defined_term_set = $2 AND broader_id IS NOT DISTINCT FROM $3",
        )
        .bind(cx.tenant.as_str())
        .bind(&set)
        .bind(input.parent_id.as_deref())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        if let Some(existing) = taken {
            return Err(ToolError::new(
                DUPLICATE_TERM_CODE,
                format!("The term code {code} is taken by {existing}."),
            )
            .detail(json!({"entity": "category", "field": "term_code", "existing_id": existing}))
            .next("create_category"));
        }
        if depth > rules.max_depth {
            return Err(ToolError::new(
                DEPTH_EXCEEDED,
                format!("It would be {depth} levels deep; the store allows {}.", rules.max_depth),
            )
            .detail(json!({"limit": rules.max_depth, "actual": depth, "policy_key": "category.max_depth"}))
            .next("update_category"));
        }
        let plan = CreateCategoryPlan {
            category_id: format!("cat_{code}"),
            name,
            term_code: code,
            term_set: set,
            parent_id: input.parent_id,
            position: input.display_order.unwrap_or(last.unwrap_or(0) + 1).max(1),
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "category".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"category_id": plan.category_id, "name": plan.name, "term_code": plan.term_code,
                           "term_set": plan.term_set, "parent_id": plan.parent_id,
                           "display_order": plan.position}),
                note: None,
            }],
            checks: vec![
                Check {
                    rule: "category.max_depth".into(),
                    passed: true,
                    detail: Some(format!(
                        "Level {depth}; the store allows {}",
                        rules.max_depth
                    )),
                    fix_tool: None,
                },
                Check {
                    rule: "category.term_code_unique".into(),
                    passed: true,
                    detail: Some(format!(
                        "{} is not used by another category",
                        plan.term_code
                    )),
                    fix_tool: None,
                },
            ],
            basis: Vec::new(),
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan,
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &CreateCategoryPlan,
    ) -> Result<Committed, ToolError> {
        sqlx::query(
            "INSERT INTO merchandising.defined_term (tenant_id, id, name, term_code,
               in_defined_term_set, broader_id, position)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (tenant_id, id) DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.category_id)
        .bind(&plan.name)
        .bind(&plan.term_code)
        .bind(&plan.term_set)
        .bind(plan.parent_id.as_deref())
        .bind(plan.position)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
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
        plan: &CreateCategoryPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let t = term(&mut tx, &plan.category_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = t.is_some_and(|t| t.name == plan.name && t.broader_id == plan.parent_id);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} does not read back as created",
                    plan.category_id
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &CreateCategoryPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        let (kids, maps, products) = attached(tx, &plan.category_id).await?;
        if kids + maps + products > 0 {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} is in use now (categories, mappings or products); it stays.",
                    plan.category_id
                ),
            )
            .detail(json!({"entity": "category", "id": plan.category_id}))
            .next("get_category"));
        }
        sqlx::query("DELETE FROM merchandising.defined_term WHERE tenant_id = $1 AND id = $2")
            .bind(tx.tenant.as_str())
            .bind(&plan.category_id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        Ok(())
    }
}
