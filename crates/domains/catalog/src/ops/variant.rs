//! `create_product`, `update_product`, `set_variants` (docs/api/catalog.md#product): the
//! variants (sizes and colors, one SKU each) of a product, with their prices.

use std::collections::BTreeSet;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorClass,
    ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_core::enums::SaleType;
use acom_core::{Currency, Money};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::group_write::{
    CreateGroupPlan, INVALID_CATEGORY, MISSING_REQUIRED_FIELD, OptionAxis, free_product_id,
    standard_category,
};
use super::product_not_found;
use crate::model::{Variant, money_text, num};
use crate::rules::PriceFloorRule;
use crate::write::{self, NewGroup, NewOffer, NewVariant, VariantFields};
use crate::{BELOW_PRICE_FLOOR, DUPLICATE_SKU, PRODUCT_NOT_FOUND, neighbors, repo, slug};

/// More option axes than the store allows (`variant.max_option_axes`).
pub const TOO_MANY_OPTION_AXES: ErrorCode =
    ErrorCode::new("too_many_option_axes", ErrorClass::LimitExceeded);
/// More variants than one product may have (`variant.max_variants_per_group`).
pub const TOO_MANY_VARIANTS: ErrorCode =
    ErrorCode::new("too_many_variants", ErrorClass::LimitExceeded);
/// A variant names an option value the product's axes do not have.
pub const INCOMPLETE_COMBINATION: ErrorCode =
    ErrorCode::new("incomplete_combination", ErrorClass::Domain);
/// The sale type cannot be used this way.
pub const INVALID_SALE_TYPE: ErrorCode =
    ErrorCode::new("invalid_sale_type", ErrorClass::InvalidInput);

/// The color and size of one variant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariantOptions {
    /// Color name, for example Slate. Leave out for a one-color product.
    #[schemars(length(min = 1, max = 60), example = &"Slate")]
    pub color: Option<String>,
    /// Size word, for example M, 32 or 9. Leave out for a one-size product.
    #[schemars(length(min = 1, max = 20), example = &"M")]
    pub size: Option<String>,
}

/// A variant's list price.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OfferInput {
    /// List price in major units, for example 89.00.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", example = 89.00)]
    pub price: rust_decimal::Decimal,
    /// ISO 4217 code of `price`; must be the store's currency, which is the default.
    pub price_currency: Option<Currency>,
}

/// A variant's unit cost.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostInput {
    /// What one unit costs the store, in major units, for example 36.00.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", example = 36.00)]
    pub price: rust_decimal::Decimal,
    /// ISO 4217 code of the cost; must be the store's currency, which is the default.
    pub price_currency: Option<Currency>,
}

fn sale_word(s: Option<SaleType>) -> String {
    s.unwrap_or(SaleType::Always).as_str().to_owned()
}

fn variant_name(product: &str, o: &VariantOptions) -> String {
    let mut parts = vec![product.to_owned()];
    parts.extend(o.color.clone());
    parts.extend(o.size.clone());
    parts.join(", ")
}

fn money_in(
    cx: &Cx,
    field: &str,
    value: rust_decimal::Decimal,
    currency: Option<Currency>,
) -> Result<Money, ToolError> {
    let currency = currency.unwrap_or(cx.currency);
    if value.is_sign_negative() {
        return Err(
            ToolError::invalid_input(format!("`{field}` cannot be negative."))
                .detail(json!({"field": field, "reason": "range"})),
        );
    }
    if currency != cx.currency {
        return Err(ToolError::invalid_input(format!(
            "This store sells in {}, not {currency}.",
            cx.currency
        ))
        .detail(json!({"field": field, "reason": "enum", "allowed": [cx.currency.to_string()]})));
    }
    Ok(Money::new(value, currency).rounded())
}

fn floor_check(rule: &PriceFloorRule, priced: &[(String, Money, Option<Money>)]) -> Check {
    let below: Vec<String> = priced
        .iter()
        .filter_map(|(sku, price, cost)| {
            let cost = (*cost)?;
            (!rule.allows(*price, cost)).then(|| {
                format!(
                    "{sku} at {} is under the floor {}",
                    money_text(price),
                    money_text(&rule.floor(cost))
                )
            })
        })
        .collect();
    let with_cost = priced.iter().filter(|p| p.2.is_some()).count();
    Check {
        rule: "price_floor".into(),
        passed: below.is_empty(),
        detail: Some(if !below.is_empty() {
            below.join("; ")
        } else if with_cost == 0 {
            "No cost on file, so there is no floor to check".into()
        } else {
            format!(
                "{with_cost} of {with_cost} prices at or above the floor ({})",
                rule.markup_text()
            )
        }),
        fix_tool: (!below.is_empty()).then(|| "set_price".into()),
    }
}

fn sku_check(taken: &[String], skus: usize) -> Check {
    Check {
        rule: "product.sku_unique".into(),
        passed: taken.is_empty(),
        detail: Some(if taken.is_empty() {
            format!(
                "{skus} SKU{} not used elsewhere in the store",
                if skus == 1 { "" } else { "s" }
            )
        } else {
            format!("Already used in the store: {}", taken.join(", "))
        }),
        fix_tool: (!taken.is_empty()).then(|| "get_product".into()),
    }
}

fn gift_check(p: &acom_policy::PolicySet, sale_types: &[String]) -> Check {
    let disallowed: Vec<String> = p
        .lookup("catalog.gift_card.disallow")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let gift = sale_types.iter().any(|t| t == "gift_card");
    let clash: Vec<&String> = sale_types
        .iter()
        .filter(|t| disallowed.contains(t))
        .collect();
    let ok = !gift || clash.is_empty();
    Check {
        rule: "gift_card.disallow".into(),
        passed: ok,
        detail: Some(if !gift {
            "No gift card variants".into()
        } else if ok {
            "Gift cards are sold on their own".into()
        } else {
            "A gift card cannot be sold with pre-order or subscription variants".into()
        }),
        fix_tool: (!ok).then(|| "update_product".into()),
    }
}

fn customer_facing(status: &str) -> bool {
    matches!(status, "active" | "pending_review")
}

fn approval_from(cx: &Cx, tool: &str, structural: bool, reason: String) -> ApprovalNeed {
    let a = cx.policy.approval_for(
        tool,
        &json!({"change_type": if structural { "structural" } else { "content" }, "new_product": false}),
    );
    if a.needed {
        ApprovalNeed::Needed { reason }
    } else {
        ApprovalNeed::NotNeeded
    }
}

// ---------------------------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProductInput {
    /// The variant to change.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_windbreaker_m_slate")]
    pub variant_id: String,
    /// New variant name.
    #[schemars(length(min = 1, max = 200), example = &"Packable windbreaker, Slate, M")]
    pub name: Option<String>,
    /// Image URLs of the variant; the first is used.
    #[schemars(example = ["https://cdn.northwind.example/p/windbreaker-slate.jpg"])]
    pub image: Option<Vec<String>>,
    /// New color and size. A structural change on a product shoppers can see.
    #[schemars(example = &serde_json::json!({"color": "Slate", "size": "M"}))]
    pub additional_property: Option<VariantOptions>,
    /// Whether stock is tracked.
    #[serde(rename = "acom:managed", alias = "managed")]
    #[schemars(example = true)]
    pub managed: Option<bool>,
    /// How it is sold. always: from stock; pre_order: before stock arrives; subscription: on a
    /// schedule; gift_card: a digital gift card. A structural change.
    #[serde(rename = "acom:sale_type", alias = "sale_type")]
    #[schemars(example = &"pre_order")]
    pub sale_type: Option<SaleType>,
}

/// What `update_product` runs, with what it replaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateVariantPlan {
    pub variant_id: String,
    pub to: VariantFields,
    pub from: VariantFields,
}

/// Change one variant.
#[derive(Debug)]
pub struct UpdateProduct;

impl Operation for UpdateProduct {
    const META: OpMeta = OpMeta {
        address: "catalog.product.update",
        tool: "update_product",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/catalog/variants/{variant_id}",
        summary: "Change a variant's name, image, color and size, stock tracking or sale type. Content needs no approval; size, color or sale type on a product shoppers can see does. Prices are set_price.",
        consumes: &[Slot::one("variant_id")],
        produces: &[Slot::one("variant_id")],
        checks: &["product.approval", "gift_card.disallow"],
        errors: &[
            ErrorCode::INVALID_STATE,
            BELOW_PRICE_FLOOR,
            PRODUCT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateProductInput;
    type Output = Value;
}

#[derive(Debug, sqlx::FromRow)]
struct VariantRow {
    id: String,
    name: String,
    color: Option<String>,
    size: Option<String>,
    image: Option<String>,
    sale_type: String,
    is_managed: bool,
    version: i32,
    approval_status: String,
}

async fn variant_row(tx: &mut TxCx, id: &str) -> Result<Option<VariantRow>, ToolError> {
    sqlx::query_as::<_, VariantRow>(
        "SELECT p.id, p.name, p.color, p.size, p.image, p.sale_type,
                p.is_managed, p.version, g.approval_status
         FROM catalog.product p
         JOIN catalog.product_group g ON g.tenant_id = p.tenant_id AND g.id = p.product_group_id
         WHERE p.tenant_id = $1 AND p.id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(crate::db)
}

impl WriteOp<Cx> for UpdateProduct {
    type Plan = UpdateVariantPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateProductInput,
    ) -> Result<Planned<UpdateVariantPlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let v = variant_row(&mut tx, &input.variant_id)
            .await?
            .ok_or_else(|| product_not_found("variant", &input.variant_id, "search_products"))?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut to = VariantFields::default();
        let mut from = VariantFields::default();
        let (mut cf, mut ct) = (Map::new(), Map::new());
        let mut structural = false;
        if let Some(n) = input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty() && *n != v.name)
        {
            cf.insert("name".into(), json!(v.name));
            ct.insert("name".into(), json!(n));
            from.name = Some(v.name.clone());
            to.name = Some(n.to_owned());
        }
        if let Some(i) = input
            .image
            .as_ref()
            .and_then(|l| l.iter().map(|s| s.trim()).find(|s| !s.is_empty()))
            .filter(|i| Some(*i) != v.image.as_deref())
        {
            cf.insert("image".into(), json!(v.image));
            ct.insert("image".into(), json!(i));
            from.image = Some(v.image.clone());
            to.image = Some(Some(i.to_owned()));
        }
        if let Some(o) = &input.additional_property {
            if o.color.is_some() && o.color != v.color {
                structural = true;
                cf.insert("color".into(), json!(v.color));
                ct.insert("color".into(), json!(o.color));
                from.color = Some(v.color.clone());
                to.color = Some(o.color.clone());
            }
            if o.size.is_some() && o.size != v.size {
                structural = true;
                cf.insert("size".into(), json!(v.size));
                ct.insert("size".into(), json!(o.size));
                from.size = Some(v.size.clone());
                to.size = Some(o.size.clone());
            }
        }
        if let Some(m) = input.managed.filter(|m| *m != v.is_managed) {
            cf.insert("acom:managed".into(), json!(v.is_managed));
            ct.insert("acom:managed".into(), json!(m));
            from.is_managed = Some(v.is_managed);
            to.is_managed = Some(m);
        }
        let mut sale_types = vec![v.sale_type.clone()];
        if let Some(s) = input
            .sale_type
            .map(|s| s.as_str().to_owned())
            .filter(|s| *s != v.sale_type)
        {
            structural = true;
            cf.insert("acom:sale_type".into(), json!(v.sale_type));
            ct.insert("acom:sale_type".into(), json!(s));
            from.sale_type = Some(v.sale_type.clone());
            to.sale_type = Some(s.clone());
            sale_types = vec![s];
        }
        if ct.is_empty() {
            return Err(ToolError::invalid_input(format!(
                "Nothing to change: {} already has these values.",
                v.id
            ))
            .detail(json!({"field": "variant_id", "reason": "missing"})));
        }
        let live = customer_facing(&v.approval_status);
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "variant".into(),
                id: v.id.clone(),
                from: Value::Object(cf),
                to: Value::Object(ct),
                note: None,
            }],
            checks: vec![
                Check {
                    rule: "product.approval".into(),
                    passed: true,
                    detail: Some(if structural && live {
                        "Changes the structure of a variant shoppers can see; a person approves it"
                            .into()
                    } else {
                        "No approval needed for this change".into()
                    }),
                    fix_tool: None,
                },
                gift_check(&cx.policy, &sale_types),
            ],
            basis: vec![Basis {
                entity: "catalog.product".into(),
                id: v.id.clone(),
                version: i64::from(v.version),
            }],
            approval: approval_from(
                cx,
                Self::META.tool,
                structural && live,
                "Changes a variant shoppers can see".into(),
            ),
            plan: UpdateVariantPlan {
                variant_id: v.id,
                to,
                from,
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &UpdateVariantPlan,
    ) -> Result<Committed, ToolError> {
        if write::update_variant(tx, &plan.variant_id, &plan.to).await? == 0 {
            return Err(product_not_found(
                "variant",
                &plan.variant_id,
                "search_products",
            ));
        }
        let mut produced = Map::new();
        produced.insert("variant_id".into(), json!(plan.variant_id));
        Ok(Committed {
            id: plan.variant_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &UpdateVariantPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let v = variant_row(&mut tx, &plan.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match v {
            None => mismatches.push(format!("{} is missing", plan.variant_id)),
            Some(v) => {
                if plan.to.name.as_ref().is_some_and(|n| *n != v.name) {
                    mismatches.push(format!("{} is named {}", v.id, v.name));
                }
                if plan
                    .to
                    .sale_type
                    .as_ref()
                    .is_some_and(|s| *s != v.sale_type)
                {
                    mismatches.push(format!("{} is sold as {}", v.id, v.sale_type));
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
        plan: &UpdateVariantPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        write::update_variant(tx, &plan.variant_id, &plan.from)
            .await
            .map(|_| ())
    }
}

// ---------------------------------------------------------------------------------------------

/// The option axes and their values.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AxesInput {
    /// Colors the product comes in.
    #[schemars(example = ["Slate", "Sand"])]
    pub color: Option<Vec<String>>,
    /// Sizes the product comes in, smallest first.
    #[schemars(example = ["S", "M", "L"])]
    pub size: Option<Vec<String>>,
}

/// One variant in `set_variants`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariantInput {
    /// Stock keeping unit, unique in the store. An existing SKU of this product is updated.
    #[schemars(length(min = 1, max = 64), example = &"wb-sand-m")]
    pub sku: String,
    /// Its color and size: values from `varies_by`.
    #[schemars(example = &serde_json::json!({"color": "Sand", "size": "M"}))]
    pub additional_property: Option<VariantOptions>,
    /// List price. Leave out to keep the current one (or to price it later with set_price).
    #[schemars(example = &serde_json::json!({"price": 89.00, "price_currency": "USD"}))]
    pub offers: Option<OfferInput>,
    /// Operators only. Unit cost, the base of the price floor.
    #[serde(rename = "acom:cost", alias = "cost")]
    pub cost: Option<CostInput>,
    /// Whether stock is tracked. Default true.
    #[serde(rename = "acom:managed", alias = "managed")]
    #[schemars(example = true)]
    pub managed: Option<bool>,
}

/// What to do with combinations of `varies_by` that `has_variant` does not list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MissingCombination {
    /// Create a variant for every missing combination, without a price.
    CreateAll,
    /// Only the variants listed (the default).
    ListedOnly,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetVariantsInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_windbreaker")]
    pub product_id: String,
    /// The option axes and their values.
    #[schemars(example = &serde_json::json!({"color": ["Slate", "Sand"], "size": ["S", "M", "L"]}))]
    pub varies_by: AxesInput,
    /// The variants. An existing variant missing from this list stops being sold
    /// (`variant.missing_from_input: deactivate`); none is deleted.
    pub has_variant: Vec<VariantInput>,
    /// create_all: also create the combinations not listed, without a price; listed_only
    /// (default): only the listed ones.
    #[schemars(example = &"listed_only")]
    pub missing_combination: Option<MissingCombination>,
}

/// One variant `set_variants` creates or changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariantStep {
    pub variant_id: String,
    pub sku: String,
    pub name: String,
    pub options: VariantOptions,
    pub managed: bool,
    /// Creates the variant (otherwise it exists).
    pub create: bool,
    /// The variant as it was, for undo.
    pub from: Option<VariantFields>,
    /// `(offer_id, new price, new cost, old price and cost)`, when the price is set.
    pub offer: Option<OfferStep>,
}

/// A price `set_variants` sets on one variant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OfferStep {
    pub offer_id: String,
    pub price: Money,
    pub cost: Option<Money>,
    /// `None` when the offer is created.
    pub from: Option<(Money, Option<Money>)>,
}

/// What `set_variants` runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetVariantsPlan {
    pub product_id: String,
    pub varies_by: Vec<String>,
    pub from_varies_by: Vec<String>,
    pub steps: Vec<VariantStep>,
    /// `(offer_id, availability before)` of variants that stop being sold.
    pub deactivate: Vec<(String, String)>,
}

/// Set a product's option axes and variants in one go.
#[derive(Debug)]
pub struct SetVariants;

impl Operation for SetVariants {
    const META: OpMeta = OpMeta {
        address: "catalog.product.set_variants",
        tool: "set_variants",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Put,
        path: "/v1/catalog/product-groups/{product_id}/variants",
        summary: "Set a product's option axes (color, size) and its variants with prices in one call: listed variants are created or updated, variants left out stop being sold.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("variant_id")],
        checks: &[
            "variant.max_option_axes",
            "variant.max_variants_per_group",
            "product.sku_unique",
            "price_floor",
        ],
        errors: &[
            TOO_MANY_OPTION_AXES,
            TOO_MANY_VARIANTS,
            DUPLICATE_SKU,
            INCOMPLETE_COMBINATION,
            BELOW_PRICE_FLOOR,
            PRODUCT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetVariantsInput;
    type Output = Value;
}

fn axis_values(v: &Option<Vec<String>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for x in v
        .iter()
        .flatten()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        if !out.iter().any(|o| o == x) {
            out.push(x.to_owned());
        }
    }
    out
}

impl WriteOp<Cx> for SetVariants {
    type Plan = SetVariantsPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetVariantsInput,
    ) -> Result<Planned<SetVariantsPlan>, ToolError> {
        if input.has_variant.iter().any(|v| v.cost.is_some()) && !crate::is_operator(&cx.principal)
        {
            return Err(crate::operator_only("acom:cost"));
        }
        let rule = PriceFloorRule::from_policy(&cx.policy)?;
        let colors = axis_values(&input.varies_by.color);
        let sizes = axis_values(&input.varies_by.size);
        let mut axes = Vec::new();
        if !colors.is_empty() {
            axes.push(OptionAxis::Color.as_str().to_owned());
        }
        if !sizes.is_empty() {
            axes.push(OptionAxis::Size.as_str().to_owned());
        }
        let mut checks = Vec::new();
        if let Some(max) = cx
            .policy
            .lookup("catalog.variant.max_option_axes")
            .and_then(Value::as_u64)
        {
            if axes.len() as u64 > max {
                return Err(ToolError::new(TOO_MANY_OPTION_AXES, format!("{} option axes; the store allows {max}.", axes.len()))
                    .detail(json!({"limit": max, "actual": axes.len(), "policy_key": "variant.max_option_axes"}))
                    .next("set_variants"));
            }
            checks.push(Check {
                rule: "variant.max_option_axes".into(),
                passed: true,
                detail: Some(format!(
                    "{} option axes; the store allows {max}",
                    axes.len()
                )),
                fix_tool: None,
            });
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let g = repo::group(&mut tx, &input.product_id)
            .await?
            .ok_or_else(|| {
                product_not_found("product", &input.product_id, "search_product_groups")
            })?;
        let mut existing: Vec<Variant> =
            repo::variants(&mut tx, std::slice::from_ref(&g.id)).await?;
        crate::model::order_variants(&mut existing);
        // The listed variants, then the missing combinations when asked.
        let mut listed: Vec<(String, VariantOptions, &VariantInput)> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut repeated = Vec::new();
        for v in &input.has_variant {
            let sku = v.sku.trim().to_lowercase();
            let o = v.additional_property.clone().unwrap_or_default();
            let bad_color = o.color.as_ref().is_some_and(|c| !colors.contains(c))
                || (!colors.is_empty() && o.color.is_none());
            let bad_size = o.size.as_ref().is_some_and(|s| !sizes.contains(s))
                || (!sizes.is_empty() && o.size.is_none());
            if bad_color || bad_size {
                return Err(ToolError::new(
                    INCOMPLETE_COMBINATION,
                    format!("{} names a {} that `varies_by` does not have.", sku.to_uppercase(), if bad_color { "color" } else { "size" }),
                )
                .detail(json!({"field": "has_variant", "sku": sku, "allowed": {"color": colors, "size": sizes}}))
                .next("set_variants"));
            }
            if !seen.insert(sku.clone()) {
                repeated.push(sku.to_uppercase());
            }
            listed.push((sku, o, v));
        }
        let mut extra: Vec<(String, VariantOptions)> = Vec::new();
        if input.missing_combination == Some(MissingCombination::CreateAll) {
            let cs: Vec<Option<String>> = if colors.is_empty() {
                vec![None]
            } else {
                colors.iter().cloned().map(Some).collect()
            };
            let ss: Vec<Option<String>> = if sizes.is_empty() {
                vec![None]
            } else {
                sizes.iter().cloned().map(Some).collect()
            };
            let base = g.id.trim_start_matches("prd_").replace('_', "-");
            for c in &cs {
                for s in &ss {
                    let o = VariantOptions {
                        color: c.clone(),
                        size: s.clone(),
                    };
                    let covered = listed.iter().any(|(_, lo, _)| *lo == o)
                        || existing
                            .iter()
                            .any(|e| e.color == o.color && e.size == o.size);
                    if !covered {
                        let mut sku = base.clone();
                        for part in [&o.color, &o.size].into_iter().flatten() {
                            sku.push('-');
                            sku.push_str(&part.to_lowercase());
                        }
                        extra.push((sku, o));
                    }
                }
            }
        }
        let all_skus: Vec<String> = listed
            .iter()
            .map(|l| l.0.clone())
            .chain(extra.iter().map(|e| e.0.clone()))
            .collect();
        let taken = write::taken_skus(&mut tx, &all_skus).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let mut taken_elsewhere: Vec<String> = taken
            .iter()
            .filter(|(_, owner)| **owner != g.id)
            .map(|(s, _)| s.to_uppercase())
            .collect();
        taken_elsewhere.extend(repeated);
        let max_variants = cx
            .policy
            .lookup("catalog.variant.max_variants_per_group")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX);
        let mut steps = Vec::new();
        let mut priced = Vec::new();
        let mut changes = Vec::new();
        for (sku, o, v) in &listed {
            let found = existing.iter().find(|e| e.sku.to_lowercase() == *sku);
            let price = v
                .offers
                .as_ref()
                .map(|p| money_in(cx, "has_variant.offers.price", p.price, p.price_currency))
                .transpose()?;
            let cost = v
                .cost
                .as_ref()
                .map(|c| money_in(cx, "has_variant.acom:cost", c.price, c.price_currency))
                .transpose()?;
            let name = variant_name(&g.name, o);
            match found {
                Some(e) => {
                    let mut from = VariantFields::default();
                    let mut to_fields = Map::new();
                    let mut from_fields = Map::new();
                    if o.color != e.color {
                        from.color = Some(e.color.clone());
                        from_fields.insert("color".into(), json!(e.color));
                        to_fields.insert("color".into(), json!(o.color));
                    }
                    if o.size != e.size {
                        from.size = Some(e.size.clone());
                        from_fields.insert("size".into(), json!(e.size));
                        to_fields.insert("size".into(), json!(o.size));
                    }
                    let managed = v.managed.unwrap_or(e.is_managed);
                    if managed != e.is_managed {
                        from.is_managed = Some(e.is_managed);
                        from_fields.insert("acom:managed".into(), json!(e.is_managed));
                        to_fields.insert("acom:managed".into(), json!(managed));
                    }
                    let offer = match (price, &e.offer_id) {
                        (Some(p), Some(off))
                            if Some(p.value) != e.price
                                || (cost.is_some() && cost.map(|c| c.value) != e.cost_value) =>
                        {
                            from_fields.insert("price".into(), e.price.map_or(Value::Null, num));
                            to_fields.insert("price".into(), num(p.value));
                            Some(OfferStep {
                                offer_id: off.clone(),
                                price: p,
                                cost,
                                from: e.list_price().map(|lp| (lp, e.cost())),
                            })
                        }
                        (Some(p), None) => {
                            to_fields.insert("price".into(), num(p.value));
                            Some(OfferStep {
                                offer_id: format!("off_{}", e.id.trim_start_matches("var_")),
                                price: p,
                                cost,
                                from: None,
                            })
                        }
                        _ => None,
                    };
                    if let Some(p) = price {
                        priced.push((sku.to_uppercase(), p, cost.or_else(|| e.cost())));
                    }
                    if to_fields.is_empty() {
                        continue;
                    }
                    changes.push(Change {
                        op: ChangeOp::Update,
                        entity: "variant".into(),
                        id: e.id.clone(),
                        from: Value::Object(from_fields),
                        to: Value::Object(to_fields),
                        note: None,
                    });
                    steps.push(VariantStep {
                        variant_id: e.id.clone(),
                        sku: sku.clone(),
                        name,
                        options: o.clone(),
                        managed,
                        create: false,
                        from: Some(from),
                        offer,
                    });
                }
                None => {
                    let variant_id = format!("var_{}", slug(sku));
                    let offer = price.map(|p| OfferStep {
                        offer_id: format!("off_{}", slug(sku)),
                        price: p,
                        cost,
                        from: None,
                    });
                    if let Some(p) = price {
                        priced.push((sku.to_uppercase(), p, cost));
                    }
                    changes.push(Change {
                        op: ChangeOp::Create,
                        entity: "variant".into(),
                        id: String::new(),
                        from: Value::Null,
                        to: json!({"variant_id": variant_id, "sku": sku.to_uppercase(), "color": o.color, "size": o.size,
                                   "price": price.map(|p| num(p.value)), "cost": cost.map(|c| num(c.value))}),
                        note: None,
                    });
                    steps.push(VariantStep {
                        variant_id,
                        sku: sku.clone(),
                        name,
                        options: o.clone(),
                        managed: v.managed.unwrap_or(true),
                        create: true,
                        from: None,
                        offer,
                    });
                }
            }
        }
        for (sku, o) in &extra {
            let variant_id = format!("var_{}", slug(sku));
            changes.push(Change {
                op: ChangeOp::Create,
                entity: "variant".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"variant_id": variant_id, "sku": sku.to_uppercase(), "color": o.color, "size": o.size, "price": null}),
                note: Some("A combination not listed; price it with set_price".into()),
            });
            steps.push(VariantStep {
                variant_id,
                sku: sku.clone(),
                name: variant_name(&g.name, o),
                options: o.clone(),
                managed: true,
                create: true,
                from: None,
                offer: None,
            });
        }
        let listed_skus: BTreeSet<&str> = listed.iter().map(|l| l.0.as_str()).collect();
        let mut deactivate = Vec::new();
        for e in &existing {
            if listed_skus.contains(e.sku.to_lowercase().as_str()) {
                continue;
            }
            if let (Some(off), Some(a)) = (&e.offer_id, &e.availability)
                && a != "discontinued"
            {
                changes.push(Change {
                    op: ChangeOp::Update,
                    entity: "variant".into(),
                    id: e.id.clone(),
                    from: json!({"availability": a}),
                    to: json!({"availability": "discontinued"}),
                    note: Some(
                        "Not in the list: it stops being sold (variant.missing_from_input)".into(),
                    ),
                });
                deactivate.push((off.clone(), a.clone()));
            }
        }
        let total = existing.len() + steps.iter().filter(|s| s.create).count();
        if total as u64 > max_variants {
            return Err(ToolError::new(TOO_MANY_VARIANTS, format!("{total} variants; a product may have {max_variants}."))
                .detail(json!({"limit": max_variants, "actual": total, "policy_key": "variant.max_variants_per_group"}))
                .next("set_variants"));
        }
        if axes != g.varies_by {
            changes.insert(
                0,
                Change {
                    op: ChangeOp::Update,
                    entity: "product".into(),
                    id: g.id.clone(),
                    from: json!({"varies_by": g.varies_by}),
                    to: json!({"varies_by": axes}),
                    note: None,
                },
            );
        }
        checks.push(Check {
            rule: "variant.max_variants_per_group".into(),
            passed: true,
            detail: Some(format!(
                "{total} variants; a product may have {max_variants}"
            )),
            fix_tool: None,
        });
        checks.push(sku_check(&taken_elsewhere, all_skus.len()));
        checks.push(floor_check(&rule, &priced));
        let structural = customer_facing(&g.approval_status);
        let created = steps.iter().filter(|s| s.create).count();
        Ok(Planned {
            approval: approval_from(
                cx,
                Self::META.tool,
                structural,
                format!(
                    "Changes the variants of a product shoppers can see ({created} new, {} stop selling)",
                    deactivate.len()
                ),
            ),
            plan: SetVariantsPlan {
                product_id: g.id.clone(),
                varies_by: axes,
                from_varies_by: g.varies_by.clone(),
                steps,
                deactivate,
            },
            changes,
            checks,
            basis: vec![Basis {
                entity: "catalog.product_group".into(),
                id: g.id,
                version: i64::from(g.version),
            }],
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SetVariantsPlan) -> Result<Committed, ToolError> {
        if plan.varies_by != plan.from_varies_by {
            write::update_group(
                tx,
                &plan.product_id,
                &write::GroupFields {
                    varies_by: Some(plan.varies_by.clone()),
                    ..Default::default()
                },
            )
            .await?;
        }
        for s in &plan.steps {
            if s.create {
                write::insert_variant(
                    tx,
                    &NewVariant {
                        id: &s.variant_id,
                        product_group_id: &plan.product_id,
                        sku: &s.sku,
                        name: &s.name,
                        color: s.options.color.as_deref(),
                        size: s.options.size.as_deref(),
                        image: None,
                        sale_type: "always",
                        is_managed: s.managed,
                    },
                )
                .await?;
            } else {
                write::update_variant(
                    tx,
                    &s.variant_id,
                    &VariantFields {
                        color: Some(s.options.color.clone()),
                        size: Some(s.options.size.clone()),
                        is_managed: Some(s.managed),
                        ..Default::default()
                    },
                )
                .await?;
            }
            if let Some(o) = &s.offer {
                match o.from {
                    None => {
                        write::insert_offer(
                            tx,
                            &NewOffer {
                                id: &o.offer_id,
                                variant_id: &s.variant_id,
                                price: o.price.value,
                                price_currency: o.price.currency.as_str(),
                                availability: "out_of_stock",
                                inventory_level: 0,
                                cost: o.cost.as_ref().map(|c| (c.value, c.currency.as_str())),
                            },
                        )
                        .await?;
                    }
                    Some(_) => {
                        let cost = o
                            .cost
                            .map(|c| (Some(c.value), Some(c.currency.to_string())));
                        repo::set_offer_price(
                            tx,
                            &o.offer_id,
                            o.price.value,
                            o.price.currency.as_str(),
                            cost,
                        )
                        .await?;
                    }
                }
            }
        }
        for (offer, _) in &plan.deactivate {
            write::set_availability(tx, offer, "discontinued").await?;
        }
        let ids: Vec<&str> = plan.steps.iter().map(|s| s.variant_id.as_str()).collect();
        let mut produced = Map::new();
        produced.insert("variant_id".into(), json!(ids));
        produced.insert("product_id".into(), json!(plan.product_id));
        Ok(Committed {
            id: plan.product_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SetVariantsPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let now = repo::variants(&mut tx, std::slice::from_ref(&plan.product_id)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        for s in &plan.steps {
            match now.iter().find(|v| v.id == s.variant_id) {
                None => mismatches.push(format!("{} is missing", s.variant_id)),
                Some(v) => {
                    if let Some(o) = &s.offer
                        && v.price != Some(o.price.value)
                    {
                        mismatches.push(format!("{} is not priced {}", v.id, money_text(&o.price)));
                    }
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
        plan: &SetVariantsPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        for (offer, availability) in &plan.deactivate {
            write::set_availability(tx, offer, availability).await?;
        }
        for s in plan.steps.iter().rev() {
            if s.create {
                write::delete_variant(tx, &s.variant_id).await?;
                continue;
            }
            if let Some(from) = &s.from {
                write::update_variant(tx, &s.variant_id, from).await?;
            }
            if let Some(o) = &s.offer {
                match &o.from {
                    Some((price, cost)) => {
                        let cost =
                            Some((cost.map(|c| c.value), cost.map(|c| c.currency.to_string())));
                        repo::set_offer_price(
                            tx,
                            &o.offer_id,
                            price.value,
                            price.currency.as_str(),
                            cost,
                        )
                        .await?;
                    }
                    None => write::delete_offer(tx, &o.offer_id).await?,
                }
            }
        }
        if plan.varies_by != plan.from_varies_by {
            write::update_group(
                tx,
                &plan.product_id,
                &write::GroupFields {
                    varies_by: Some(plan.from_varies_by.clone()),
                    ..Default::default()
                },
            )
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;

    #[test]
    fn inputs_are_closed_flat_schemas() {
        for schema in [
            acom_contract::schema::closed_schema_for::<CreateProductInput>(),
            acom_contract::schema::closed_schema_for::<UpdateProductInput>(),
            acom_contract::schema::closed_schema_for::<SetVariantsInput>(),
        ] {
            assert!(
                acom_contract::schema::violations(&schema).is_empty(),
                "{schema:#}"
            );
        }
        let i: SetVariantsInput = serde_json::from_value(json!({
            "product_id": "prd_windbreaker",
            "varies_by": {"color": ["Slate"], "size": ["S", "M"]},
            "has_variant": [{"sku": "wb-slate-s", "additional_property": {"color": "Slate", "size": "S"},
                             "offers": {"price": 89.00, "price_currency": "USD"}}],
            "missing_combination": "create_all"}))
        .unwrap();
        assert_eq!(i.missing_combination, Some(MissingCombination::CreateAll));
        assert_eq!(axis_values(&i.varies_by.size), ["S", "M"]);
    }

    #[test]
    fn names_and_floors_of_new_variants() {
        let o = VariantOptions {
            color: Some("Sand".into()),
            size: Some("M".into()),
        };
        assert_eq!(
            variant_name("Packable windbreaker", &o),
            "Packable windbreaker, Sand, M"
        );
        let rule = PriceFloorRule::from_policy(&crate::testing::policies()).unwrap();
        let usd = |s: &str| Money::new(s.parse().unwrap(), Currency::USD);
        let c = floor_check(
            &rule,
            &[("WB-SAND-M".into(), usd("40.00"), Some(usd("36.00")))],
        );
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("WB-SAND-M at $40.00 is under the floor $41.40")
        );
        assert!(floor_check(&rule, &[("X".into(), usd("40.00"), None)]).passed);
    }

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
