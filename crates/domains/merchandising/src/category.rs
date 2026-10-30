//! `list_categories`, `get_category` (docs/api/merchandising.md#category).

use acom_contract::{
    ActionType, Audience, ErrorClass, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp,
    Slot, ToolError,
};
use acom_core::enums::TermSet;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::repo::{self, CategoryRow};
use crate::{check_id, pick};

pub const CATEGORY_NOT_FOUND: ErrorCode =
    ErrorCode::new("category_not_found", ErrorClass::NotFound);

fn category_json(c: &CategoryRow) -> Value {
    json!({
        "id": c.id,
        "name": c.name,
        "term_code": c.term_code,
        "term_set": c.term_set,
        "parent_id": c.parent_id,
        "display_order": c.display_order,
        "active": c.active,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListCategoriesInput {
    /// Which tree. standard: the internal classification products belong to; display: the
    /// menu shoppers see. Both when left out.
    #[schemars(example = &"standard")]
    pub term_set: Option<TermSet>,
    /// Only categories under this one.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_outerwear")]
    pub parent_id: Option<String>,
    /// How many levels down: from the top, or below `parent_id` when given (1 = direct
    /// children). Every level when left out.
    #[schemars(range(min = 1, max = 10), example = 1)]
    pub depth: Option<i32>,
    /// Only these fields of each category (`id` always comes back).
    #[schemars(example = ["name", "parent_id"])]
    pub fields: Option<Vec<String>>,
    /// Page size, 1–100. Default 50.
    #[schemars(range(min = 1, max = 100), example = 50)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o50")]
    pub cursor: Option<String>,
}

pub struct ListCategories;

impl Operation for ListCategories {
    const META: OpMeta = OpMeta {
        address: "merchandising.category.list",
        tool: "list_categories",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/merchandising/categories",
        summary: "The category tree (standard classification and the shopper menu), top down. A category under an inactive one reads as inactive.",
        consumes: &[],
        produces: &[Slot::many("category_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListCategoriesInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListCategories {
    async fn read(&self, cx: &Cx, input: ListCategoriesInput) -> Result<Value, ToolError> {
        let page = Page::new(Some(input.limit.unwrap_or(50)), input.cursor.as_deref())?;
        if let Some(p) = &input.parent_id {
            check_id("parent_id", "cat", p)?;
        }
        if input.depth.is_some_and(|d| d < 1) {
            return Err(ToolError::invalid_input("`depth` is 1 or more.")
                .detail(json!({"field": "depth", "reason": "range"})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (total, rows) = repo::categories(
            &mut tx,
            input.term_set.map(|t| t.as_str()),
            input.parent_id.as_deref(),
            input.depth,
            page,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|c| pick(category_json(c), input.fields.as_deref()))
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCategoryInput {
    /// The category.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub category_id: String,
    /// Only these fields (`id` always comes back).
    #[schemars(example = ["name", "product_count"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetCategory;

impl Operation for GetCategory {
    const META: OpMeta = OpMeta {
        address: "merchandising.category.get",
        tool: "get_category",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/merchandising/categories/{category_id}",
        summary: "One category with its place in the tree, its mappings to display categories, and how many products sit in it or under it.",
        consumes: &[Slot::one("category_id")],
        produces: &[],
        checks: &[],
        errors: &[CATEGORY_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCategoryInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCategory {
    async fn read(&self, cx: &Cx, input: GetCategoryInput) -> Result<Value, ToolError> {
        check_id("category_id", "cat", &input.category_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::category(&mut tx, &input.category_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    CATEGORY_NOT_FOUND,
                    format!("There is no category {}.", input.category_id),
                )
                .detail(json!({"entity": "category", "id": input.category_id}))
                .next("list_categories")
            })?;
        let count = repo::product_count(&mut tx, &c.id).await?;
        let mappings = crate::category_write::mappings_of(&mut tx, &c.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut out = category_json(&c);
        out["description"] = json!(c.description);
        out["ancestor_ids"] = json!(c.ancestors);
        out["mappings"] = json!(
            mappings
                .iter()
                .map(
                    |m| json!({"id": m.id, "standard_category_id": m.standard_category_id,
                                "display_category_id": m.display_category_id,
                                "is_primary": m.is_primary})
                )
                .collect::<Vec<_>>()
        );
        out["product_count"] = json!(count);
        Ok(pick(out, input.fields.as_deref()))
    }
}
