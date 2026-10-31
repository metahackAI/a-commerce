//! `create_bundle` (docs/api/catalog.md#bundle): a new product sold as a set of variants. Its
//! price is the components' list prices less an optional bundle discount, its cost theirs, and
//! its stock how many whole sets the components' stock makes. Components must be on sale and
//! share a seller and an age limit (`bundle`). A price is set, so a person approves it
//! (`approval.yml thresholds.product_approve`); once approved the bundle is on sale. Undo takes
//! it off sale (discontinued): a bundle that may have been ordered is not deleted.

use std::collections::BTreeSet;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::{Currency, Money};
use acom_store::{Cx, TxCx};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::model::{money_of, money_text, num};
use crate::product_group::{
    INVALID_CATEGORY, MISSING_REQUIRED_FIELD, free_product_id, standard_category,
};
use crate::rules::{BundleRules, PriceFloorRule};
use crate::write::{self, NewGroup, NewOffer, NewVariant};
use crate::{
    BELOW_PRICE_FLOOR, COMPONENT_INACTIVE, DUPLICATE_SKU, MIXED_AGE_RESTRICTION, MIXED_SELLER,
    PRODUCT_NOT_FOUND, neighbors, product_not_found, repo,
};

/// One component of the bundle.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundleComponentInput {
    /// The component variant (a size and color, from `get_product`).
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_denim_jacket_m_indigo")]
    pub variant_id: String,
    /// Units of it in one bundle. Default 1.
    #[schemars(range(min = 1), example = 1)]
    pub order_quantity: Option<u32>,
}

/// The bundle's discount on the sum of its components' list prices.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundleDiscountInput {
    /// Percent off the components' total, 0–100; at most the store's `bundle.max_discount`.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", range(min = 0, max = 100), example = 10)]
    pub percent_off: Decimal,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateBundleInput {
    /// The bundle's name.
    #[schemars(length(min = 1, max = 200), example = &"Denim bundle")]
    pub name: String,
    /// The components, at least `bundle.min_components` different variants.
    #[schemars(example = &serde_json::json!([{"variant_id": "var_denim_jacket_m_indigo", "order_quantity": 1}, {"variant_id": "var_linen_shirt_m_white", "order_quantity": 1}]))]
    pub is_related_to: Vec<BundleComponentInput>,
    /// Standard category of the bundle (`list_categories`).
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_outerwear")]
    pub category_id: String,
    /// A discount on the components' total. Leave out to sell at their full total.
    #[serde(rename = "acom:bundle_discount", alias = "bundle_discount")]
    #[schemars(example = &serde_json::json!({"percent_off": 10}))]
    pub bundle_discount: Option<BundleDiscountInput>,
}

/// One component as the bundle's checks and numbers see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Component {
    pub variant_id: String,
    pub name: String,
    pub product_id: String,
    pub quantity: i64,
    pub price: Money,
    pub cost: Option<Money>,
    pub inventory_level: i64,
    pub managed: bool,
    /// Whether it can be sold: its product is active and its offer is not discontinued.
    pub on_sale: bool,
    pub image: Option<String>,
}

/// The bundle's numbers, from its components.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Derived {
    /// The components' list prices × units.
    pub list_total: Money,
    pub discount: Money,
    pub price: Money,
    /// `None` when a component has no cost.
    pub cost: Option<Money>,
    /// Whole sets the tracked components' stock makes; `None` when no component is tracked.
    pub inventory_level: Option<i64>,
}

/// Price, cost and stock of a bundle of `components` at `percent_off` (0–100).
pub(crate) fn derive(
    components: &[Component],
    percent_off: Decimal,
    currency: Currency,
) -> Derived {
    let mut list_total = Money::zero(currency);
    let mut cost = Some(Money::zero(currency));
    let mut stock: Option<i64> = None;
    for c in components {
        let q = Decimal::from(c.quantity);
        list_total = Money::new(list_total.value + c.price.value * q, currency);
        cost = match (cost, c.cost) {
            (Some(sum), Some(each)) => Some(Money::new(sum.value + each.value * q, currency)),
            _ => None,
        };
        if c.managed {
            let sets = c.inventory_level.max(0) / c.quantity.max(1);
            stock = Some(stock.map_or(sets, |s| s.min(sets)));
        }
    }
    let list_total = list_total.rounded();
    let discount = list_total
        .times(percent_off / Decimal::ONE_HUNDRED)
        .rounded();
    Derived {
        price: Money::new(list_total.value - discount.value, currency),
        list_total,
        discount,
        cost: cost.map(Money::rounded),
        inventory_level: stock,
    }
}

/// `bundle.components_must_be_active`.
pub(crate) fn active_check(rules: &BundleRules, components: &[Component]) -> Check {
    let off: Vec<&str> = components
        .iter()
        .filter(|c| !c.on_sale)
        .map(|c| c.name.as_str())
        .collect();
    let failed = rules.components_must_be_active && !off.is_empty();
    Check {
        rule: "bundle.components_must_be_active".into(),
        passed: !failed,
        detail: Some(if off.is_empty() {
            format!("All {} components are on sale", components.len())
        } else {
            format!("Not on sale: {}", off.join(", "))
        }),
        fix_tool: failed.then(|| "get_product".into()),
    }
}

/// `price_floor` on the bundle's price against its cost.
pub(crate) fn floor_check(rule: &PriceFloorRule, d: &Derived) -> Check {
    match d.cost {
        None => Check {
            rule: "price_floor".into(),
            passed: true,
            detail: Some("A component has no cost, so there is no floor to check".into()),
            fix_tool: None,
        },
        Some(cost) => {
            let ok = rule.allows(d.price, cost);
            Check {
                rule: "price_floor".into(),
                passed: ok,
                detail: Some(format!(
                    "{} against a floor of {} ({} of {})",
                    money_text(&d.price),
                    money_text(&rule.floor(cost)),
                    rule.markup_text(),
                    money_text(&cost)
                )),
                fix_tool: (!ok).then(|| "set_price".into()),
            }
        }
    }
}

/// What `create_bundle` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateBundlePlan {
    pub product_id: String,
    pub variant_id: String,
    pub offer_id: String,
    pub sku: String,
    pub name: String,
    pub category_id: String,
    pub components: Vec<Component>,
    pub derived: Derived,
    pub release_date: NaiveDate,
    pub floor_ok: bool,
    pub all_on_sale: bool,
}

/// Make a bundle of variants.
#[derive(Debug)]
pub struct CreateBundle;

impl Operation for CreateBundle {
    const META: OpMeta = OpMeta {
        address: "catalog.bundle.create",
        tool: "create_bundle",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/bundles",
        summary: "Make a bundle: a new product sold as a set of variants, priced at their total less an optional bundle discount, with stock from theirs. A person approves it before it goes on sale.",
        consumes: &[Slot::many("variant_id")],
        produces: &[Slot::one("product_id")],
        checks: &[
            "bundle.same_seller",
            "bundle.same_age_restriction",
            "price_floor",
            "bundle.components_must_be_active",
        ],
        errors: &[
            MIXED_SELLER,
            MIXED_AGE_RESTRICTION,
            BELOW_PRICE_FLOOR,
            COMPONENT_INACTIVE,
            MISSING_REQUIRED_FIELD,
            INVALID_CATEGORY,
            PRODUCT_NOT_FOUND,
            DUPLICATE_SKU,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateBundleInput;
    type Output = Value;
}

fn input_error(field: &str, message: String, reason: &str) -> ToolError {
    ToolError::invalid_input(message)
        .detail(json!({"field": field, "reason": reason}))
        .next("create_bundle")
}

/// A free SKU for the bundle, from its product id: `prd_denim_bundle` → `denim-bundle`.
async fn free_sku(tx: &mut TxCx, product_id: &str) -> Result<String, ToolError> {
    let base = product_id.trim_start_matches("prd_").replace('_', "-");
    let mut candidates = vec![base.clone()];
    candidates.extend((2..50).map(|n| format!("{base}-{n}")));
    let taken = write::taken_skus(tx, &candidates).await?;
    candidates
        .into_iter()
        .find(|s| !taken.contains_key(s))
        .ok_or_else(|| {
            ToolError::new(DUPLICATE_SKU, format!("No free SKU for {product_id}."))
                .detail(json!({"entity": "variant", "field": "sku"}))
                .next("get_product")
        })
}

impl WriteOp<Cx> for CreateBundle {
    type Plan = CreateBundlePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateBundleInput,
    ) -> Result<Planned<CreateBundlePlan>, ToolError> {
        crate::operators_only(cx, "make bundles")?;
        let rules = BundleRules::from_policy(&cx.policy)?;
        let floor = PriceFloorRule::from_policy(&cx.policy)?;
        let name = input.name.trim().to_owned();
        if name.is_empty() {
            return Err(
                ToolError::new(MISSING_REQUIRED_FIELD, "A bundle needs a name.")
                    .detail(json!({"field": "name", "reason": "missing"}))
                    .next("create_bundle"),
            );
        }
        let mut seen = BTreeSet::new();
        if let Some(twice) = input
            .is_related_to
            .iter()
            .find(|c| !seen.insert(c.variant_id.clone()))
        {
            return Err(input_error(
                "is_related_to",
                format!(
                    "{} is listed twice; give its units in order_quantity.",
                    twice.variant_id
                ),
                "duplicate",
            ));
        }
        if input.is_related_to.len() < rules.min_components {
            return Err(
                input_error(
                    "is_related_to",
                    format!(
                        "A bundle has at least {} different variants.",
                        rules.min_components
                    ),
                    "range",
                )
                .detail(json!({"field": "is_related_to", "reason": "range",
                           "policy_key": "bundle.min_components", "limit": rules.min_components})),
            );
        }
        let percent = input
            .bundle_discount
            .as_ref()
            .map_or(Decimal::ZERO, |d| d.percent_off);
        if percent.is_sign_negative() || percent > rules.max_discount * Decimal::ONE_HUNDRED {
            return Err(input_error(
                "acom:bundle_discount",
                format!(
                    "The bundle discount is 0–{}%.",
                    (rules.max_discount * Decimal::ONE_HUNDRED).normalize()
                ),
                "range",
            )
            .detail(json!({"field": "acom:bundle_discount", "reason": "range",
                           "policy_key": "bundle.max_discount"})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let term = standard_category(&mut tx, &input.category_id).await?;
        let mut components = Vec::new();
        let mut basis = Vec::new();
        for c in &input.is_related_to {
            let quantity = i64::from(c.order_quantity.unwrap_or(1).max(1));
            let (Some(offer), Some(variant)) = (
                repo::offer_state(&mut tx, &c.variant_id).await?,
                repo::variant(&mut tx, &c.variant_id).await?,
            ) else {
                return Err(product_not_found("variant", &c.variant_id, "get_product"));
            };
            let group = repo::group(&mut tx, &offer.product_group_id)
                .await?
                .ok_or_else(|| {
                    product_not_found("product", &offer.product_group_id, "get_product")
                })?;
            let price = money_of(offer.price, &offer.price_currency).ok_or_else(|| {
                ToolError::new(
                    ErrorCode::INTERNAL,
                    format!("{} has no valid price.", offer.offer_id),
                )
            })?;
            if price.currency != cx.currency {
                return Err(input_error(
                    "is_related_to",
                    format!(
                        "{} is priced in {}, not {}.",
                        variant.name, price.currency, cx.currency
                    ),
                    "enum",
                ));
            }
            basis.push(Basis {
                entity: "catalog.offer".into(),
                id: offer.offer_id.clone(),
                version: i64::from(offer.version),
            });
            basis.push(Basis {
                entity: "catalog.product_group".into(),
                id: group.id.clone(),
                version: i64::from(group.version),
            });
            components.push(Component {
                variant_id: variant.id.clone(),
                name: variant.name.clone(),
                product_id: group.id.clone(),
                quantity,
                price,
                cost: offer
                    .cost_value
                    .zip(offer.cost_currency.as_deref())
                    .and_then(|(v, c)| money_of(v, c)),
                inventory_level: i64::from(offer.inventory_level),
                managed: offer.is_managed,
                on_sale: group.approval_status == "active" && offer.availability != "discontinued",
                image: group.image.clone(),
            });
        }
        let product_id = free_product_id(&mut tx, &name).await?;
        let sku = free_sku(&mut tx, &product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let derived = derive(&components, percent, cx.currency);
        let suffix = product_id.trim_start_matches("prd_").to_owned();
        let floor_check = floor_check(&floor, &derived);
        let active = active_check(&rules, &components);
        let checks = vec![
            Check {
                rule: "bundle.same_seller".into(),
                passed: true,
                detail: Some("Every component is sold by the store itself".into()),
                fix_tool: None,
            },
            Check {
                rule: "bundle.same_age_restriction".into(),
                passed: true,
                detail: Some("No component has a minimum age".into()),
                fix_tool: None,
            },
            floor_check.clone(),
            active.clone(),
        ];
        let release_date = cx.clock.now().with_timezone(&cx.time.zone).date_naive();
        let stock = derived.inventory_level.unwrap_or(0);
        let availability = if derived.inventory_level.is_none() || stock > 0 {
            "in_stock"
        } else {
            "out_of_stock"
        };
        let change = Change {
            op: ChangeOp::Create,
            entity: "product".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({
                "product_id": product_id, "variant_id": format!("var_{suffix}"), "sku": sku,
                "name": name, "category_id": term.id, "approval_status": "active",
                "is_related_to": components.iter().map(|c| json!({
                    "variant_id": c.variant_id, "name": c.name, "order_quantity": c.quantity,
                    "price": num(c.price.value)})).collect::<Vec<_>>(),
                "list_total": num(derived.list_total.value),
                "acom:bundle_discount": {"percent_off": num(percent), "amount": num(derived.discount.value)},
                "price": num(derived.price.value), "price_currency": cx.currency.to_string(),
                "cost": derived.cost.map(|c| num(c.value)),
                "price_floor": derived.cost.map(|c| num(floor.floor(c).value)),
                "inventory_level": stock, "availability": availability,
            }),
            note: Some(format!(
                "On sale once approved; stock is {} whole set{} from the components",
                stock,
                if stock == 1 { "" } else { "s" }
            )),
        };
        let a = cx.policy.approval_for(
            Self::META.tool,
            &json!({"change_type": "structural", "new_product": true}),
        );
        let approval = if a.needed {
            ApprovalNeed::Needed {
                reason: format!(
                    "Puts a new bundle on sale at {}",
                    money_text(&derived.price)
                ),
            }
        } else {
            ApprovalNeed::NotNeeded
        };
        Ok(Planned {
            changes: vec![change],
            checks,
            basis,
            approval,
            plan: CreateBundlePlan {
                variant_id: format!("var_{suffix}"),
                offer_id: format!("off_{suffix}"),
                product_id,
                sku,
                name,
                category_id: term.id,
                components,
                derived,
                release_date,
                floor_ok: floor_check.passed,
                all_on_sale: active.passed,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &CreateBundlePlan) -> Result<Committed, ToolError> {
        if !plan.floor_ok {
            return Err(ToolError::new(
                BELOW_PRICE_FLOOR,
                format!(
                    "At {} the bundle would sell under its price floor.",
                    money_text(&plan.derived.price)
                ),
            )
            .detail(json!({"rule": "price_floor", "items": [plan.product_id]}))
            .next("set_price"));
        }
        if !plan.all_on_sale {
            return Err(ToolError::new(
                COMPONENT_INACTIVE,
                "A component of the bundle is not on sale.",
            )
            .detail(json!({"rule": "bundle.components_must_be_active"}))
            .next("get_product"));
        }
        if repo::group(tx, &plan.product_id).await?.is_some() {
            // The same plan run again.
            return Ok(committed(plan));
        }
        let image = plan.components.iter().find_map(|c| c.image.clone());
        let description = format!(
            "{} in one set.",
            plan.components
                .iter()
                .map(|c| format!("{} × {}", c.quantity, c.name))
                .collect::<Vec<_>>()
                .join(", ")
        );
        write::insert_group(
            tx,
            &NewGroup {
                id: &plan.product_id,
                name: &plan.name,
                description: Some(&description),
                brand_id: None,
                varies_by: &[],
                material: None,
                image: image.as_deref(),
            },
        )
        .await?;
        write::set_approval_status(
            tx,
            &plan.product_id,
            "draft",
            "active",
            Some(Some(plan.release_date)),
        )
        .await?;
        neighbors::place_product(tx, &plan.product_id, &plan.category_id).await?;
        let managed = plan.derived.inventory_level.is_some();
        write::insert_variant(
            tx,
            &NewVariant {
                id: &plan.variant_id,
                product_group_id: &plan.product_id,
                sku: &plan.sku,
                name: &plan.name,
                color: None,
                size: None,
                image: image.as_deref(),
                sale_type: "always",
                is_managed: managed,
            },
        )
        .await?;
        let level = plan.derived.inventory_level.unwrap_or(0);
        let level = i32::try_from(level).unwrap_or(i32::MAX);
        write::insert_offer(
            tx,
            &NewOffer {
                id: &plan.offer_id,
                variant_id: &plan.variant_id,
                price: plan.derived.price.value,
                price_currency: plan.derived.price.currency.as_str(),
                availability: if !managed || level > 0 {
                    "in_stock"
                } else {
                    "out_of_stock"
                },
                inventory_level: level,
                cost: plan
                    .derived
                    .cost
                    .as_ref()
                    .map(|c| (c.value, c.currency.as_str())),
            },
        )
        .await?;
        let parts: Vec<(String, i32)> = plan
            .components
            .iter()
            .map(|c| (c.variant_id.clone(), i32::try_from(c.quantity).unwrap_or(1)))
            .collect();
        super::insert_components(tx, &plan.product_id, &parts).await?;
        Ok(committed(plan))
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CreateBundlePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = repo::group(&mut tx, &plan.product_id).await?;
        let offer = repo::offer_state(&mut tx, &plan.variant_id).await?;
        let parts = super::components(&mut tx, &plan.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        if group.is_none_or(|g| g.approval_status != "active") {
            mismatches.push(format!("{} is not on sale", plan.product_id));
        }
        if offer.is_none_or(|o| o.price != plan.derived.price.value) {
            mismatches.push(format!(
                "{} is not priced {}",
                plan.variant_id,
                money_text(&plan.derived.price)
            ));
        }
        if parts.len() != plan.components.len() {
            mismatches.push(format!(
                "{} has {} components instead of {}",
                plan.product_id,
                parts.len(),
                plan.components.len()
            ));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &CreateBundlePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        // It may have been sold since: take it off sale rather than delete it.
        write::set_approval_status(tx, &plan.product_id, "active", "discontinued", None).await?;
        repo::discontinue_offer(tx, &plan.offer_id).await?;
        Ok(())
    }
}

fn committed(plan: &CreateBundlePlan) -> Committed {
    let mut produced = Map::new();
    produced.insert("product_id".into(), json!(plan.product_id));
    produced.insert("variant_id".into(), json!(plan.variant_id));
    Committed {
        id: plan.product_id.clone(),
        produced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;
    use std::str::FromStr;

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap_or_default(), Currency::USD)
    }

    fn part(id: &str, qty: i64, price: &str, cost: Option<&str>, stock: i64) -> Component {
        Component {
            variant_id: id.into(),
            name: id.into(),
            product_id: "prd_x".into(),
            quantity: qty,
            price: usd(price),
            cost: cost.map(usd),
            inventory_level: stock,
            managed: true,
            on_sale: true,
            image: None,
        }
    }

    #[test]
    fn price_cost_and_stock_come_from_the_components() {
        // A denim jacket S ($118, cost $48, 9 left) and two merino tees ($58, cost $22, 45 left).
        let parts = [
            part("var_denim_jacket_s_indigo", 1, "118.00", Some("48.00"), 9),
            part("var_merino_tee_m", 2, "58.00", Some("22.00"), 45),
        ];
        let d = derive(&parts, Decimal::from(10), Currency::USD);
        assert_eq!(d.list_total, usd("234.00"));
        assert_eq!(d.discount, usd("23.40"));
        assert_eq!(d.price, usd("210.60"));
        assert_eq!(d.cost, Some(usd("92.00")));
        assert_eq!(d.inventory_level, Some(9));
        let rule = PriceFloorRule::from_policy(&crate::testing::policies())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(floor_check(&rule, &d).passed);
        // 60% off would sell under cost + 15%.
        let deep = derive(&parts, Decimal::from(60), Currency::USD);
        let c = floor_check(&rule, &deep);
        assert!(!c.passed);
        assert_eq!(c.fix_tool.as_deref(), Some("set_price"));
        // A component without a cost leaves no floor; an untracked one does not limit stock.
        let mut loose = parts.clone();
        loose[1].cost = None;
        loose[0].managed = false;
        let d = derive(&loose, Decimal::ZERO, Currency::USD);
        assert_eq!((d.cost, d.inventory_level), (None, Some(22)));
    }

    #[test]
    fn components_must_be_on_sale() {
        let rules =
            BundleRules::from_policy(&crate::testing::policies()).unwrap_or_else(|e| panic!("{e}"));
        let mut parts = vec![
            part("var_a", 1, "10.00", None, 1),
            part("var_b", 1, "10.00", None, 1),
        ];
        assert!(active_check(&rules, &parts).passed);
        parts[1].on_sale = false;
        let c = active_check(&rules, &parts);
        assert!(!c.passed);
        assert_eq!(c.detail.as_deref(), Some("Not on sale: var_b"));
    }

    #[tokio::test]
    async fn inputs_are_checked_before_anything_is_read() {
        let op = testing::cx(acom_core::enums::ActorKind::Operator, "adm_sj");
        let input = |parts: Value, discount: Value| -> CreateBundleInput {
            serde_json::from_value(json!({"name": "Denim bundle", "is_related_to": parts,
                                          "category_id": "cat_outerwear",
                                          "acom:bundle_discount": discount}))
            .unwrap_or_else(|e| panic!("{e}"))
        };
        let one = json!([{"variant_id": "var_a"}]);
        let two = json!([{"variant_id": "var_a"}, {"variant_id": "var_b"}]);
        let twice = json!([{"variant_id": "var_a"}, {"variant_id": "var_a"}]);
        let ten = json!({"percent_off": 10});
        for (parts, discount) in [
            (one, ten.clone()),
            (twice, ten),
            (two, json!({"percent_off": 45})),
        ] {
            let e = CreateBundle
                .preview(&op, input(parts, discount))
                .await
                .unwrap_err();
            assert_eq!(e.code, "invalid_input", "{e}");
        }
        let e = CreateBundle
            .preview(
                &testing::shopper(),
                input(json!([]), json!({"percent_off": 0})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let schema = acom_contract::schema::closed_schema_for::<CreateBundleInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }

    #[test]
    fn a_bundle_waits_for_a_person() {
        let p = crate::testing::policies();
        let a = p.approval_for(
            CreateBundle::META.tool,
            &json!({"change_type": "structural", "new_product": true}),
        );
        assert!(a.needed);
    }
}
