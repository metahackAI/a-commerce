//! `list_brands`, `get_brand` (docs/api/catalog.md#brand).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::like_pattern;
use crate::repo::{self, Brand};
use crate::{BRAND_NOT_FOUND, INVALID_FILTER, list};

fn brand_json(b: &Brand) -> Value {
    json!({"id": b.id, "@type": "Brand", "name": b.name, "logo": b.logo,
           "description": b.description})
}

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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetBrandInput {
    /// The brand.
    #[schemars(regex(pattern = r"^brn_[a-z0-9_]+$"), example = &"brn_northwind")]
    pub brand_id: String,
}

/// One brand.
#[derive(Debug)]
pub struct GetBrand;

impl Operation for GetBrand {
    const META: OpMeta = OpMeta {
        address: "catalog.brand.get",
        tool: "get_brand",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/brands/{brand_id}",
        summary: "One brand: name and logo.",
        consumes: &[Slot::one("brand_id")],
        produces: &[],
        checks: &[],
        errors: &[BRAND_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetBrandInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetBrand {
    async fn read(&self, cx: &Cx, input: GetBrandInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let b = repo::brand(&mut tx, &input.brand_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let b = b.ok_or_else(|| {
            ToolError::new(
                BRAND_NOT_FOUND,
                format!("There is no brand {}.", input.brand_id),
            )
            .detail(json!({"entity": "brand", "id": input.brand_id}))
            .next("list_brands")
        })?;
        Ok(brand_json(&b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;

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

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn northwind_is_listed_and_found() {
        let cx = testing::shopper_db().await;
        let out = ListBrands
            .read(
                &cx,
                ListBrandsInput {
                    name: Some("north".into()),
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["total"], json!(1));
        let b = GetBrand
            .read(
                &cx,
                GetBrandInput {
                    brand_id: "brn_northwind".into(),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(b["name"], "Northwind");
    }
}
