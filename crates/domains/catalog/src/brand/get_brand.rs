//! `get_brand` (docs/api/catalog.md#brand).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::brand_json;
use crate::BRAND_NOT_FOUND;
use crate::repo::{self};

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
    use crate::brand::list_brands::{ListBrands, ListBrandsInput};
    use crate::testing;

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
