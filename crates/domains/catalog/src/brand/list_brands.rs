//! `list_brands` (docs/api/catalog.md#brand).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::brand_json;
use crate::model::like_pattern;
use crate::repo::{self};
use crate::{INVALID_FILTER, list};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListBrandsInput {
    /// Only brands whose name contains this text (any case).
    #[schemars(length(max = 100), example = &"north")]
    pub name: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// The store's brands.
#[derive(Debug)]
pub struct ListBrands;

impl Operation for ListBrands {
    const META: OpMeta = OpMeta {
        address: "catalog.brand.list",
        tool: "list_brands",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/brands",
        summary: "The store's brands, by name.",
        consumes: &[],
        produces: &[Slot::many("brand_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListBrandsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListBrands {
    async fn read(&self, cx: &Cx, input: ListBrandsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let name = match input.name.as_deref().map(str::trim) {
            Some("") => {
                return Err(ToolError::new(INVALID_FILTER, "`name` is empty.")
                    .detail(json!({"field": "name", "reason": "format"}))
                    .next("list_brands"));
            }
            other => other.map(like_pattern),
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (rows, total) = repo::brands(&mut tx, name.as_deref(), page.limit, page.offset).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(list(rows.iter().map(brand_json).collect(), total, &page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brand::brand_json;
    use crate::repo::Brand;
    use crate::testing;

    #[test]
    fn brands_read_like_the_spec() {
        let b = Brand {
            id: "brn_northwind".into(),
            name: "Northwind".into(),
            logo: Some("https://cdn.northwind.example/logo.svg".into()),
            description: None,
        };
        let v = brand_json(&b);
        assert_eq!(v["name"], "Northwind");
        assert_eq!(v["logo"], "https://cdn.northwind.example/logo.svg");
    }

    #[tokio::test]
    async fn an_empty_name_filter_is_refused() {
        let e = ListBrands
            .read(
                &testing::guest(),
                ListBrandsInput {
                    name: Some("  ".into()),
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_filter");
    }
}
