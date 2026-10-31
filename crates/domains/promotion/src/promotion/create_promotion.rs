//! `create_promotion` (docs/api/promotion.md#promotion): promotions other than tier percent sale
//! prices — buy n get m, bundles, a benefit above an amount, a fixed amount off. Always needs a
//! person's approval.

use std::collections::{BTreeMap, BTreeSet};

use acom_catalog::service::for_promotion::{ProductOffers, products_with_offers};
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorClass,
    ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::{DiscountLayer, PromotionStatus, PromotionType, SalesChannel};
use acom_core::ids::PromotionId;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{layer_overlap, period_words};
use crate::model::{AppliesTo, OtherPromotion, SaleRules, and_list, money_text, percent_text};
use crate::offer::{BELOW_PRICE_FLOOR, others};
use crate::repo::{self, NewTypedPromotion};
use crate::rules;
use crate::service::for_merchandising::category_products;
use crate::{
    INVALID_PERIOD, TIER_NOT_FOUND, audience, check_id, known_tier, money, not_found, parse_time,
};

pub const STACKING_CONFLICT: ErrorCode =
    ErrorCode::new("stacking_conflict", ErrorClass::PolicyViolation);
pub const COST_SHARE_SUM: ErrorCode = ErrorCode::new("cost_share_sum", ErrorClass::PolicyViolation);
pub const PERCENT_OFF_FOR_TIER: ErrorCode =
    ErrorCode::new("percent_off_for_tier", ErrorClass::Domain);

/// What the promotion covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromotionScope {
    /// The products in `product_ids`.
    Products,
    /// Every product of `category_ids` (subcategories included).
    Categories,
    /// The whole order.
    Order,
}

impl PromotionScope {
    fn word(self) -> &'static str {
        match self {
            Self::Products => "products",
            Self::Categories => "categories",
            Self::Order => "order",
        }
    }
}

/// What a `threshold` promotion gives once the order reaches the threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdBenefit {
    /// `amount_off` comes off the order.
    AmountOff,
    /// Standard shipping is free (discount layer `add_on`).
    FreeShipping,
}

/// Who pays for the discount, as rates 0–1 that add up to the policy's `cost_share.sum_must_equal`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostShare {
    /// The store's share, 0–1.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", range(min = 0, max = 1), example = 1.0)]
    pub store: Decimal,
    /// The seller's share, 0–1.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", range(min = 0, max = 1), example = 0.0)]
    pub seller: Decimal,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePromotionInput {
    /// The label of the discount line in the cart.
    #[schemars(length(min = 1, max = 100), example = &"$10 off $120")]
    pub name: String,
    /// bogo: buy `buy_quantity`, get `get_quantity` free; bundle: the products together for
    /// `bundle_price`; threshold: a benefit once the subtotal reaches `threshold`; amount_off: a
    /// fixed `amount_off`.
    #[schemars(example = &"threshold")]
    pub promotion_type: PromotionType,
    /// item_price: on item prices; order_discount: off the order; add_on: shipping (a
    /// free-shipping threshold).
    #[schemars(example = &"order_discount")]
    pub discount_layer: DiscountLayer,
    /// The amount taken off; required for `amount_off`, and for `threshold` with
    /// `threshold_benefit: amount_off`.
    #[schemars(example = json!({"value": 10.00, "currency": "USD"}))]
    pub amount_off: Option<Money>,
    /// The subtotal before member benefits an order must reach; required for `threshold`.
    #[schemars(example = json!({"value": 120.00, "currency": "USD"}))]
    pub threshold: Option<Money>,
    /// What a `threshold` promotion gives. amount_off: `amount_off` off the order;
    /// free_shipping: free standard shipping.
    #[schemars(example = &"amount_off")]
    pub threshold_benefit: Option<ThresholdBenefit>,
    /// For `bogo`: how many to buy.
    #[schemars(range(min = 1, max = 100), example = 2)]
    pub buy_quantity: Option<u32>,
    /// For `bogo`: how many of them are free.
    #[schemars(range(min = 1, max = 100), example = 1)]
    pub get_quantity: Option<u32>,
    /// For `bundle`: the price of one of each product in `product_ids` together.
    #[schemars(example = json!({"value": 150.00, "currency": "USD"}))]
    pub bundle_price: Option<Money>,
    /// Start (inclusive). Without an offset it is read in store time.
    #[schemars(example = &"2026-11-20T00:00")]
    pub valid_from: String,
    /// End (inclusive, minute precision), after `valid_from`.
    #[schemars(example = &"2026-11-30T23:59")]
    pub valid_through: String,
    /// A tier id (`list_tiers`) or `everyone` (default).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"everyone")]
    pub valid_for_member_tier: Option<String>,
    /// What it covers: `products` needs `product_ids`, `categories` needs `category_ids`,
    /// `order` is the whole order.
    #[schemars(example = &"order")]
    pub scope_mode: PromotionScope,
    /// The products (`search_products`) when `scope_mode` is `products`; at least two for a
    /// bundle.
    #[schemars(
        length(min = 1, max = 500),
        inner(regex(pattern = r"^prd_[a-z0-9_]+$")),
        example = ["prd_denim_jacket", "prd_denim_trousers"]
    )]
    pub product_ids: Option<Vec<String>>,
    /// The categories (`list_categories`) when `scope_mode` is `categories`.
    #[schemars(
        length(min = 1, max = 50),
        inner(regex(pattern = r"^cat_[a-z0-9_]+$")),
        example = ["cat_denim"]
    )]
    pub category_ids: Option<Vec<String>>,
    /// Products left out of the scope.
    #[schemars(
        length(max = 500),
        inner(regex(pattern = r"^prd_[a-z0-9_]+$")),
        example = ["prd_selvedge_jeans"]
    )]
    pub exclude_product_ids: Option<Vec<String>>,
    /// Who pays: `{store, seller}` rates that add up to 1. Default: the store pays all.
    #[schemars(example = json!({"store": 1.0, "seller": 0.0}))]
    pub cost_share: Option<CostShare>,
    /// Where it applies: web, mobile_web, app, agent. Default: every channel.
    #[schemars(example = ["web", "app"])]
    pub channels: Option<Vec<SalesChannel>>,
    /// Not taken here: a percent off for a tier or a period is a sale price, made with
    /// `set_sale_prices`. Sending it returns `percent_off_for_tier`.
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    #[schemars(with = "Option<f64>", example = 20)]
    pub percent_off: Option<Decimal>,
}

/// What a run of `create_promotion` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromotionRun {
    pub promotion_id: String,
    pub name: String,
    pub description: String,
    pub promotion_type: String,
    pub discount_layer: String,
    pub valid_for_member_tier: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
    pub amount_off: Option<Money>,
    pub min_subtotal: Option<Money>,
    pub applies_to: Value,
    pub terms: Value,
    pub channels: Option<Vec<String>>,
}

pub struct CreatePromotion;

impl Operation for CreatePromotion {
    const META: OpMeta = OpMeta {
        address: "promotion.promotion.create",
        tool: "create_promotion",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/promotion/promotions",
        summary: "Create a promotion that is not a tier percent sale price: buy n get m free, a bundle price, a benefit above an order amount, or a fixed amount off. Tier or period percent discounts use set_sale_prices. Checks the discount limit, the price floor, overlaps, the combined discount and the cost share. Always needs a person's approval.",
        consumes: &[
            Slot::many("product_id"),
            Slot::one("category_id"),
            Slot::one("tier_id"),
        ],
        produces: &[Slot::one("promotion_id")],
        checks: &[
            "max_discount",
            "price_floor",
            "promotion_overlap",
            "discount_layers.max_total_discount_rate",
            "cost_share.sum_must_equal",
        ],
        errors: &[
            BELOW_PRICE_FLOOR,
            STACKING_CONFLICT,
            COST_SHARE_SUM,
            PERCENT_OFF_FOR_TIER,
            TIER_NOT_FOUND,
            INVALID_PERIOD,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreatePromotionInput;
    type Output = Value;
}

fn field_error(field: &str, reason: &str, message: impl Into<String>) -> ToolError {
    ToolError::invalid_input(message).detail(json!({"field": field, "reason": reason}))
}

/// An amount input in the store's currency, above zero.
fn amount_input(
    cx: &Cx,
    field: &str,
    m: Option<Money>,
    needed_for: &str,
) -> Result<Money, ToolError> {
    let m =
        m.ok_or_else(|| field_error(field, "missing", format!("`{needed_for}` needs `{field}`.")))?;
    if m.currency != cx.currency {
        return Err(field_error(
            field,
            "format",
            format!(
                "`{field}` must be in {}, the store's currency.",
                cx.currency
            ),
        ));
    }
    if m.value <= Decimal::ZERO || m.value.normalize().scale() > m.currency.minor_units() {
        return Err(field_error(
            field,
            "range",
            format!("`{field}` must be above zero, to the cent."),
        ));
    }
    Ok(m.rounded())
}

/// How much a promotion can take off, as a rate of what it applies to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Depth {
    /// 0–1, or `None` when there is nothing to measure it against.
    pub rate: Option<Decimal>,
    /// What the rate is measured on, in words.
    pub basis: String,
}

/// The deepest discount of the promotion: amount ÷ threshold, free ÷ bought, the bundle saving,
/// or a fixed amount on the cheapest product in scope.
pub(crate) fn depth(
    run: &PromotionRun,
    cheapest: Option<Money>,
    bundle_list: Option<Money>,
) -> Depth {
    let none = |basis: &str| Depth {
        rate: None,
        basis: basis.to_owned(),
    };
    let terms = &run.terms;
    match run.promotion_type.as_str() {
        "threshold" if terms["threshold_benefit"] == "free_shipping" => {
            none("Free shipping is not a price discount")
        }
        "threshold" => match (run.amount_off, run.min_subtotal) {
            (Some(a), Some(t)) if t.value > Decimal::ZERO => Depth {
                rate: Some(a.value / t.value),
                basis: format!("on a {} order", money_text(t.value, t.currency.as_str())),
            },
            _ => none("No threshold to measure against"),
        },
        "bogo" => {
            let buy = terms["buy_quantity"].as_u64().unwrap_or(0);
            let get = terms["get_quantity"].as_u64().unwrap_or(0);
            if buy + get == 0 {
                return none("No quantities");
            }
            Depth {
                rate: Some(Decimal::from(get) / Decimal::from(buy + get)),
                basis: format!("{get} free of {}", buy + get),
            }
        }
        "bundle" => {
            let price = terms
                .get("bundle_price")
                .and_then(|p| serde_json::from_value::<Money>(p.clone()).ok());
            match (price, bundle_list) {
                (Some(p), Some(l)) if l.value > Decimal::ZERO && p.value < l.value => Depth {
                    rate: Some((l.value - p.value) / l.value),
                    basis: format!(
                        "{} for products listed at {}",
                        money_text(p.value, p.currency.as_str()),
                        money_text(l.value, l.currency.as_str())
                    ),
                },
                (Some(_), Some(_)) => Depth {
                    rate: Some(Decimal::ZERO),
                    basis: "The bundle price is not under the list prices".into(),
                },
                _ => none("No list prices to compare the bundle with"),
            }
        }
        _ => match (run.amount_off, cheapest) {
            (Some(a), Some(c)) if c.value > Decimal::ZERO => Depth {
                rate: Some((a.value / c.value).min(Decimal::ONE)),
                basis: format!(
                    "on the cheapest product in scope ({})",
                    money_text(c.value, c.currency.as_str())
                ),
            },
            _ => none("A fixed amount off the order"),
        },
    }
}

fn pct(rate: Decimal) -> String {
    percent_text((rate * Decimal::ONE_HUNDRED).round_dp(1))
}

/// `max_discount` for a promotion of a type.
pub(crate) fn check_depth(rules: &SaleRules, tier: Option<&str>, d: &Depth) -> Check {
    let who = audience(tier);
    let cap = rules.max_discount(tier);
    let (passed, detail) = match (d.rate, cap) {
        (_, None) => (
            false,
            format!("The policy sets no discount limit for {who}"),
        ),
        (None, Some(cap)) => (
            true,
            format!(
                "{}; the {}% limit for {who} is applied at checkout",
                d.basis,
                pct(cap)
            ),
        ),
        (Some(r), Some(cap)) if r <= cap => (
            true,
            format!("At most {}% {}; limit {}%", pct(r), d.basis, pct(cap)),
        ),
        // Name the tiers whose limit would allow it, so the agent can offer that instead.
        (Some(r), Some(cap)) => (
            false,
            rules.over_limit_words((r * Decimal::ONE_HUNDRED).round_dp(1), tier, cap),
        ),
    };
    Check {
        rule: "max_discount".into(),
        passed,
        detail: Some(detail),
        fix_tool: (!passed).then(|| "create_promotion".into()),
    }
}

/// `price_floor` for the products in scope: each variant's price after the deepest discount
/// stays at or above cost × the floor multiplier.
pub(crate) fn check_floor(
    rules: &SaleRules,
    run: &PromotionRun,
    products: &[ProductOffers],
    d: &Depth,
) -> Check {
    let words = rules.floor_words();
    if run.applies_to["scope"] == "order" {
        return Check {
            rule: "price_floor".into(),
            passed: true,
            detail: Some(format!(
                "Order-wide; calculate_benefits keeps every line at {words}"
            )),
            fix_tool: None,
        };
    }
    let mut under = Vec::new();
    for p in products {
        for o in &p.offers {
            let Some(cost) = o.cost_value else { continue };
            let rate = match (run.promotion_type.as_str(), d.rate, run.amount_off) {
                ("amount_off", _, Some(a)) if o.price > Decimal::ZERO => {
                    (a.value / o.price).min(Decimal::ONE)
                }
                (_, Some(r), _) => r,
                _ => Decimal::ZERO,
            };
            let price = rules.sale_price(o.price, rate * Decimal::ONE_HUNDRED);
            let floor = rules.floor(cost);
            if !rules.above_floor(price, floor) {
                under.push(format!(
                    "{} would be {}, under the floor {}",
                    p.name,
                    money_text(price, &o.price_currency),
                    money_text(floor, &o.price_currency)
                ));
                break;
            }
        }
    }
    let passed = under.is_empty();
    Check {
        rule: "price_floor".into(),
        passed,
        detail: Some(if passed {
            format!("No line goes under {words}")
        } else {
            under.join("; ")
        }),
        fix_tool: (!passed).then(|| "create_promotion".into()),
    }
}

/// `discount_layers.max_total_discount_rate`: this promotion and the deepest overlapping
/// discount of each other layer stay under the policy's combined limit.
pub(crate) fn check_combined(
    limit: Decimal,
    ours: Option<Decimal>,
    others: &[(String, Decimal)],
) -> Check {
    let total = ours.unwrap_or_default() + others.iter().map(|(_, r)| *r).sum::<Decimal>();
    let passed = total <= limit;
    let with = if others.is_empty() {
        String::new()
    } else {
        format!(
            " with {}",
            and_list(&others.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>())
        )
    };
    Check {
        rule: "discount_layers.max_total_discount_rate".into(),
        passed,
        detail: Some(if passed {
            format!(
                "Combined discount{with} stays under {}% (at most {}%)",
                pct(limit),
                pct(total)
            )
        } else {
            format!(
                "Combined discount{with} could reach {}%, over the {}% limit",
                pct(total),
                pct(limit)
            )
        }),
        fix_tool: (!passed).then(|| "create_promotion".into()),
    }
}

/// The depth of another promotion (its percent, or amount ÷ minimum), for the combined check.
fn other_rate(r: &repo::PromotionRow, min: Option<Decimal>) -> Option<Decimal> {
    if let Some(p) = r.percent_off {
        return Some(p / Decimal::ONE_HUNDRED);
    }
    match (r.amount_off_value, min) {
        (Some(a), Some(m)) if m > Decimal::ZERO => Some((a / m).min(Decimal::ONE)),
        _ => None,
    }
}

impl WriteOp<Cx> for CreatePromotion {
    type Plan = PromotionRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreatePromotionInput,
    ) -> Result<Planned<PromotionRun>, ToolError> {
        if input.percent_off.is_some() {
            return Err(ToolError::new(
                PERCENT_OFF_FOR_TIER,
                "A percent off for a tier or a period is a sale price; make it with set_sale_prices.",
            )
            .detail(json!({"field": "percent_off"}))
            .next("set_sale_prices"));
        }
        let rules = SaleRules::from_policy(&cx.policy)?;
        let name = input.name.trim().to_owned();
        if name.is_empty() {
            return Err(field_error("name", "missing", "`name` must not be empty."));
        }
        let from = parse_time(&cx.time, "valid_from", &input.valid_from)?;
        let through = parse_time(&cx.time, "valid_through", &input.valid_through)?;
        if from >= through {
            return Err(ToolError::new(
                INVALID_PERIOD,
                "`valid_from` must come before `valid_through`.",
            )
            .detail(json!({"field": "valid_from", "reason": "range"}))
            .next("create_promotion"));
        }
        let kind = input.promotion_type;
        let mut terms = serde_json::Map::new();
        let (mut amount_off, mut min_subtotal) = (None, None);
        match kind {
            PromotionType::AmountOff => {
                amount_off = Some(amount_input(
                    cx,
                    "amount_off",
                    input.amount_off,
                    "amount_off",
                )?);
            }
            PromotionType::Threshold => {
                let t = amount_input(cx, "threshold", input.threshold, "threshold")?;
                min_subtotal = Some(t);
                let benefit = input.threshold_benefit.ok_or_else(|| {
                    field_error(
                        "threshold_benefit",
                        "missing",
                        "`threshold` needs `threshold_benefit`.",
                    )
                })?;
                terms.insert("threshold_benefit".into(), json!(benefit));
                if benefit == ThresholdBenefit::AmountOff {
                    amount_off = Some(amount_input(
                        cx,
                        "amount_off",
                        input.amount_off,
                        "threshold_benefit: amount_off",
                    )?);
                }
                let shipping = benefit == ThresholdBenefit::FreeShipping;
                if shipping != (input.discount_layer == DiscountLayer::AddOn) {
                    return Err(field_error(
                        "discount_layer",
                        "enum",
                        "Free shipping is the `add_on` layer, and `add_on` is only free shipping.",
                    ));
                }
            }
            PromotionType::Bogo => {
                let buy = input.buy_quantity.filter(|n| *n >= 1).ok_or_else(|| {
                    field_error(
                        "buy_quantity",
                        "missing",
                        "`bogo` needs `buy_quantity` of 1 or more.",
                    )
                })?;
                let get = input.get_quantity.filter(|n| *n >= 1).ok_or_else(|| {
                    field_error(
                        "get_quantity",
                        "missing",
                        "`bogo` needs `get_quantity` of 1 or more.",
                    )
                })?;
                terms.insert("buy_quantity".into(), json!(buy));
                terms.insert("get_quantity".into(), json!(get));
            }
            PromotionType::Bundle => {
                let price = amount_input(cx, "bundle_price", input.bundle_price, "bundle")?;
                terms.insert("bundle_price".into(), json!(price));
            }
        }
        if kind != PromotionType::Threshold && input.discount_layer == DiscountLayer::AddOn {
            return Err(field_error(
                "discount_layer",
                "enum",
                "`add_on` is only a free-shipping threshold; use item_price or order_discount.",
            ));
        }
        let share = input.cost_share.unwrap_or(CostShare {
            store: Decimal::ONE,
            seller: Decimal::ZERO,
        });
        let must = rules::number(&cx.policy, "promotion.cost_share.sum_must_equal")?;
        if share.store < Decimal::ZERO
            || share.seller < Decimal::ZERO
            || share.store + share.seller != must
        {
            return Err(ToolError::new(
                COST_SHARE_SUM,
                format!(
                    "Store {} and seller {} add up to {}; they must add up to {must}.",
                    share.store,
                    share.seller,
                    share.store + share.seller
                ),
            )
            .detail(json!({"rule": "cost_share.sum_must_equal", "items": []}))
            .next("create_promotion"));
        }
        terms.insert("cost_share".into(), json!(share));
        let channels: Option<Vec<String>> = input
            .channels
            .map(|l| {
                let mut w: Vec<String> = l.iter().map(|c| c.as_str().to_owned()).collect();
                w.sort();
                w.dedup();
                w
            })
            .filter(|l| !l.is_empty());
        let excluded: Vec<String> = input.exclude_product_ids.unwrap_or_default();
        for id in &excluded {
            check_id("exclude_product_ids", "prd", id, false)?;
        }

        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let tier = known_tier(
            &mut tx,
            input.valid_for_member_tier.as_deref().unwrap_or("everyone"),
        )
        .await?;
        let tier_id = tier.as_ref().map(|t| t.id.clone());
        let (product_ids, category_ids, scope_words) = match input.scope_mode {
            PromotionScope::Products => {
                let ids = input.product_ids.unwrap_or_default();
                if ids.is_empty() {
                    return Err(field_error(
                        "product_ids",
                        "missing",
                        "`scope_mode: products` needs `product_ids`.",
                    ));
                }
                let mut unique: Vec<String> = Vec::new();
                for id in ids {
                    check_id("product_ids", "prd", &id, false)?;
                    if !unique.contains(&id) {
                        unique.push(id);
                    }
                }
                let n = unique.len();
                (unique, Vec::new(), format!("these {n} products"))
            }
            PromotionScope::Categories => {
                let ids = input.category_ids.unwrap_or_default();
                if ids.is_empty() {
                    return Err(field_error(
                        "category_ids",
                        "missing",
                        "`scope_mode: categories` needs `category_ids`.",
                    ));
                }
                for id in &ids {
                    check_id("category_ids", "cat", id, false)?;
                }
                let found = category_products(&mut tx, &ids).await?;
                if let Some(missing) = ids
                    .iter()
                    .find(|c| !found.iter().any(|f| &f.category_id == *c))
                {
                    return Err(not_found("category", missing, "search_products"));
                }
                let names: Vec<String> = found.iter().map(|c| c.name.to_lowercase()).collect();
                let products: BTreeSet<String> =
                    found.into_iter().flat_map(|c| c.product_ids).collect();
                (products.into_iter().collect(), ids, and_list(&names))
            }
            PromotionScope::Order => (Vec::new(), Vec::new(), "the whole order".to_owned()),
        };
        if kind == PromotionType::Bundle
            && (input.scope_mode != PromotionScope::Products || product_ids.len() < 2)
        {
            return Err(field_error(
                "product_ids",
                "range",
                "A bundle is one of each of two or more products: use `scope_mode: products`.",
            ));
        }
        let in_scope: Vec<String> = product_ids
            .iter()
            .filter(|p| !excluded.contains(p))
            .cloned()
            .collect();
        let products = products_with_offers(&mut tx, &in_scope).await?;
        if input.scope_mode == PromotionScope::Products
            && let Some(missing) = in_scope
                .iter()
                .find(|id| !products.iter().any(|p| &p.product_id == *id))
        {
            return Err(not_found("product", missing, "search_products"));
        }
        let (others, mut basis) = others(&mut tx, &rules, from, through, now).await?;
        let other_rows: Vec<repo::PromotionRow> = {
            let mut rows = Vec::new();
            for o in &others {
                if let Some(r) = repo::promotion(&mut tx, &o.id).await? {
                    rows.push(r);
                }
            }
            rows
        };
        let mut minimums = BTreeMap::new();
        for r in &other_rows {
            if let Some(x) = repo::promotion_extras(&mut tx, &r.id).await?
                && let Some(v) = x.eligible_transaction_volume_value
            {
                minimums.insert(r.id.clone(), v);
            }
        }
        tx.commit().await.map_err(ToolError::from)?;

        let applies_to = AppliesTo {
            scope: input.scope_mode.word().to_owned(),
            product_ids: if input.scope_mode == PromotionScope::Products {
                product_ids.clone()
            } else {
                Vec::new()
            },
            category_ids: category_ids.clone(),
            exclude_product_ids: excluded.clone(),
        };
        let layer = input.discount_layer;
        let run = PromotionRun {
            promotion_id: PromotionId::generate().to_string(),
            description: String::new(),
            name,
            promotion_type: kind.as_str().to_owned(),
            discount_layer: layer.as_str().to_owned(),
            valid_for_member_tier: tier_id.clone(),
            valid_from: from,
            valid_through: through,
            amount_off,
            min_subtotal,
            applies_to: serde_json::to_value(&applies_to).unwrap_or_default(),
            terms: Value::Object(terms),
            channels,
        };
        let lowest = |p: &ProductOffers| p.offers.iter().map(|o| o.price).min();
        let cheapest = products
            .iter()
            .filter_map(lowest)
            .min()
            .map(|v| Money::new(v, cx.currency));
        let bundle_list = (kind == PromotionType::Bundle)
            .then(|| Money::new(products.iter().filter_map(lowest).sum(), cx.currency));
        let d = depth(&run, cheapest, bundle_list);
        let ours: Option<BTreeSet<String>> =
            (input.scope_mode != PromotionScope::Order).then(|| in_scope.iter().cloned().collect());
        let overlap = layer_overlap(
            &rules,
            layer,
            tier_id.as_deref(),
            ours.as_ref(),
            &others,
            &scope_words,
            from,
            through,
            &cx.time,
        );
        let combined: Vec<(String, Decimal)> = others
            .iter()
            .filter(|o| o.layer != layer && reaches(o, tier_id.as_deref(), ours.as_ref()))
            .filter_map(|o| {
                let row = other_rows.iter().find(|r| r.id == o.id)?;
                other_rate(row, minimums.get(&o.id).copied()).map(|r| (o.name.clone(), r))
            })
            .collect();
        let limit = rules::number(
            &cx.policy,
            "promotion.discount_layers.max_total_discount_rate",
        )?;
        let share_detail = if share.seller.is_zero() {
            "Store 100%".to_owned()
        } else {
            format!("Store {}%, seller {}%", pct(share.store), pct(share.seller))
        };
        let checks = vec![
            check_depth(&rules, tier_id.as_deref(), &d),
            check_floor(&rules, &run, &products, &d),
            overlap,
            check_combined(limit, d.rate, &combined),
            Check {
                rule: "cost_share.sum_must_equal".into(),
                passed: true,
                detail: Some(share_detail),
                fix_tool: None,
            },
        ];
        let mut run = run;
        run.description = format!(
            "{} for {}, {}.",
            layer.as_str().replace('_', " "),
            audience(tier_id.as_deref()),
            scope_words
        );
        let mut to = json!({
            "name": run.name,
            "promotion_type": run.promotion_type,
            "discount_layer": run.discount_layer,
            "valid_for_member_tier": audience(tier_id.as_deref()),
            "valid_from": cx.time.format(from),
            "valid_through": cx.time.format(through),
            "applies_to": {"scope": applies_to.scope, "category_ids": category_ids,
                           "product_count": (input.scope_mode != PromotionScope::Order).then_some(in_scope.len())},
        });
        if let Some(a) = amount_off {
            to["amount_off"] = money(a.value, a.currency.as_str());
        }
        if let Some(t) = min_subtotal {
            to["threshold"] = money(t.value, t.currency.as_str());
        }
        for k in [
            "threshold_benefit",
            "buy_quantity",
            "get_quantity",
            "bundle_price",
        ] {
            if let Some(v) = run.terms.get(k) {
                to[k] = v.clone();
            }
        }
        basis.extend(products.iter().flat_map(|p| {
            p.offers.iter().map(|o| Basis {
                entity: "catalog.offer".into(),
                id: o.offer_id.clone(),
                version: i64::from(o.version),
            })
        }));
        let reason = format!(
            "New {} for {}, {}",
            layer.as_str().replace('_', " "),
            audience(tier_id.as_deref()),
            period_words(&cx.time, from, Some(through))
        );
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "promotion".into(),
                id: String::new(),
                from: Value::Null,
                to,
                note: None,
            }],
            checks,
            basis,
            approval: ApprovalNeed::Needed { reason },
            plan: run,
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &PromotionRun) -> Result<Committed, ToolError> {
        let status = if tx.clock.now() < run.valid_from {
            PromotionStatus::Scheduled
        } else {
            PromotionStatus::Running
        };
        let change_set_id = tx.change_set_id.clone();
        repo::insert_typed_promotion(
            tx,
            &NewTypedPromotion {
                id: &run.promotion_id,
                name: &run.name,
                description: &run.description,
                status: status.as_str(),
                promotion_type: &run.promotion_type,
                discount_layer: &run.discount_layer,
                tier: run.valid_for_member_tier.as_deref(),
                valid_from: run.valid_from,
                valid_through: run.valid_through,
                amount_off: run
                    .amount_off
                    .as_ref()
                    .map(|m| (m.value, m.currency.as_str())),
                min_subtotal: run
                    .min_subtotal
                    .as_ref()
                    .map(|m| (m.value, m.currency.as_str())),
                applies_to: &run.applies_to,
                terms: &run.terms,
                channels: run.channels.as_deref(),
                change_set_id: change_set_id.as_deref(),
            },
        )
        .await?;
        Ok(Committed {
            id: run.promotion_id.clone(),
            produced: serde_json::Map::from_iter([(
                "promotion_id".to_owned(),
                json!(run.promotion_id),
            )]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &PromotionRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::promotion(&mut tx, &run.promotion_id).await?;
        let extras = repo::promotion_extras(&mut tx, &run.promotion_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(compare(run, row.as_ref(), extras.as_ref()))
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        run: &PromotionRun,
        _: &Committed,
    ) -> Result<(), ToolError> {
        // The row stays as a record of what ran; ended, it applies to nothing.
        if let Some(p) = repo::promotion(tx, &run.promotion_id).await? {
            repo::set_status(
                tx,
                &p.id,
                &p.promotion_status,
                PromotionStatus::Ended.as_str(),
            )
            .await?;
        }
        Ok(())
    }
}

/// Whether another promotion reaches the same shoppers and products (`None` = every product).
fn reaches(o: &OtherPromotion, tier: Option<&str>, ours: Option<&BTreeSet<String>>) -> bool {
    let audiences = tier.is_none() || o.tier.is_none() || o.tier.as_deref() == tier;
    let products = match (ours, &o.products) {
        (None, _) | (_, None) => true,
        (Some(a), Some(b)) => a.intersection(b).next().is_some(),
    };
    audiences && products
}

/// Read-back check of a created promotion.
pub(crate) fn compare(
    run: &PromotionRun,
    row: Option<&repo::PromotionRow>,
    extras: Option<&repo::PromotionExtras>,
) -> Verification {
    let mut mismatches = Vec::new();
    match (row, extras) {
        (Some(r), Some(x)) => {
            if r.name != run.name
                || r.discount_layer != run.discount_layer
                || r.valid_for_member_tier != run.valid_for_member_tier
                || r.valid_from != run.valid_from
                || r.valid_through != Some(run.valid_through)
                || r.amount_off_value != run.amount_off.map(|m| m.value)
                || x.eligible_transaction_volume_value != run.min_subtotal.map(|m| m.value)
                || x.promotion_type.as_deref() != Some(run.promotion_type.as_str())
                || r.applies_to != run.applies_to
                || x.promotion_terms != run.terms
            {
                mismatches.push(format!(
                    "promotion {} differs from the plan",
                    run.promotion_id
                ));
            }
        }
        _ => mismatches.push(format!("promotion {} is missing", run.promotion_id)),
    }
    Verification {
        matched: mismatches.is_empty(),
        mismatches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{d, policies};
    use acom_contract::schema::{closed_schema_for, violations};
    use acom_core::Currency;

    fn usd(v: &str) -> Money {
        Money::new(d(v), Currency::USD)
    }

    fn run(kind: &str, terms: Value) -> PromotionRun {
        PromotionRun {
            promotion_id: "promo_x".into(),
            name: "$10 off $120".into(),
            description: String::new(),
            promotion_type: kind.into(),
            discount_layer: "order_discount".into(),
            valid_for_member_tier: None,
            valid_from: chrono::DateTime::UNIX_EPOCH,
            valid_through: chrono::DateTime::UNIX_EPOCH,
            amount_off: Some(usd("10.00")),
            min_subtotal: Some(usd("120.00")),
            applies_to: json!({"scope": "order"}),
            terms,
            channels: None,
        }
    }

    #[test]
    fn the_spec_example_is_at_most_8_3_percent() {
        let rules = SaleRules::from_policy(&policies(false)).unwrap();
        let r = run("threshold", json!({"threshold_benefit": "amount_off"}));
        let dp = depth(&r, None, None);
        let c = check_depth(&rules, None, &dp);
        assert!(c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("At most 8.3% on a $120.00 order; limit 20%")
        );
        let floor = check_floor(&rules, &r, &[], &dp);
        assert!(floor.passed);
        let combined = check_combined(d("0.5"), dp.rate, &[]);
        assert!(combined.passed);
    }

    #[test]
    fn buy_two_get_one_is_a_third_off_and_too_deep_for_everyone() {
        let rules = SaleRules::from_policy(&policies(false)).unwrap();
        let r = run("bogo", json!({"buy_quantity": 2, "get_quantity": 1}));
        let c = check_depth(&rules, None, &depth(&r, None, None));
        assert!(!c.passed, "{c:?}");
        assert_eq!(c.fix_tool.as_deref(), Some("create_promotion"));
        assert_eq!(
            c.detail.as_deref(),
            Some("33.3% is over the 20% limit for everyone")
        );
        let demo = SaleRules::from_policy(&policies(true)).unwrap();
        let c = check_depth(&demo, None, &depth(&r, None, None));
        assert_eq!(
            c.detail.as_deref(),
            Some("33.3% is over the 20% limit for everyone")
        );
        let half = run("bogo", json!({"buy_quantity": 3, "get_quantity": 1}));
        let c = check_depth(&demo, None, &depth(&half, None, None));
        assert_eq!(
            c.detail.as_deref(),
            Some("25% is over the 20% limit for everyone; vip allows up to 30%")
        );
        let free = run("threshold", json!({"threshold_benefit": "free_shipping"}));
        assert!(check_depth(&rules, None, &depth(&free, None, None)).passed);
        let combined = check_combined(
            d("0.5"),
            Some(d("0.3")),
            &[("Autumn knitwear 15%".into(), d("0.15"))],
        );
        assert!(combined.passed);
        assert!(!check_combined(d("0.5"), Some(d("0.4")), &[("x".into(), d("0.2"))]).passed);
    }

    #[test]
    fn input_schema_is_closed_and_takes_the_spec_example() {
        let s = closed_schema_for::<CreatePromotionInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        let i: CreatePromotionInput = serde_json::from_value(json!({
            "name": "$10 off $120", "promotion_type": "threshold", "discount_layer": "order_discount",
            "amount_off": {"value": 10.00, "currency": "USD"},
            "threshold": {"value": 120.00, "currency": "USD"}, "threshold_benefit": "amount_off",
            "valid_from": "2026-11-20T00:00", "valid_through": "2026-11-30T23:59",
            "valid_for_member_tier": "everyone", "scope_mode": "order",
            "cost_share": {"store": 1.0, "seller": 0.0}, "channels": ["web", "app"]}))
        .unwrap();
        assert_eq!(i.cost_share.unwrap().store, d("1.0"));
        assert!(
            serde_json::from_value::<CreatePromotionInput>(
                json!({"name": "x", "promotion_type": "gift"})
            )
            .is_err()
        );
        assert_eq!(CreatePromotion::META.kind, Kind::WriteApproval);
    }
}
