//! `get_inventory` (docs/api/catalog.md#inventory).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError, fields,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{self};
use crate::repo::{self};
use crate::{PRODUCT_NOT_FOUND, product_not_found, visible_group};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetInventoryInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_chore_coat")]
    pub product_id: String,
    /// Only these top-level fields.
    #[schemars(example = ["product_id", "variants"])]
    pub fields: Option<Vec<String>>,
}

/// Stock per variant.
#[derive(Debug)]
pub struct GetInventory;

impl Operation for GetInventory {
    const META: OpMeta = OpMeta {
        address: "catalog.inventory.get",
        tool: "get_inventory",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/products/{product_id}/inventory",
        summary: "Sellable stock of each variant of a product, its safety stock, whether stock is tracked, and the availability that follows; with the product total.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("variant_id")],
        checks: &[],
        errors: &[PRODUCT_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetInventoryInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetInventory {
    async fn read(&self, cx: &Cx, input: GetInventoryInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = visible_group(cx, &mut tx, &input.product_id).await?;
        let (mut variants, ordered) = match &group {
            Some(g) => (
                repo::variants(&mut tx, std::slice::from_ref(&g.id)).await?,
                Some(crate::reorder::on_order(cx, &mut tx, &g.id).await?),
            ),
            None => (Vec::new(), None),
        };
        tx.commit().await.map_err(ToolError::from)?;
        let Some(ordered) = ordered else {
            return Err(product_not_found(
                "product",
                &input.product_id,
                "search_products",
            ));
        };
        variants.retain(|v| v.offer_id.is_some());
        model::order_variants(&mut variants);
        let summary = model::summarize(&variants);
        let lines: Vec<Value> = variants
            .iter()
            .map(|v| {
                json!({"variant_id": v.id, "sku": v.sku, "size": v.size, "color": v.color,
                       "inventory_level": v.stock(),
                       "acom:safety_stock": v.safety_stock.unwrap_or(0),
                       "acom:managed": v.is_managed, "availability": v.availability,
                       "on_order": ordered.by_variant.get(&v.id).copied().unwrap_or(0)})
            })
            .collect();
        let out = json!({"product_id": input.product_id,
                         "inventory_level": summary.inventory_level,
                         "availability": summary.availability, "variants": lines,
                         "reorders": ordered.reorders});
        fields::keep(out, &input.fields.unwrap_or_default())
    }
}
