//! `get_category` (docs/api/merchandising.md#category).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError, fields,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CATEGORY_NOT_FOUND, category_json, mappings_of};
use crate::check_id;
use crate::repo;

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
        let mappings = mappings_of(&mut tx, &c.id).await?;
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
        fields::keep(out, &input.fields.unwrap_or_default())
    }
}
