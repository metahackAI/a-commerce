//! `get_product_group` (docs/api/catalog.md#product_group).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError, fields,
};
use acom_core::StoreTime;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::draft_json;
use crate::model::{self, num};
use crate::repo::{self, PendingRow};
use crate::write::{self};
use crate::{PRODUCT_NOT_FOUND, is_operator, neighbors, product_not_found, visible_group};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetProductGroupInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_windbreaker")]
    pub product_id: String,
    /// Only these top-level fields.
    #[schemars(example = ["id", "approval_status", "has_variant"])]
    pub fields: Option<Vec<String>>,
}

/// One product as the catalog manages it.
#[derive(Debug)]
pub struct GetProductGroup;

impl Operation for GetProductGroup {
    const META: OpMeta = OpMeta {
        address: "catalog.product_group.get",
        tool: "get_product_group",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/product-groups/{product_id}",
        summary: "One product as the catalog manages it: review state, option axes, every variant with its offer, and changes waiting for approval. For selling prices and stock use get_product.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("variant_id")],
        checks: &[],
        errors: &[PRODUCT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetProductGroupInput;
    type Output = Value;
}

fn pending_json(p: &PendingRow, time: &StoreTime) -> Value {
    json!({"preview_id": p.id, "change_set_id": p.change_set_id, "tool": p.instrument,
           "op": p.change.get("op"), "entity": p.change.get("entity"), "id": p.change.get("id"),
           "from": p.change.get("from"), "to": p.change.get("to"),
           "created_at": time.format(p.created_at)})
}

impl ReadOp<Cx> for GetProductGroup {
    async fn read(&self, cx: &Cx, input: GetProductGroupInput) -> Result<Value, ToolError> {
        let operator = is_operator(&cx.principal);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let Some(g) = visible_group(cx, &mut tx, &input.product_id).await? else {
            return Err(product_not_found(
                "product",
                &input.product_id,
                "search_product_groups",
            ));
        };
        let mut variants = repo::variants(&mut tx, std::slice::from_ref(&g.id)).await?;
        model::order_variants(&mut variants);
        let mut records = vec![g.id.clone()];
        for v in &variants {
            records.push(v.id.clone());
            records.extend(v.offer_id.clone());
        }
        let pending = if operator {
            repo::pending(&mut tx, &records).await?
        } else {
            Vec::new()
        };
        let category = neighbors::categories(&mut tx, Some(std::slice::from_ref(&g.id)))
            .await?
            .into_iter()
            .filter(|c| c.direct)
            .max_by_key(|c| c.is_primary)
            .map(|c| c.category_id);
        let draft = if operator {
            write::items_of(&mut tx, std::slice::from_ref(&g.id))
                .await?
                .pop()
        } else {
            None
        };
        let offers: Vec<String> = variants.iter().filter_map(|v| v.offer_id.clone()).collect();
        let schedules = if operator {
            write::schedules(&mut tx, &offers).await?
        } else {
            Vec::new()
        };
        tx.commit().await.map_err(ToolError::from)?;
        let has_variant: Vec<Value> = variants
            .iter()
            .map(|v| {
                let mut line = json!({"variant_id": v.id, "sku": v.sku, "name": v.name,
                    "size": v.size, "color": v.color, "acom:sale_type": v.sale_type,
                    "acom:managed": v.is_managed, "offer_id": v.offer_id,
                    "price": v.price.map_or(Value::Null, num), "price_currency": v.price_currency,
                    "availability": v.availability, "inventory_level": v.inventory_level,
                    "acom:safety_stock": v.safety_stock});
                if operator {
                    line["cost"] = v.cost_value.map_or(Value::Null, num);
                    let mine: Vec<Value> = schedules
                        .iter()
                        .filter(|s| Some(&s.offer_id) == v.offer_id.as_ref())
                        .map(|s| json!({"unit_price_specification_id": s.id, "price": num(s.price),
                                        "valid_from": cx.time.format(s.valid_from),
                                        "valid_through": s.valid_through.map(|t| cx.time.format(t))}))
                        .collect();
                    if !mine.is_empty() {
                        line["scheduled_prices"] = json!(mine);
                    }
                }
                line
            })
            .collect();
        let out = json!({
            "id": g.id, "@type": "ProductGroup", "name": g.name, "description": g.description,
            "brand_id": g.brand_id, "category_id": category, "approval_status": g.approval_status,
            "varies_by": g.varies_by, "material": g.material, "keywords": g.keywords,
            "additional_property": model::properties_json(g.material.as_deref(), &g.additional_property),
            "image": g.image, "release_date": g.release_date.map(|d| d.to_string()),
            "has_merchant_return_policy_id": g.has_merchant_return_policy_id,
            "has_variant": has_variant,
            "pending_changes": pending.iter().map(|p| pending_json(p, &cx.time)).collect::<Vec<_>>(),
            "version": g.version, "updated_at": cx.time.format(g.updated_at),
        });
        let mut out = out;
        if let Some(d) = &draft {
            out["acom:draft"] = draft_json(d);
        }
        fields::keep(out, &input.fields.unwrap_or_default())
    }
}
