//! `set_variants` (docs/api/catalog.md#product): the option axes and every combination of a product at once.

use std::collections::BTreeSet;

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    CostInput, INCOMPLETE_COMBINATION, OfferInput, TOO_MANY_OPTION_AXES, TOO_MANY_VARIANTS,
    VariantOptions, approval_from, customer_facing, floor_check, money_in, sku_check, variant_name,
};
use crate::model::{Variant, money_text, num};
use crate::product_group::OptionAxis;
use crate::rules::PriceFloorRule;
use crate::write::{self, NewOffer, NewVariant, VariantFields};
use crate::{BELOW_PRICE_FLOOR, DUPLICATE_SKU, PRODUCT_NOT_FOUND, product_not_found, repo, slug};

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

    use crate::product::create_product::CreateProductInput;
    use crate::product::update_product::UpdateProductInput;

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
}
