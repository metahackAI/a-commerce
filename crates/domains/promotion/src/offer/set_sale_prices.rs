//! `set_sale_prices` (docs/api/promotion.md#offer): tier and period sale prices for a set of
//! products, grouped as one item-price promotion. List prices never change.

use std::collections::{BTreeMap, BTreeSet};

use acom_catalog::service::for_promotion::{ProductOffers, products_with_offers};
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::{DiscountLayer, PromotionStatus};
use acom_core::ids::PromotionId;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    BELOW_PRICE_FLOOR, LOW_STOCK, OVER_MAX_DISCOUNT, PROMOTION_OVERLAP, field_error, others,
    valid_percent,
};
use crate::model::{
    AppliesTo, LeftOut, SaleAsk, SalePlan, SaleRules, check_max_discount, check_min_stock,
    check_overlap, check_price_floor, plan_sale,
};
use crate::repo::{self, NewPrice, NewPromotion};
use crate::service::for_merchandising::category_products;
use crate::{
    INVALID_PERIOD, TIER_NOT_FOUND, amount, audience, check_id, known_tier, parse_time, percent,
};

/// Which products the sale covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SaleScope {
    /// Every product of `category_id`, its subcategories included.
    Category,
    /// Only the products in `product_ids`.
    Products,
}

/// A product in scope with its own discount.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PercentOverride {
    /// A product inside the sale's scope.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_seersucker_overshirt")]
    pub product_id: String,
    /// Its discount in percent, above 0 and below 100, at most two decimals.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", range(min = 0.01, max = 99.99), example = 15)]
    pub percent_off: Decimal,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetSalePricesInput {
    /// Promotion name; also the discount line label in the cart.
    #[schemars(length(min = 1, max = 100), example = &"VIP weekend")]
    pub name: String,
    /// Who pays the sale price: a tier id (`list_tiers`) or `everyone`.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub valid_for_member_tier: String,
    /// Start (inclusive). Without an offset it is read in store time.
    #[schemars(example = &"2026-11-07T00:00")]
    pub valid_from: String,
    /// End (inclusive, minute precision), after `valid_from`.
    #[schemars(example = &"2026-11-08T23:59")]
    pub valid_through: String,
    /// Which products: `category` needs `category_id`, `products` needs `product_ids`.
    #[schemars(example = &"category")]
    pub scope_mode: SaleScope,
    /// The category (`list_categories`) when `scope_mode` is `category`.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub category_id: Option<String>,
    /// 1–500 products (`search_products`) when `scope_mode` is `products`.
    #[schemars(
        length(min = 1, max = 500),
        inner(regex(pattern = r"^prd_[a-z0-9_]+$")),
        example = ["prd_windbreaker", "prd_field_jacket"]
    )]
    pub product_ids: Option<Vec<String>>,
    /// Discount in percent for every product in scope, above 0 and below 100, at most two
    /// decimals. Sale price = list price × (1 − percent_off / 100), rounded to the cent.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", range(min = 0.01, max = 99.99), example = 20)]
    pub percent_off: Decimal,
    /// Products in scope that get a different discount than `percent_off`.
    #[schemars(example = serde_json::json!([{"product_id": "prd_seersucker_overshirt", "percent_off": 15}]))]
    pub overrides: Option<Vec<PercentOverride>>,
    /// Default false: products under the minimum stock (`promo_min_stock`) stay in and the
    /// `promo_min_stock` check fails so a person decides. true: they are left out and listed
    /// as `exclude` changes.
    #[schemars(example = true)]
    pub exclude_low_stock: Option<bool>,
}

/// One sale price to create.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedPrice {
    /// `ups_<offer>_<promotion>`.
    pub id: String,
    pub offer_id: String,
    pub product_id: String,
    pub price: Decimal,
    pub price_currency: String,
    pub percent_off: Decimal,
}

/// What a run of `set_sale_prices` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaleRun {
    pub promotion_id: String,
    pub name: String,
    pub description: String,
    /// `None` for everyone.
    pub valid_for_member_tier: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
    pub percent_off: Decimal,
    /// `promotion.applies_to`.
    pub applies_to: Value,
    pub prices: Vec<PlannedPrice>,
}

pub struct SetSalePrices;

impl Operation for SetSalePrices {
    const META: OpMeta = OpMeta {
        address: "promotion.offer.set_sale_prices",
        tool: "set_sale_prices",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/promotion/sale-prices",
        summary: "Set tier- and period-limited percent sale prices on products (a category or a list) as one promotion; list prices stay. Checks the discount limit, the price floor, minimum stock and overlapping promotions. Always needs a person's approval.",
        consumes: &[
            Slot::one("category_id"),
            Slot::many("product_id"),
            Slot::one("tier_id"),
        ],
        produces: &[Slot::one("promotion_id")],
        checks: &[
            "max_discount",
            "price_floor",
            "promo_min_stock",
            "promotion_overlap",
        ],
        errors: &[
            BELOW_PRICE_FLOOR,
            OVER_MAX_DISCOUNT,
            LOW_STOCK,
            PROMOTION_OVERLAP,
            TIER_NOT_FOUND,
            ErrorCode::NOT_FOUND,
            INVALID_PERIOD,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetSalePricesInput;
    type Output = Value;
}

/// The change rows: the promotion, one price row per product, one `exclude` row per product
/// left out.
pub(crate) fn sale_changes(
    time: &acom_core::StoreTime,
    run: &SaleRun,
    plan: &SalePlan,
    min_stock: i64,
) -> Vec<Change> {
    let mut changes = vec![Change {
        op: ChangeOp::Create,
        entity: "promotion".into(),
        id: String::new(),
        from: Value::Null,
        to: json!({
            "name": run.name,
            "valid_for_member_tier": audience(run.valid_for_member_tier.as_deref()),
            "discount_layer": DiscountLayer::ItemPrice,
            "valid_from": time.format(run.valid_from),
            "valid_through": time.format(run.valid_through),
            "products": plan.included.len(),
        }),
        note: None,
    }];
    for p in &plan.included {
        let Some(shown) = p.shown() else {
            continue;
        };
        changes.push(Change {
            op: ChangeOp::Update,
            entity: "unit_price_specification".into(),
            id: String::new(),
            from: json!({"product_id": p.product_id, "price": amount(shown.list_price),
                         "price_currency": shown.currency, "price_type": "list_price"}),
            to: json!({"product_id": p.product_id, "price": amount(shown.sale_price),
                       "price_currency": shown.currency, "price_type": "sale_price",
                       "percent_off": percent(p.percent_off)}),
            note: None,
        });
    }
    for e in &plan.excluded {
        let note = match e.why {
            LeftOut::LowStock => format!("{} has {} units, under {min_stock}", e.name, e.stock),
            LeftOut::NotSellable => format!("{} is not on sale in the catalog", e.name),
            LeftOut::NoOffer => format!("{} has no price to discount", e.name),
        };
        changes.push(Change {
            op: ChangeOp::Exclude,
            entity: "product".into(),
            id: e.product_id.clone(),
            from: Value::Null,
            to: json!({"why": e.why.word(), "inventory_level": e.stock}),
            note: Some(note),
        });
    }
    changes
}

impl WriteOp<Cx> for SetSalePrices {
    type Plan = SaleRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetSalePricesInput,
    ) -> Result<Planned<SaleRun>, ToolError> {
        let rules = SaleRules::from_policy(&cx.policy)?;
        let name = input.name.trim().to_owned();
        if name.is_empty() {
            return Err(field_error("name", "missing", "`name` must not be empty."));
        }
        if !input
            .valid_for_member_tier
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase())
        {
            return Err(field_error(
                "valid_for_member_tier",
                "format",
                "`valid_for_member_tier` is a tier id such as `vip`, or `everyone`.",
            ));
        }
        let base = valid_percent("percent_off", input.percent_off)?;
        let mut overrides = BTreeMap::new();
        for o in input.overrides.unwrap_or_default() {
            check_id("overrides.product_id", "prd", &o.product_id, false)?;
            let p = valid_percent("overrides.percent_off", o.percent_off)?;
            if overrides.insert(o.product_id.clone(), p).is_some() {
                return Err(field_error(
                    "overrides",
                    "format",
                    format!("{} is in `overrides` twice.", o.product_id),
                ));
            }
        }
        let from = parse_time(&cx.time, "valid_from", &input.valid_from)?;
        let through = parse_time(&cx.time, "valid_through", &input.valid_through)?;
        if from >= through {
            return Err(ToolError::new(
                INVALID_PERIOD,
                "`valid_from` must come before `valid_through`.",
            )
            .detail(json!({"field": "valid_from", "reason": "range"}))
            .next("set_sale_prices"));
        }
        let now = cx.clock.now();

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let tier = known_tier(&mut tx, &input.valid_for_member_tier).await?;
        let tier_id = tier.as_ref().map(|t| t.id.clone());
        let (product_ids, scope_words, category_ids) = match input.scope_mode {
            SaleScope::Category => {
                let id = input.category_id.ok_or_else(|| {
                    field_error(
                        "category_id",
                        "missing",
                        "`scope_mode: category` needs `category_id`.",
                    )
                })?;
                check_id("category_id", "cat", &id, false)?;
                let c = category_products(&mut tx, std::slice::from_ref(&id))
                    .await?
                    .into_iter()
                    .next()
                    .ok_or_else(|| crate::not_found("category", &id, "search_products"))?;
                (c.product_ids, c.name.to_lowercase(), vec![id])
            }
            SaleScope::Products => {
                let ids = input.product_ids.unwrap_or_default();
                if ids.is_empty() || ids.len() > 500 {
                    return Err(field_error(
                        "product_ids",
                        "range",
                        "`scope_mode: products` needs 1–500 `product_ids`.",
                    ));
                }
                let mut unique = Vec::new();
                for id in ids {
                    check_id("product_ids", "prd", &id, false)?;
                    if !unique.contains(&id) {
                        unique.push(id);
                    }
                }
                let words = format!("these {} products", unique.len());
                (unique, words, Vec::new())
            }
        };
        if let Some(stray) = overrides.keys().find(|k| !product_ids.contains(k)) {
            return Err(field_error(
                "overrides",
                "range",
                format!("{stray} in `overrides` is not in the sale's scope."),
            ));
        }
        let products: Vec<ProductOffers> = products_with_offers(&mut tx, &product_ids).await?;
        let missing: Vec<&String> = product_ids
            .iter()
            .filter(|id| !products.iter().any(|p| p.product_id == **id))
            .collect();
        if !missing.is_empty() {
            return Err(ToolError::new(
                ErrorCode::NOT_FOUND,
                format!("The store has no product {}.", missing[0]),
            )
            .detail(json!({"entity": "product", "id": missing[0], "ids": missing}))
            .next("search_products"));
        }
        let (others, mut basis) = others(&mut tx, &rules, from, through, now).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let ask = SaleAsk {
            tier: tier_id.clone(),
            percent_off: base,
            overrides,
            exclude_low_stock: input.exclude_low_stock.unwrap_or(false),
        };
        let plan = plan_sale(&rules, &ask, &products);
        let ours: BTreeSet<String> = plan.included.iter().map(|p| p.product_id.clone()).collect();
        let checks = vec![
            check_max_discount(&rules, &ask, &plan),
            check_price_floor(&rules, &ask, &plan),
            check_min_stock(&rules, &plan),
            check_overlap(&rules, tier_id.as_deref(), &ours, &others, &scope_words),
        ];

        let promotion_id = PromotionId::generate().to_string();
        let suffix = promotion_id.trim_start_matches("promo_").to_owned();
        let prices: Vec<PlannedPrice> = plan
            .included
            .iter()
            .flat_map(|p| {
                p.offers.iter().map(|o| PlannedPrice {
                    id: format!("ups_{}_{suffix}", o.offer_id.trim_start_matches("off_")),
                    offer_id: o.offer_id.clone(),
                    product_id: p.product_id.clone(),
                    price: o.sale_price,
                    price_currency: o.currency.clone(),
                    percent_off: p.percent_off,
                })
            })
            .collect();
        let scope = AppliesTo {
            scope: "products".into(),
            product_ids: ours.iter().cloned().collect(),
            category_ids,
            exclude_product_ids: plan.excluded.iter().map(|e| e.product_id.clone()).collect(),
        };
        let run = SaleRun {
            promotion_id,
            description: format!(
                "{}% off {} products for {}.",
                crate::model::percent_text(base),
                plan.included.len(),
                audience(tier_id.as_deref())
            ),
            name,
            valid_for_member_tier: tier_id,
            valid_from: from,
            valid_through: through,
            percent_off: base,
            applies_to: serde_json::to_value(&scope).unwrap_or_default(),
            prices,
        };
        let changes = sale_changes(&cx.time, &run, &plan, rules.promo_min_stock);
        for p in &products {
            if !ours.contains(&p.product_id) {
                continue;
            }
            basis.extend(p.offers.iter().map(|o| Basis {
                entity: "catalog.offer".into(),
                id: o.offer_id.clone(),
                version: i64::from(o.version),
            }));
        }
        Ok(Planned {
            approval: ApprovalNeed::Needed {
                reason: format!("Changes {} prices", plan.included.len()),
            },
            plan: run,
            changes,
            checks,
            basis,
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &SaleRun) -> Result<Committed, ToolError> {
        let status = if tx.clock.now() < run.valid_from {
            PromotionStatus::Scheduled
        } else {
            PromotionStatus::Running
        };
        repo::insert_promotion(
            tx,
            &NewPromotion {
                id: &run.promotion_id,
                name: &run.name,
                description: &run.description,
                status: status.as_str(),
                tier: run.valid_for_member_tier.as_deref(),
                valid_from: run.valid_from,
                valid_through: run.valid_through,
                percent_off: run.percent_off,
                applies_to: &run.applies_to,
            },
        )
        .await?;
        let prices: Vec<NewPrice> = run
            .prices
            .iter()
            .map(|p| NewPrice {
                id: p.id.clone(),
                offer_id: p.offer_id.clone(),
                price: p.price,
                price_currency: p.price_currency.clone(),
                percent_off: p.percent_off,
            })
            .collect();
        repo::insert_prices(
            tx,
            &run.promotion_id,
            run.valid_for_member_tier.as_deref(),
            run.valid_from,
            run.valid_through,
            &prices,
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
        run: &SaleRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let promotion = repo::promotion(&mut tx, &run.promotion_id).await?;
        let stored = repo::prices(&mut tx, &run.promotion_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(compare_sale(run, promotion.as_ref(), &stored))
    }

    async fn undo(&self, tx: &mut TxCx, run: &SaleRun, _: &Committed) -> Result<(), ToolError> {
        let ids: Vec<String> = run.prices.iter().map(|p| p.id.clone()).collect();
        repo::delete_prices(tx, &run.promotion_id, &ids).await?;
        // The promotion row stays as a record of what ran; ended, it applies to nothing.
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

/// Read-back check: the promotion and every planned price are stored as planned.
pub(crate) fn compare_sale(
    run: &SaleRun,
    promotion: Option<&repo::PromotionRow>,
    stored: &[repo::PriceRow],
) -> Verification {
    let mut mismatches = Vec::new();
    match promotion {
        None => mismatches.push(format!("promotion {} is missing", run.promotion_id)),
        Some(p) => {
            if p.name != run.name
                || p.valid_for_member_tier != run.valid_for_member_tier
                || p.valid_from != run.valid_from
                || p.valid_through != Some(run.valid_through)
            {
                mismatches.push(format!(
                    "promotion {} differs from the plan (name, tier or period)",
                    run.promotion_id
                ));
            }
        }
    }
    let by_id: BTreeMap<&str, &repo::PriceRow> =
        stored.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut missing = 0usize;
    for p in &run.prices {
        match by_id.get(p.id.as_str()) {
            None => missing += 1,
            Some(r) => {
                if r.offer_id != p.offer_id
                    || r.price != p.price
                    || r.percent_off != Some(p.percent_off)
                    || r.valid_for_member_tier != run.valid_for_member_tier
                    || r.valid_from != run.valid_from
                    || r.valid_through != Some(run.valid_through)
                {
                    mismatches.push(format!(
                        "{} on {} is {} {} instead of {} {}",
                        r.id, r.offer_id, r.price, r.price_currency, p.price, p.price_currency
                    ));
                }
            }
        }
    }
    if missing > 0 {
        mismatches.push(format!(
            "{missing} of {} sale prices are missing",
            run.prices.len()
        ));
    }
    Verification {
        matched: mismatches.is_empty(),
        mismatches,
    }
}
