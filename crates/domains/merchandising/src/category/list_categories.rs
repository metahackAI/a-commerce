//! `list_categories` (docs/api/merchandising.md#category).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError, fields,
};
use acom_core::enums::TermSet;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::category_json;
use crate::check_id;
use crate::repo;

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
        let items: Vec<Value> = rows.iter().map(category_json).collect();
        let items = fields::keep_each(items, &input.fields.unwrap_or_default())?;
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
