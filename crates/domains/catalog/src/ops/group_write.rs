//! `create_product_group`, `update_product_group` (docs/api/catalog.md#product_group): a new
//! product starts as a draft; its content changes without approval, and a structural change
//! (category, option axes) on a product shoppers can see waits for a person.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorClass,
    ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_core::enums::DraftStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::product_not_found;
use crate::neighbors::{self, CategoryTerm};
use crate::rules::ProductRules;
use crate::write::{self, DraftItem, GroupFields, NewGroup};
use crate::{BRAND_NOT_FOUND, PRODUCT_NOT_FOUND, repo, slug};

/// A required field is missing from the input.
pub const MISSING_REQUIRED_FIELD: ErrorCode =
    ErrorCode::new("missing_required_field", ErrorClass::InvalidInput);
/// The category is not a standard category of this store (or is inactive).
pub const INVALID_CATEGORY: ErrorCode =
    ErrorCode::new("invalid_category", ErrorClass::InvalidInput);

/// Option axes a product can vary by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptionAxis {
    /// Variants differ in color.
    Color,
    /// Variants differ in size.
    Size,
}

impl OptionAxis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Color => "color",
            Self::Size => "size",
        }
    }
}

/// The standard category with this id, or `invalid_category`.
pub(crate) async fn standard_category(
    tx: &mut TxCx,
    category_id: &str,
) -> Result<CategoryTerm, ToolError> {
    let terms = neighbors::category_terms(tx).await?;
    match terms.into_iter().find(|t| t.id == category_id) {
        Some(t) if t.term_set == "standard" && t.active => Ok(t),
        Some(t) => Err(ToolError::new(
            INVALID_CATEGORY,
            if t.term_set == "standard" {
                format!("{category_id} is inactive; products go in active standard categories.")
            } else {
                format!("{category_id} is a display category; products go in a standard category, and display categories show them through map_category.")
            },
        )
        .detail(json!({"field": "category_id", "reason": "enum", "term_set": t.term_set, "active": t.active}))
        .next("list_categories")),
        None => Err(ToolError::new(
            INVALID_CATEGORY,
            format!("There is no category {category_id} in this store."),
        )
        .detail(json!({"field": "category_id", "reason": "enum"}))
        .next("list_categories")),
    }
}

/// The product's primary category now.
pub(crate) async fn primary_category(
    tx: &mut TxCx,
    product_id: &str,
) -> Result<Option<String>, ToolError> {
    Ok(neighbors::categories(tx, Some(&[product_id.to_owned()]))
        .await?
        .into_iter()
        .filter(|c| c.direct)
        .max_by_key(|c| c.is_primary)
        .map(|c| c.category_id))
}

async fn known_brand(tx: &mut TxCx, brand_id: &str) -> Result<(), ToolError> {
    match repo::brand(tx, brand_id).await? {
        Some(_) => Ok(()),
        None => Err(
            ToolError::new(BRAND_NOT_FOUND, format!("There is no brand {brand_id}."))
                .detail(json!({"entity": "brand", "id": brand_id}))
                .next("list_brands"),
        ),
    }
}

/// An id for a new product from its name: `prd_wool_blend_overcoat`, then `_2`, `_3` while
/// taken.
pub(crate) async fn free_product_id(tx: &mut TxCx, base: &str) -> Result<String, ToolError> {
    let base = format!("prd_{}", slug(base));
    let mut candidates = vec![base.clone()];
    candidates.extend((2..50).map(|n| format!("{base}_{n}")));
    let taken = write::existing_groups(tx, &candidates).await?;
    candidates
        .into_iter()
        .find(|c| !taken.contains_key(c))
        .ok_or_else(|| ToolError::new(ErrorCode::INTERNAL, "no free product id"))
}

fn max_axes_check(p: &acom_policy::PolicySet, axes: usize) -> Option<Check> {
    let max = p
        .lookup("catalog.variant.max_option_axes")
        .and_then(Value::as_u64)?;
    let ok = (axes as u64) <= max;
    Some(Check {
        rule: "variant.max_option_axes".into(),
        passed: ok,
        detail: Some(format!("{axes} option axes; the store allows {max}")),
        fix_tool: (!ok).then(|| "update_product_group".into()),
    })
}

/// The first image URL of an `image` list.
fn first_image(images: Option<&Vec<String>>) -> Option<String> {
    images
        .and_then(|l| l.iter().map(|s| s.trim()).find(|s| !s.is_empty()))
        .map(str::to_owned)
}

fn trimmed(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProductGroupInput {
    /// The product's name as shoppers will see it.
    #[schemars(length(min = 1, max = 200), example = &"Packable windbreaker")]
    pub name: String,
    /// The description in the store's tone (`content.copy`). Can come later.
    #[schemars(length(max = 4000), example = &"A wind layer that packs into its own pocket. Light enough to forget until the weather turns. Machine wash cold.")]
    pub description: Option<String>,
    /// The brand (`list_brands`).
    #[schemars(regex(pattern = r"^brn_[a-z0-9_]+$"), example = &"brn_northwind")]
    pub brand_id: Option<String>,
    /// The standard category (`list_categories` with term_set standard).
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub category_id: String,
    /// Image URLs; the first is the main image.
    #[schemars(example = ["https://cdn.northwind.example/p/windbreaker-slate.jpg"])]
    pub image: Option<Vec<String>>,
    /// Option axes of its variants. color: variants differ in color; size: in size.
    #[schemars(example = ["color", "size"])]
    pub varies_by: Option<Vec<OptionAxis>>,
    /// Main material shown to shoppers.
    #[schemars(length(max = 200), example = &"Recycled nylon")]
    pub material: Option<String>,
}

/// What `create_product_group` runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateGroupPlan {
    pub product_id: String,
    pub name: String,
    pub description: Option<String>,
    pub brand_id: Option<String>,
    pub category_id: String,
    pub image: Option<String>,
    pub varies_by: Vec<String>,
    pub material: Option<String>,
}

/// Register a new product as a draft.
#[derive(Debug)]
pub struct CreateProductGroup;

impl Operation for CreateProductGroup {
    const META: OpMeta = OpMeta {
        address: "catalog.product_group.create",
        tool: "create_product_group",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/product-groups",
        summary: "Register a new product as a draft in a standard category. Shoppers do not see it; add variants with set_variants and publish with approve_product.",
        consumes: &[Slot::one("brand_id"), Slot::one("category_id")],
        produces: &[Slot::one("product_id")],
        checks: &["product.required_on_create", "variant.max_option_axes"],
        errors: &[MISSING_REQUIRED_FIELD, INVALID_CATEGORY, BRAND_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateProductGroupInput;
    type Output = Value;
}

impl WriteOp<Cx> for CreateProductGroup {
    type Plan = CreateGroupPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateProductGroupInput,
    ) -> Result<Planned<CreateGroupPlan>, ToolError> {
        let rules = ProductRules::from_policy(&cx.policy)?;
        let Some(name) = trimmed(Some(&input.name)) else {
            return Err(
                ToolError::new(MISSING_REQUIRED_FIELD, "A product needs a name.")
                    .detail(json!({"field": "name", "reason": "missing"}))
                    .next("create_product_group"),
            );
        };
        let mut varies_by: Vec<String> = Vec::new();
        for a in input.varies_by.unwrap_or_default() {
            if !varies_by.iter().any(|x| x == a.as_str()) {
                varies_by.push(a.as_str().to_owned());
            }
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let category = standard_category(&mut tx, &input.category_id).await?;
        if let Some(b) = &input.brand_id {
            known_brand(&mut tx, b).await?;
        }
        let product_id = free_product_id(&mut tx, &name).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let plan = CreateGroupPlan {
            product_id,
            name,
            description: trimmed(input.description.as_deref()),
            brand_id: input.brand_id,
            category_id: category.id.clone(),
            image: first_image(input.image.as_ref()),
            varies_by,
            material: trimmed(input.material.as_deref()),
        };
        let mut checks = vec![Check {
            rule: "product.required_on_create".into(),
            passed: true,
            detail: Some(format!(
                "It has {}; the store is the seller",
                rules
                    .required_on_create
                    .iter()
                    .filter(|r| *r != "seller")
                    .map(|r| match r.as_str() {
                        "category" => format!("the category {}", category.name),
                        "name" => "a name".to_owned(),
                        other => other.replace('_', " "),
                    })
                    .collect::<Vec<_>>()
                    .join(" and ")
            )),
            fix_tool: None,
        }];
        checks.extend(max_axes_check(&cx.policy, plan.varies_by.len()));
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "product".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"product_id": plan.product_id, "name": plan.name,
                           "description": plan.description, "brand_id": plan.brand_id,
                           "category_id": plan.category_id, "image": plan.image,
                           "varies_by": plan.varies_by, "material": plan.material,
                           "approval_status": "draft"}),
                note: Some("A draft: shoppers do not see it until it is approved.".into()),
            }],
            checks,
            basis: Vec::new(),
            approval: crate::policy_approval(
                cx,
                Self::META.tool,
                &json!({"change_type": "content"}),
            ),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &CreateGroupPlan) -> Result<Committed, ToolError> {
        if write::existing_groups(tx, std::slice::from_ref(&plan.product_id))
            .await?
            .is_empty()
        {
            write::insert_group(
                tx,
                &NewGroup {
                    id: &plan.product_id,
                    name: &plan.name,
                    description: plan.description.as_deref(),
                    brand_id: plan.brand_id.as_deref(),
                    varies_by: &plan.varies_by,
                    material: plan.material.as_deref(),
                    image: plan.image.as_deref(),
                },
            )
            .await?;
            neighbors::place_product(tx, &plan.product_id, &plan.category_id).await?;
        }
        let mut produced = Map::new();
        produced.insert("product_id".into(), json!(plan.product_id));
        Ok(Committed {
            id: plan.product_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CreateGroupPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let g = repo::group(&mut tx, &plan.product_id).await?;
        let category = primary_category(&mut tx, &plan.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match g {
            None => mismatches.push(format!("{} was not created", plan.product_id)),
            Some(g) if g.name != plan.name => {
                mismatches.push(format!("{} is named {}", g.id, g.name))
            }
            Some(_) => {}
        }
        if category.as_deref() != Some(plan.category_id.as_str()) {
            mismatches.push(format!(
                "{} is not in {}",
                plan.product_id, plan.category_id
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
        plan: &CreateGroupPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        let variants = repo::variants(tx, std::slice::from_ref(&plan.product_id)).await?;
        if !variants.is_empty() {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} has variants now; take those back first.",
                    plan.product_id
                ),
            )
            .detail(json!({"entity": "product", "id": plan.product_id}))
            .next("get_product_group"));
        }
        if !write::delete_draft(tx, &plan.product_id).await? {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("{} is no longer a draft; it stays.", plan.product_id),
            )
            .detail(json!({"entity": "product", "id": plan.product_id}))
            .next("get_change_history"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProductGroupInput {
    /// The product to change.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_fw26_ot_022_olv")]
    pub product_id: String,
    /// New name.
    #[schemars(length(min = 1, max = 200), example = &"Quilted liner jacket, olive")]
    pub name: Option<String>,
    /// New description in the store's tone (`content.copy`). Empty text clears it.
    #[schemars(length(max = 4000), example = &"A light quilted layer for under a shell or over a knit. Warm without bulk. Machine wash cold, tumble dry low.")]
    pub description: Option<String>,
    /// New standard category (`list_categories`). A structural change.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_outerwear")]
    pub category_id: Option<String>,
    /// New brand (`list_brands`).
    #[schemars(regex(pattern = r"^brn_[a-z0-9_]+$"), example = &"brn_northwind")]
    pub brand_id: Option<String>,
    /// Image URLs; the first becomes the main image. Fills an open `image` on a draft.
    #[schemars(example = ["https://cdn.northwind.example/fw26/fw26-ot-022-olive.jpg"])]
    pub image: Option<Vec<String>>,
    /// New option axes. A structural change. color: variants differ in color; size: in size.
    #[schemars(example = ["size"])]
    pub varies_by: Option<Vec<OptionAxis>>,
    /// Main material, for example a fabric mix. Fills an open `material` on a draft.
    #[schemars(length(min = 1, max = 200), example = &"Shell 100% nylon; fill 100% recycled polyester")]
    pub material: Option<String>,
}

/// What `update_product_group` runs, with what it replaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateGroupPlan {
    pub product_id: String,
    pub to: GroupFields,
    pub from: GroupFields,
    /// `(from, to)` when the category changes.
    pub category: Option<(Option<String>, String)>,
    /// The draft's sheet row before and after, when filling a field changes what is open.
    pub item: Option<(DraftItem, DraftItem)>,
}

/// Change a product's content or structure.
#[derive(Debug)]
pub struct UpdateProductGroup;

impl Operation for UpdateProductGroup {
    const META: OpMeta = OpMeta {
        address: "catalog.product_group.update",
        tool: "update_product_group",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/catalog/product-groups/{product_id}",
        summary: "Change a product's name, description, images, material, brand, category or option axes. Content changes need no approval; a category or axis change on a product shoppers can see does. Filling a draft's open field clears it.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("product_id")],
        checks: &["product.approval", "import.required_fields"],
        errors: &[
            ErrorCode::INVALID_STATE,
            INVALID_CATEGORY,
            BRAND_NOT_FOUND,
            PRODUCT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateProductGroupInput;
    type Output = Value;
}

/// A draft's sheet row after `filled` fields were given: they are no longer open.
pub(crate) fn fill_fields(item: &DraftItem, filled: &[&str]) -> DraftItem {
    let mut next = item.clone();
    next.open_fields.retain(|f| !filled.contains(&f.as_str()));
    if next.open_fields.is_empty() {
        next.draft_status = DraftStatus::Ready.as_str().into();
        next.draft_note = None;
    } else if next.open_fields != item.open_fields {
        next.draft_note = Some(format!("Still open: {}", next.open_fields.join(", ")));
    }
    next
}

impl WriteOp<Cx> for UpdateProductGroup {
    type Plan = UpdateGroupPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateProductGroupInput,
    ) -> Result<Planned<UpdateGroupPlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let g = repo::group(&mut tx, &input.product_id)
            .await?
            .ok_or_else(|| {
                product_not_found("product", &input.product_id, "search_product_groups")
            })?;
        if g.approval_status == "discontinued" {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("{} is discontinued and is no longer edited.", g.id),
            )
            .detail(json!({"entity": "product", "from": "discontinued"}))
            .next("get_product_group"));
        }
        let mut to = GroupFields::default();
        let mut from = GroupFields::default();
        let mut changed_from = Map::new();
        let mut changed_to = Map::new();
        if let Some(n) = trimmed(input.name.as_deref()).filter(|n| *n != g.name) {
            changed_from.insert("name".into(), json!(g.name));
            changed_to.insert("name".into(), json!(n));
            from.name = Some(g.name.clone());
            to.name = Some(n);
        }
        if let Some(d) = input.description.as_deref() {
            let d = trimmed(Some(d));
            if d != g.description {
                changed_from.insert("description".into(), json!(g.description));
                changed_to.insert("description".into(), json!(d));
                from.description = Some(g.description.clone());
                to.description = Some(d);
            }
        }
        if let Some(m) =
            trimmed(input.material.as_deref()).filter(|m| Some(m) != g.material.as_ref())
        {
            changed_from.insert("material".into(), json!(g.material));
            changed_to.insert("material".into(), json!(m));
            from.material = Some(g.material.clone());
            to.material = Some(Some(m));
        }
        let image = first_image(input.image.as_ref());
        if let Some(i) = image.clone().filter(|i| Some(i) != g.image.as_ref()) {
            changed_from.insert("image".into(), json!(g.image));
            changed_to.insert("image".into(), json!(i));
            from.image = Some(g.image.clone());
            to.image = Some(Some(i));
        }
        if let Some(b) = input
            .brand_id
            .as_deref()
            .filter(|b| Some(*b) != g.brand_id.as_deref())
        {
            known_brand(&mut tx, b).await?;
            changed_from.insert("brand_id".into(), json!(g.brand_id));
            changed_to.insert("brand_id".into(), json!(b));
            from.brand_id = Some(g.brand_id.clone());
            to.brand_id = Some(Some(b.to_owned()));
        }
        let mut structural = false;
        if let Some(axes) = &input.varies_by {
            let mut list: Vec<String> = Vec::new();
            for a in axes {
                if !list.iter().any(|x| x == a.as_str()) {
                    list.push(a.as_str().to_owned());
                }
            }
            if list != g.varies_by {
                structural = true;
                changed_from.insert("varies_by".into(), json!(g.varies_by));
                changed_to.insert("varies_by".into(), json!(list));
                from.varies_by = Some(g.varies_by.clone());
                to.varies_by = Some(list);
            }
        }
        let mut category = None;
        if let Some(c) = &input.category_id {
            let now = primary_category(&mut tx, &g.id).await?;
            if now.as_deref() != Some(c.as_str()) {
                let term = standard_category(&mut tx, c).await?;
                structural = true;
                changed_from.insert("category_id".into(), json!(now));
                changed_to.insert("category_id".into(), json!(term.id));
                category = Some((now, term.id));
            }
        }
        let item_now = write::items_of(&mut tx, std::slice::from_ref(&g.id))
            .await?
            .pop();
        tx.commit().await.map_err(ToolError::from)?;

        let mut filled: Vec<&str> = Vec::new();
        if to.material.is_some() {
            filled.push("material");
        }
        if image.is_some() {
            filled.push("image");
        }
        if to.brand_id.is_some() {
            filled.push("brand");
        }
        let item = item_now.and_then(|before| {
            let after = fill_fields(&before, &filled);
            (after != before).then_some((before, after))
        });
        let mut checks = Vec::new();
        if let Some((before, after)) = &item {
            changed_from.insert("acom:draft_status".into(), json!(before.draft_status));
            changed_to.insert("acom:draft_status".into(), json!(after.draft_status));
            changed_from.insert("acom:open_fields".into(), json!(before.open_fields));
            changed_to.insert("acom:open_fields".into(), json!(after.open_fields));
            checks.push(Check {
                rule: "import.required_fields".into(),
                passed: true,
                detail: Some(if after.open_fields.is_empty() {
                    "Fills every field the supplier sheet left open".into()
                } else {
                    format!("Still open after this: {}", after.open_fields.join(", "))
                }),
                fix_tool: (!after.open_fields.is_empty()).then(|| "update_product_group".into()),
            });
        }
        if changed_to.is_empty() {
            return Err(ToolError::invalid_input(format!(
                "Nothing to change: {} already has these values.",
                g.id
            ))
            .detail(json!({"field": "product_id", "reason": "missing"})));
        }
        let customer_facing = matches!(g.approval_status.as_str(), "active" | "pending_review");
        let change_type = if structural && customer_facing {
            "structural"
        } else {
            "content"
        };
        checks.insert(
            0,
            Check {
                rule: "product.approval".into(),
                passed: true,
                detail: Some(match (structural, customer_facing) {
                    (true, true) => {
                        "Changes the structure of a product shoppers can see; a person approves it"
                            .into()
                    }
                    (true, false) => format!(
                        "A {} product: its structure changes without review",
                        g.approval_status.replace('_', " ")
                    ),
                    _ => "A content change; no approval needed".into(),
                }),
                fix_tool: None,
            },
        );
        let approval = cx.policy.approval_for(
            Self::META.tool,
            &json!({"change_type": change_type, "new_product": false}),
        );
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "product".into(),
                id: g.id.clone(),
                from: Value::Object(changed_from),
                to: Value::Object(changed_to),
                note: None,
            }],
            checks,
            basis: vec![Basis {
                entity: "catalog.product_group".into(),
                id: g.id.clone(),
                version: i64::from(g.version),
            }],
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: if category.is_some() {
                        "Changes the category".into()
                    } else {
                        approval
                            .reason
                            .unwrap_or_else(|| "Changes the option axes".into())
                    },
                }
            } else {
                ApprovalNeed::NotNeeded
            },
            plan: UpdateGroupPlan {
                product_id: g.id,
                to,
                from,
                category,
                item,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &UpdateGroupPlan) -> Result<Committed, ToolError> {
        let n = write::update_group(tx, &plan.product_id, &plan.to).await?;
        if n == 0 {
            return Err(product_not_found(
                "product",
                &plan.product_id,
                "search_product_groups",
            ));
        }
        if let Some((_, to)) = &plan.category {
            neighbors::place_product(tx, &plan.product_id, to).await?;
        }
        if let Some((_, after)) = &plan.item {
            write::set_item_state(tx, after).await?;
        }
        let mut produced = Map::new();
        produced.insert("product_id".into(), json!(plan.product_id));
        Ok(Committed {
            id: plan.product_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &UpdateGroupPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let g = repo::group(&mut tx, &plan.product_id).await?;
        let category = primary_category(&mut tx, &plan.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        let Some(g) = g else {
            return Ok(Verification {
                matched: false,
                mismatches: vec![format!("{} is missing", plan.product_id)],
            });
        };
        if let Some(n) = &plan.to.name
            && *n != g.name
        {
            mismatches.push(format!("{} is named {}", g.id, g.name));
        }
        if let Some(d) = &plan.to.description
            && *d != g.description
        {
            mismatches.push(format!("{} has another description", g.id));
        }
        if let Some(m) = &plan.to.material
            && *m != g.material
        {
            mismatches.push(format!("{} has another material", g.id));
        }
        if let Some((_, c)) = &plan.category
            && category.as_deref() != Some(c.as_str())
        {
            mismatches.push(format!("{} is not in {c}", g.id));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &UpdateGroupPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        write::update_group(tx, &plan.product_id, &plan.from).await?;
        if let Some((Some(from), _)) = &plan.category {
            neighbors::place_product(tx, &plan.product_id, from).await?;
        }
        if let Some((before, _)) = &plan.item {
            write::set_item_state(tx, before).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;

    #[test]
    fn filling_the_last_open_field_makes_the_draft_ready() {
        let item = DraftItem {
            product_import_id: "imp_fw26_supplier".into(),
            position: 4,
            product_group_id: "prd_fw26_ot_022_olv".into(),
            supplier_code: "FW26-OT-022".into(),
            supplier_text: None,
            draft_status: "needs_you".into(),
            open_fields: vec!["material".into(), "image".into()],
            draft_note: Some("Fabric mix is missing in the sheet".into()),
        };
        let after = fill_fields(&item, &["material"]);
        assert_eq!(after.open_fields, ["image"]);
        assert_eq!(after.draft_status, "needs_you");
        assert_eq!(after.draft_note.as_deref(), Some("Still open: image"));
        let done = fill_fields(&after, &["image"]);
        assert_eq!(done.draft_status, "ready");
        assert!(done.open_fields.is_empty() && done.draft_note.is_none());
        assert_eq!(fill_fields(&item, &[]), item);
    }

    #[test]
    fn inputs_are_closed_and_axes_are_words() {
        for schema in [
            acom_contract::schema::closed_schema_for::<CreateProductGroupInput>(),
            acom_contract::schema::closed_schema_for::<UpdateProductGroupInput>(),
        ] {
            assert!(
                acom_contract::schema::violations(&schema).is_empty(),
                "{schema:#}"
            );
        }
        let i: CreateProductGroupInput = serde_json::from_value(json!({
            "name": "Packable windbreaker", "category_id": "cat_summer_outerwear",
            "varies_by": ["color", "size"]}))
        .unwrap();
        assert_eq!(i.varies_by.unwrap()[1], OptionAxis::Size);
        assert!(
            serde_json::from_value::<CreateProductGroupInput>(
                json!({"name": "x", "category_id": "cat_x", "varies_by": ["fit"]})
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn a_blank_name_is_refused_before_any_lookup() {
        let e = CreateProductGroup
            .preview(
                &testing::cx(acom_core::enums::ActorKind::Operator, "adm_sj"),
                serde_json::from_value(json!({"name": "  ", "category_id": "cat_outerwear"}))
                    .unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("missing_required_field", Some("create_product_group"))
        );
    }

    #[test]
    fn structural_changes_to_live_products_wait_for_a_person() {
        let p = crate::testing::policies();
        assert!(
            p.approval_for(
                "update_product_group",
                &json!({"change_type": "structural", "new_product": false})
            )
            .needed
        );
        assert!(
            !p.approval_for(
                "update_product_group",
                &json!({"change_type": "content", "new_product": false})
            )
            .needed
        );
    }
}
