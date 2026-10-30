//! `create_category`, `update_category`, `map_category`, `unmap_category`
//! (docs/api/merchandising.md#category): the standard tree products are classified in, the
//! display tree shoppers browse, and the mappings that show a standard category's products in
//! display categories. Rules come from `merchandising.yml category`.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorClass, ErrorCode, Group,
    Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::TermSet;
use acom_policy::PolicySet;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::category::CATEGORY_NOT_FOUND;
use crate::check_id;
use crate::repo::{self, CategoryRow, db};

pub const PARENT_NOT_FOUND: ErrorCode = ErrorCode::new("parent_not_found", ErrorClass::NotFound);
pub const DEPTH_EXCEEDED: ErrorCode = ErrorCode::new("depth_exceeded", ErrorClass::LimitExceeded);
pub const DUPLICATE_TERM_CODE: ErrorCode =
    ErrorCode::new("duplicate_term_code", ErrorClass::AlreadyExists);
pub const CYCLE_DETECTED: ErrorCode = ErrorCode::new("cycle_detected", ErrorClass::Domain);
pub const HAS_CHILDREN: ErrorCode = ErrorCode::new("has_children", ErrorClass::InvalidState);
pub const HAS_MAPPED_PRODUCTS: ErrorCode =
    ErrorCode::new("has_mapped_products", ErrorClass::InvalidState);
pub const CROSS_SET_VIOLATION: ErrorCode =
    ErrorCode::new("cross_set_violation", ErrorClass::Domain);
pub const PRIMARY_CONFLICT: ErrorCode = ErrorCode::new("primary_conflict", ErrorClass::Domain);
pub const DISPLAY_CATEGORY_INACTIVE: ErrorCode =
    ErrorCode::new("display_category_inactive", ErrorClass::InvalidState);
pub const MAPPING_NOT_FOUND: ErrorCode = ErrorCode::new("mapping_not_found", ErrorClass::NotFound);
pub const PRIMARY_NEEDS_REASSIGN: ErrorCode =
    ErrorCode::new("primary_needs_reassign", ErrorClass::Domain);

/// `merchandising.yml category`, the parts these writes check.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryRules {
    pub max_depth: usize,
    pub primary_per_standard: u64,
    pub delete_only_if: Vec<String>,
}

impl CategoryRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let missing = |k: &str| {
            ToolError::new(
                ErrorCode::INTERNAL,
                format!("The store policy `{k}` is missing or malformed."),
            )
        };
        Ok(Self {
            max_depth: p
                .lookup("merchandising.category.max_depth")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| missing("merchandising.category.max_depth"))?,
            primary_per_standard: p
                .lookup("merchandising.category.primary_display_per_standard")
                .and_then(Value::as_u64)
                .unwrap_or(1),
            delete_only_if: p
                .lookup("merchandising.category.delete_only_if")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

fn not_found(id: &str) -> ToolError {
    ToolError::new(CATEGORY_NOT_FOUND, format!("There is no category {id}."))
        .detail(json!({"entity": "category", "id": id}))
        .next("list_categories")
}

/// The category row as a write sees it, with its version.
#[derive(Debug, Clone, sqlx::FromRow)]
struct Term {
    id: String,
    name: String,
    in_defined_term_set: String,
    broader_id: Option<String>,
    position: i32,
    is_active: bool,
    version: i32,
}

async fn term(tx: &mut TxCx, id: &str) -> Result<Option<Term>, ToolError> {
    sqlx::query_as::<_, Term>(
        "SELECT id, name, in_defined_term_set, broader_id, position, is_active, version
         FROM merchandising.defined_term WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// How many levels a category's subtree has (1 for a leaf).
async fn subtree_height(tx: &mut TxCx, id: &str) -> Result<usize, ToolError> {
    let h: i32 = sqlx::query_scalar(
        "WITH RECURSIVE down AS (
           SELECT id, 1 AS level FROM merchandising.defined_term WHERE tenant_id = $1 AND id = $2
           UNION ALL
           SELECT c.id, d.level + 1 FROM merchandising.defined_term c JOIN down d ON c.broader_id = d.id
           WHERE c.tenant_id = $1 AND d.level < 16)
         SELECT max(level) FROM down",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(usize::try_from(h).unwrap_or(1))
}

/// A category's mappings, either side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Mapping {
    pub id: String,
    pub standard_category_id: String,
    pub display_category_id: String,
    pub is_primary: bool,
}

pub(crate) async fn mappings_of(
    tx: &mut TxCx,
    category_id: &str,
) -> Result<Vec<Mapping>, ToolError> {
    sqlx::query_as::<_, Mapping>(
        "SELECT id, standard_category_id, display_category_id, is_primary
         FROM merchandising.category_mapping
         WHERE tenant_id = $1 AND (standard_category_id = $2 OR display_category_id = $2)
         ORDER BY is_primary DESC, id",
    )
    .bind(tx.tenant.as_str())
    .bind(category_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

async fn mapping(tx: &mut TxCx, id: &str) -> Result<Option<Mapping>, ToolError> {
    sqlx::query_as::<_, Mapping>(
        "SELECT id, standard_category_id, display_category_id, is_primary
         FROM merchandising.category_mapping WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

async fn insert_mapping(tx: &mut TxCx, m: &Mapping) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO merchandising.category_mapping (tenant_id, id, standard_category_id,
           display_category_id, is_primary)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&m.id)
    .bind(&m.standard_category_id)
    .bind(&m.display_category_id)
    .bind(m.is_primary)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

async fn delete_mapping(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query("DELETE FROM merchandising.category_mapping WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}

async fn children(tx: &mut TxCx, id: &str) -> Result<i64, ToolError> {
    sqlx::query_scalar(
        "SELECT count(*) FROM merchandising.defined_term WHERE tenant_id = $1 AND broader_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// What still hangs on a category: `(children, mappings, products)`.
async fn attached(tx: &mut TxCx, id: &str) -> Result<(i64, i64, i64), ToolError> {
    let kids = children(tx, id).await?;
    let maps = mappings_of(tx, id).await?.len() as i64;
    let products: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM merchandising.product_group_category WHERE tenant_id = $1 AND category_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((kids, maps, products))
}

/// `category.delete_only_if`: whether a category can be switched off now.
fn delete_check(rules: &CategoryRules, (kids, maps, products): (i64, i64, i64)) -> Check {
    let mut left = Vec::new();
    for r in &rules.delete_only_if {
        match r.as_str() {
            "no_children" if kids > 0 => left.push(format!(
                "{kids} categor{} under it",
                if kids == 1 { "y" } else { "ies" }
            )),
            "no_mappings" if maps > 0 => left.push(format!(
                "{maps} mapping{}",
                if maps == 1 { "" } else { "s" }
            )),
            "no_products" if products > 0 => left.push(format!(
                "{products} product{}",
                if products == 1 { "" } else { "s" }
            )),
            _ => {}
        }
    }
    Check {
        rule: "category.delete_only_if".into(),
        passed: left.is_empty(),
        detail: Some(if left.is_empty() {
            "Nothing is under it, mapped to it or placed in it".into()
        } else {
            format!("Still has {}", left.join(", "))
        }),
        fix_tool: (!left.is_empty()).then(|| {
            if maps > 0 {
                "unmap_category".into()
            } else {
                "list_categories".into()
            }
        }),
    }
}

// ---------------------------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapCategoryInput {
    /// The standard category whose products are shown.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub standard_category_id: String,
    /// Display categories that show them.
    #[schemars(length(min = 1, max = 20), example = ["cat_new_in", "cat_travel"])]
    pub display_category_ids: Vec<String>,
    /// Make the first display category the standard category's primary one (the path product
    /// pages show). A standard category has one.
    #[schemars(example = true)]
    pub is_primary: Option<bool>,
}

/// What `map_category` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapPlan {
    pub created: Vec<Mapping>,
}

/// Show a standard category's products in display categories.
pub struct MapCategory;

impl Operation for MapCategory {
    const META: OpMeta = OpMeta {
        address: "merchandising.category.map",
        tool: "map_category",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/merchandising/category-mappings",
        summary: "Show a standard category's products in one or more display categories of the shopper menu. Then call invalidate_display_cache.",
        consumes: &[Slot::one("category_id"), Slot::many("category_id")],
        produces: &[Slot::many("category_mapping_id")],
        checks: &["category.mapping", "category.primary_display_per_standard"],
        errors: &[
            CROSS_SET_VIOLATION,
            PRIMARY_CONFLICT,
            DISPLAY_CATEGORY_INACTIVE,
            CATEGORY_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = MapCategoryInput;
    type Output = Value;
}

fn mapping_id(standard: &str, display: &str) -> String {
    format!(
        "map_{}_{}",
        standard.trim_start_matches("cat_"),
        display.trim_start_matches("cat_")
    )
}

impl WriteOp<Cx> for MapCategory {
    type Plan = MapPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: MapCategoryInput,
    ) -> Result<Planned<MapPlan>, ToolError> {
        let rules = CategoryRules::from_policy(&cx.policy)?;
        check_id("standard_category_id", "cat", &input.standard_category_id)?;
        if input.display_category_ids.is_empty() {
            return Err(
                ToolError::invalid_input("Name at least one display category.")
                    .detail(json!({"field": "display_category_ids", "reason": "missing"})),
            );
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let std = repo::category(&mut tx, &input.standard_category_id)
            .await?
            .ok_or_else(|| not_found(&input.standard_category_id))?;
        if std.term_set != "standard" {
            return Err(ToolError::new(
                CROSS_SET_VIOLATION,
                format!("{} is a display category; mappings go from a standard category to display ones.", std.id),
            )
            .detail(json!({"field": "standard_category_id", "term_set": std.term_set, "rule": "category.mapping"}))
            .next("map_category"));
        }
        let existing = mappings_of(&mut tx, &std.id).await?;
        let mut displays = Vec::new();
        for d in &input.display_category_ids {
            check_id("display_category_ids", "cat", d)?;
            let c = repo::category(&mut tx, d)
                .await?
                .ok_or_else(|| not_found(d))?;
            if c.term_set != "display" {
                return Err(ToolError::new(
                    CROSS_SET_VIOLATION,
                    format!("{d} is a standard category; a standard category maps to display ones."),
                )
                .detail(json!({"field": "display_category_ids", "term_set": c.term_set, "rule": "category.mapping"}))
                .next("map_category"));
            }
            if !c.active {
                return Err(ToolError::new(
                    DISPLAY_CATEGORY_INACTIVE,
                    format!("{d} is switched off; switch it on first."),
                )
                .detail(json!({"entity": "category", "id": d, "from": "inactive"}))
                .next("update_category"));
            }
            displays.push(c);
        }
        let products = repo::product_count(&mut tx, &std.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let want_primary = input.is_primary == Some(true);
        let mut created = Vec::new();
        let mut changes = Vec::new();
        for (i, d) in displays.iter().enumerate() {
            if existing.iter().any(|m| m.display_category_id == d.id) {
                continue;
            }
            let m = Mapping {
                id: mapping_id(&std.id, &d.id),
                standard_category_id: std.id.clone(),
                display_category_id: d.id.clone(),
                is_primary: want_primary && i == 0,
            };
            changes.push(Change {
                op: ChangeOp::Create,
                entity: "category_mapping".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"category_mapping_id": m.id, "standard_category_id": m.standard_category_id,
                           "display_category_id": m.display_category_id, "is_primary": m.is_primary,
                           "products_shown": products}),
                note: Some(format!("{products} products of {} show under {}", std.name, d.name)),
            });
            created.push(m);
        }
        if created.is_empty() {
            return Err(ToolError::invalid_input(
                "Those display categories already show this standard category.",
            )
            .detail(
                json!({"field": "display_category_ids", "reason": "missing", "existing": existing}),
            ));
        }
        let primaries = existing.iter().filter(|m| m.is_primary).count() as u64
            + created.iter().filter(|m| m.is_primary).count() as u64;
        let conflict = primaries > rules.primary_per_standard;
        Ok(Planned {
            changes,
            checks: vec![
                Check {
                    rule: "category.mapping".into(),
                    passed: true,
                    detail: Some(format!(
                        "{} (standard) to display categories only",
                        std.name
                    )),
                    fix_tool: None,
                },
                Check {
                    rule: "category.primary_display_per_standard".into(),
                    passed: !conflict,
                    detail: Some(if conflict {
                        format!(
                            "{} already has a primary display category ({}); unmap it or keep this one not primary",
                            std.name,
                            existing
                                .iter()
                                .find(|m| m.is_primary)
                                .map_or("", |m| m.display_category_id.as_str())
                        )
                    } else {
                        format!(
                            "{} has at most {} primary display category",
                            std.name, rules.primary_per_standard
                        )
                    }),
                    fix_tool: conflict.then(|| "unmap_category".into()),
                },
            ],
            basis: Vec::new(),
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan: MapPlan { created },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &MapPlan) -> Result<Committed, ToolError> {
        for m in &plan.created {
            insert_mapping(tx, m).await?;
        }
        let ids: Vec<&str> = plan.created.iter().map(|m| m.id.as_str()).collect();
        let mut produced = Map::new();
        produced.insert("category_mapping_id".into(), json!(ids));
        Ok(Committed {
            id: ids.first().map(|s| (*s).to_owned()).unwrap_or_default(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &MapPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        for m in &plan.created {
            if mapping(&mut tx, &m.id).await?.as_ref() != Some(m) {
                mismatches.push(format!("{} is not there", m.id));
            }
        }
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, tx: &mut TxCx, plan: &MapPlan, _d: &Committed) -> Result<(), ToolError> {
        for m in &plan.created {
            delete_mapping(tx, &m.id).await?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnmapCategoryInput {
    /// The mapping to remove (`get_category` lists them).
    #[schemars(regex(pattern = r"^map_[a-z0-9_]+$"), example = &"map_summer_outerwear_travel")]
    pub category_mapping_id: String,
}

/// What `unmap_category` runs: the mapping as it was, to put back on undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmapPlan {
    pub mapping: Mapping,
}

/// Stop showing a standard category's products in a display category.
pub struct UnmapCategory;

impl Operation for UnmapCategory {
    const META: OpMeta = OpMeta {
        address: "merchandising.category.unmap",
        tool: "unmap_category",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Delete,
        path: "/v1/merchandising/category-mappings/{category_mapping_id}",
        summary: "Remove a standard-to-display category mapping. A primary mapping goes only when it is the last one; otherwise make another primary first. Then call invalidate_display_cache.",
        consumes: &[Slot::one("category_mapping_id")],
        produces: &[],
        checks: &["category.primary_display_per_standard"],
        errors: &[MAPPING_NOT_FOUND, PRIMARY_NEEDS_REASSIGN],
        audience: Audience::Operator,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = UnmapCategoryInput;
    type Output = Value;
}

impl WriteOp<Cx> for UnmapCategory {
    type Plan = UnmapPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UnmapCategoryInput,
    ) -> Result<Planned<UnmapPlan>, ToolError> {
        check_id("category_mapping_id", "map", &input.category_mapping_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let m = mapping(&mut tx, &input.category_mapping_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    MAPPING_NOT_FOUND,
                    format!("There is no mapping {}.", input.category_mapping_id),
                )
                .detail(json!({"entity": "category_mapping", "id": input.category_mapping_id}))
                .next("get_category")
            })?;
        let others = mappings_of(&mut tx, &m.standard_category_id)
            .await?
            .into_iter()
            .filter(|o| o.id != m.id && o.standard_category_id == m.standard_category_id)
            .count();
        let products = repo::product_count(&mut tx, &m.standard_category_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let blocked = m.is_primary && others > 0;
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Delete,
                entity: "category_mapping".into(),
                id: m.id.clone(),
                from: json!({"standard_category_id": m.standard_category_id,
                             "display_category_id": m.display_category_id, "is_primary": m.is_primary}),
                to: Value::Null,
                note: Some(format!(
                    "{products} products of {} stop showing under {}",
                    m.standard_category_id, m.display_category_id
                )),
            }],
            checks: vec![Check {
                rule: "category.primary_display_per_standard".into(),
                passed: !blocked,
                detail: Some(if blocked {
                    format!(
                        "It is the primary mapping and {others} more remain; make one of them primary first"
                    )
                } else if m.is_primary {
                    "The last mapping of this standard category; it will have no primary path"
                        .into()
                } else {
                    "Not the primary mapping".into()
                }),
                fix_tool: blocked.then(|| "map_category".into()),
            }],
            basis: Vec::new(),
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan: UnmapPlan { mapping: m },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &UnmapPlan) -> Result<Committed, ToolError> {
        delete_mapping(tx, &plan.mapping.id).await?;
        let mut produced = Map::new();
        produced.insert("category_mapping_id".into(), json!(plan.mapping.id));
        Ok(Committed {
            id: plan.mapping.id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &UnmapPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let still = mapping(&mut tx, &plan.mapping.id).await?.is_some();
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: !still,
            mismatches: if still {
                vec![format!("{} is still there", plan.mapping.id)]
            } else {
                Vec::new()
            },
        })
    }

    async fn undo(&self, tx: &mut TxCx, plan: &UnmapPlan, _d: &Committed) -> Result<(), ToolError> {
        insert_mapping(tx, &plan.mapping).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::policies;

    #[test]
    fn rules_come_from_the_policy() {
        let r = CategoryRules::from_policy(&policies(false)).unwrap();
        assert_eq!(r.max_depth, 4);
        assert_eq!(r.primary_per_standard, 1);
        let c = delete_check(&r, (0, 2, 5));
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("Still has 2 mappings, 5 products")
        );
        assert_eq!(c.fix_tool.as_deref(), Some("unmap_category"));
        assert!(delete_check(&r, (0, 0, 0)).passed);
    }

    #[test]
    fn ids_and_inputs() {
        assert_eq!(
            mapping_id("cat_summer_outerwear", "cat_travel"),
            "map_summer_outerwear_travel"
        );
        assert_eq!(slug("Light Outerwear!"), "light_outerwear");
        for schema in [
            acom_contract::schema::closed_schema_for::<CreateCategoryInput>(),
            acom_contract::schema::closed_schema_for::<UpdateCategoryInput>(),
            acom_contract::schema::closed_schema_for::<MapCategoryInput>(),
            acom_contract::schema::closed_schema_for::<UnmapCategoryInput>(),
        ] {
            assert!(
                acom_contract::schema::violations(&schema).is_empty(),
                "{schema:#}"
            );
        }
    }
}
