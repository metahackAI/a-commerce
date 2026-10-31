//! `create_product` (docs/api/catalog.md#product): one new variant, with a single-variant product when it has none.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::SaleType;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    CostInput, INVALID_SALE_TYPE, OfferInput, VariantOptions, approval_from, customer_facing,
    floor_check, gift_check, money_in, sale_word, sku_check,
};
use crate::model::{money_text, num};
use crate::product_group::{
    CreateGroupPlan, INVALID_CATEGORY, MISSING_REQUIRED_FIELD, free_product_id, standard_category,
};
use crate::rules::PriceFloorRule;
use crate::write::{self, NewGroup, NewOffer, NewVariant};
use crate::{
    BELOW_PRICE_FLOOR, DUPLICATE_SKU, PRODUCT_NOT_FOUND, neighbors, product_not_found, repo, slug,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProductInput {
    /// The product the variant belongs to. Leave out to create a one-variant product as well.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_windbreaker")]
    pub product_id: Option<String>,
    /// Stock keeping unit, unique in the store.
    #[schemars(length(min = 1, max = 64), example = &"wb-sand-m")]
    pub sku: String,
    /// The variant's name, for example "Packable windbreaker, Sand, M".
    #[schemars(length(min = 1, max = 200), example = &"Packable windbreaker, Sand, M")]
    pub name: String,
    /// Standard category (`list_categories`). Required when `product_id` is left out.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub category_id: Option<String>,
    /// Brand of a new one-variant product (`list_brands`).
    #[schemars(regex(pattern = r"^brn_[a-z0-9_]+$"), example = &"brn_northwind")]
    pub brand_id: Option<String>,
    /// Image URLs of the variant; the first is used.
    #[schemars(example = ["https://cdn.northwind.example/p/windbreaker-sand.jpg"])]
    pub image: Option<Vec<String>>,
    /// The variant's color and size.
    #[schemars(example = &serde_json::json!({"color": "Sand", "size": "M"}))]
    pub additional_property: Option<VariantOptions>,
    /// First list price. Leave out and set it later with set_price.
    #[schemars(example = &serde_json::json!({"price": 89.00, "price_currency": "USD"}))]
    pub offers: Option<OfferInput>,
    /// Operators only. Unit cost, the base of the price floor.
    #[serde(rename = "acom:cost", alias = "cost")]
    pub cost: Option<CostInput>,
    /// Whether stock is tracked. Default true.
    #[serde(rename = "acom:managed", alias = "managed")]
    #[schemars(example = true)]
    pub managed: Option<bool>,
    /// How it is sold. always (default): from stock; pre_order: before stock arrives;
    /// subscription: on a schedule; gift_card: a digital gift card.
    #[serde(rename = "acom:sale_type", alias = "sale_type")]
    #[schemars(example = &"always")]
    pub sale_type: Option<SaleType>,
}

/// What `create_product` runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateVariantPlan {
    /// The one-variant product made with it, if any.
    pub new_group: Option<CreateGroupPlan>,
    pub product_id: String,
    pub variant_id: String,
    pub sku: String,
    pub name: String,
    pub options: VariantOptions,
    pub image: Option<String>,
    pub sale_type: String,
    pub managed: bool,
    /// `(offer_id, price, cost)` when a price is given.
    pub offer: Option<(String, Money, Option<Money>)>,
}

/// Create one variant (and its product when there is none).
#[derive(Debug)]
pub struct CreateProduct;

impl Operation for CreateProduct {
    const META: OpMeta = OpMeta {
        address: "catalog.product.create",
        tool: "create_product",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/variants",
        summary: "Create one variant (a SKU) of a product, or a one-variant product. For several sizes and colors at once use set_variants.",
        consumes: &[Slot::one("product_id"), Slot::one("brand_id")],
        produces: &[Slot::one("variant_id"), Slot::one("product_id")],
        checks: &[
            "product.required_on_create",
            "product.sku_unique",
            "price_floor",
            "gift_card.disallow",
        ],
        errors: &[
            DUPLICATE_SKU,
            BELOW_PRICE_FLOOR,
            MISSING_REQUIRED_FIELD,
            INVALID_SALE_TYPE,
            INVALID_CATEGORY,
            PRODUCT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateProductInput;
    type Output = Value;
}

impl WriteOp<Cx> for CreateProduct {
    type Plan = CreateVariantPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateProductInput,
    ) -> Result<Planned<CreateVariantPlan>, ToolError> {
        if input.cost.is_some() && !crate::is_operator(&cx.principal) {
            return Err(crate::operator_only("acom:cost"));
        }
        let sku = input.sku.trim().to_lowercase();
        let name = input.name.trim().to_owned();
        if sku.is_empty() || name.is_empty() {
            return Err(ToolError::new(
                MISSING_REQUIRED_FIELD,
                "A variant needs a SKU and a name.",
            )
            .detail(
                json!({"field": if sku.is_empty() { "sku" } else { "name" }, "reason": "missing"}),
            )
            .next("create_product"));
        }
        let sale_type = sale_word(input.sale_type);
        if sale_type == "gift_card" && input.managed == Some(true) {
            return Err(ToolError::new(
                INVALID_SALE_TYPE,
                "A gift card is digital; its stock is not tracked.",
            )
            .detail(json!({"field": "acom:managed", "reason": "enum"}))
            .next("update_product"));
        }
        let rule = PriceFloorRule::from_policy(&cx.policy)?;
        let options = input.additional_property.unwrap_or_default();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (product_id, new_group, status) = match &input.product_id {
            Some(pid) => {
                let g = repo::group(&mut tx, pid)
                    .await?
                    .ok_or_else(|| product_not_found("product", pid, "search_product_groups"))?;
                (g.id, None, g.approval_status)
            }
            None => {
                let Some(cat) = &input.category_id else {
                    return Err(ToolError::new(
                        MISSING_REQUIRED_FIELD,
                        "A variant without a product needs a category for the product made with it.",
                    )
                    .detail(json!({"field": "category_id", "reason": "missing"}))
                    .next("create_product"));
                };
                let term = standard_category(&mut tx, cat).await?;
                let base = name.split(',').next().unwrap_or(&name).trim().to_owned();
                let id = free_product_id(&mut tx, &base).await?;
                let g = CreateGroupPlan {
                    product_id: id.clone(),
                    name: base,
                    description: None,
                    brand_id: input.brand_id.clone(),
                    category_id: term.id,
                    image: None,
                    varies_by: Vec::new(),
                    material: None,
                };
                (id, Some(g), "draft".to_owned())
            }
        };
        let taken = write::taken_skus(&mut tx, std::slice::from_ref(&sku)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let variant_id = format!("var_{}", slug(&sku));
        let price = input
            .offers
            .as_ref()
            .map(|o| money_in(cx, "offers.price", o.price, o.price_currency))
            .transpose()?;
        let cost = input
            .cost
            .as_ref()
            .map(|c| money_in(cx, "acom:cost", c.price, c.price_currency))
            .transpose()?;
        let offer = price.map(|p| (format!("off_{}", slug(&sku)), p, cost));
        let taken_list: Vec<String> = taken.keys().map(|s| s.to_uppercase()).collect();
        let mut checks = vec![
            Check {
                rule: "product.required_on_create".into(),
                passed: true,
                detail: Some("It has a name and a category; the store is the seller".into()),
                fix_tool: None,
            },
            sku_check(&taken_list, 1),
        ];
        if let Some((_, p, c)) = &offer {
            checks.push(floor_check(&rule, &[(sku.to_uppercase(), *p, *c)]));
        }
        checks.push(gift_check(&cx.policy, std::slice::from_ref(&sale_type)));
        let mut changes = Vec::new();
        if let Some(g) = &new_group {
            changes.push(Change {
                op: ChangeOp::Create,
                entity: "product".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"product_id": g.product_id, "name": g.name, "category_id": g.category_id,
                           "brand_id": g.brand_id, "approval_status": "draft"}),
                note: None,
            });
        }
        changes.push(Change {
            op: ChangeOp::Create,
            entity: "variant".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"variant_id": variant_id, "product_id": product_id, "sku": sku.to_uppercase(),
                       "name": name, "color": options.color, "size": options.size,
                       "acom:sale_type": sale_type, "acom:managed": input.managed.unwrap_or(sale_type != "gift_card")}),
            note: None,
        });
        if let Some((offer_id, p, c)) = &offer {
            changes.push(Change {
                op: ChangeOp::Create,
                entity: "offer".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"offer_id": offer_id, "price": num(p.value), "price_currency": p.currency.to_string(),
                           "cost": c.map(|c| num(c.value)), "inventory_level": 0, "availability": "out_of_stock"}),
                note: None,
            });
        }
        let structural = offer.is_some() && customer_facing(&status);
        Ok(Planned {
            approval: approval_from(
                cx,
                Self::META.tool,
                structural,
                "Adds a priced variant to a product shoppers can see".into(),
            ),
            plan: CreateVariantPlan {
                new_group,
                product_id,
                variant_id,
                sku,
                name,
                options,
                image: input.image.and_then(|l| {
                    l.into_iter()
                        .map(|s| s.trim().to_owned())
                        .find(|s| !s.is_empty())
                }),
                managed: input.managed.unwrap_or(sale_type != "gift_card"),
                sale_type,
                offer,
            },
            changes,
            checks,
            basis: Vec::new(),
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &CreateVariantPlan,
    ) -> Result<Committed, ToolError> {
        let taken = write::taken_skus(tx, std::slice::from_ref(&plan.sku)).await?;
        if !taken.is_empty() {
            return Err(ToolError::new(
                DUPLICATE_SKU,
                format!("SKU {} is already used.", plan.sku.to_uppercase()),
            )
            .detail(
                json!({"entity": "variant", "field": "sku", "existing_id": taken.values().next()}),
            )
            .next("get_product"));
        }
        if let Some(g) = &plan.new_group {
            write::insert_group(
                tx,
                &NewGroup {
                    id: &g.product_id,
                    name: &g.name,
                    description: None,
                    brand_id: g.brand_id.as_deref(),
                    varies_by: &g.varies_by,
                    material: None,
                    image: None,
                },
            )
            .await?;
            neighbors::place_product(tx, &g.product_id, &g.category_id).await?;
        }
        write::insert_variant(
            tx,
            &NewVariant {
                id: &plan.variant_id,
                product_group_id: &plan.product_id,
                sku: &plan.sku,
                name: &plan.name,
                color: plan.options.color.as_deref(),
                size: plan.options.size.as_deref(),
                image: plan.image.as_deref(),
                sale_type: &plan.sale_type,
                is_managed: plan.managed,
            },
        )
        .await?;
        if let Some((offer_id, p, c)) = &plan.offer {
            write::insert_offer(
                tx,
                &NewOffer {
                    id: offer_id,
                    variant_id: &plan.variant_id,
                    price: p.value,
                    price_currency: p.currency.as_str(),
                    availability: "out_of_stock",
                    inventory_level: 0,
                    cost: c.as_ref().map(|c| (c.value, c.currency.as_str())),
                },
            )
            .await?;
        }
        let mut produced = Map::new();
        produced.insert("variant_id".into(), json!(plan.variant_id));
        produced.insert("product_id".into(), json!(plan.product_id));
        Ok(Committed {
            id: plan.variant_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CreateVariantPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let v = repo::variant(&mut tx, &plan.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match v {
            None => mismatches.push(format!("{} was not created", plan.variant_id)),
            Some(v) => {
                if let Some((_, p, _)) = &plan.offer
                    && v.price != Some(p.value)
                {
                    mismatches.push(format!("{} is not priced {}", v.id, money_text(p)));
                }
            }
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &CreateVariantPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        write::delete_variant(tx, &plan.variant_id).await?;
        if let Some(g) = &plan.new_group {
            write::delete_draft(tx, &g.product_id).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    async fn shoppers_cannot_set_costs_and_a_variant_needs_a_name() {
        let i: CreateProductInput = serde_json::from_value(json!({
            "sku": "x-1", "name": "X", "category_id": "cat_x",
            "acom:cost": {"price": 1.00, "price_currency": "USD"}}))
        .unwrap();
        assert_eq!(
            CreateProduct
                .preview(&testing::shopper(), i)
                .await
                .unwrap_err()
                .code,
            "forbidden"
        );
        let op = testing::cx(acom_core::enums::ActorKind::Operator, "adm_sj");
        let i: CreateProductInput =
            serde_json::from_value(json!({"sku": "x-1", "name": " "})).unwrap();
        assert_eq!(
            CreateProduct.preview(&op, i).await.unwrap_err().code,
            "missing_required_field"
        );
    }
}
