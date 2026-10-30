//! `get_price_breakdown`, `set_price` (docs/api/catalog.md#offer).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, ReadOp, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_core::enums::PriceType;
use acom_core::{Currency, Money};
use acom_store::{Cx, TxCx};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::product_not_found;
use crate::model::{self, money_text, num};
use crate::neighbors;
use crate::repo::{self, OfferState};
use crate::rules::PriceFloorRule;
use crate::viewer::Viewer;
use crate::{BELOW_PRICE_FLOOR, PRODUCT_NOT_FOUND, TIER_NOT_FOUND, is_operator, operator_only};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPriceBreakdownInput {
    /// The variant.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_windbreaker_m_slate")]
    pub variant_id: String,
    /// Operators only. The member tier to price for (`list_tiers`), or `everyone`.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub valid_for_member_tier: Option<String>,
    /// Operators only. The moment to price at, in store time. Default now.
    #[schemars(example = &"2026-11-07T00:00:00-05:00")]
    pub as_of: Option<String>,
    /// Units, to see the line total too.
    #[schemars(range(min = 1), example = 2)]
    pub order_quantity: Option<u32>,
}

/// How a variant's price is made up for the caller.
#[derive(Debug)]
pub struct GetPriceBreakdown;

impl Operation for GetPriceBreakdown {
    const META: OpMeta = OpMeta {
        address: "catalog.offer.get_breakdown",
        tool: "get_price_breakdown",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/variants/{variant_id}/price-breakdown",
        summary: "The price of one variant taken apart: list price, the member or sale price in effect and the promotion it comes from. Operators also see cost and the price floor.",
        consumes: &[Slot::one("variant_id")],
        produces: &[],
        checks: &[],
        errors: &[PRODUCT_NOT_FOUND, TIER_NOT_FOUND, ErrorCode::FORBIDDEN],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPriceBreakdownInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetPriceBreakdown {
    async fn read(&self, cx: &Cx, input: GetPriceBreakdownInput) -> Result<Value, ToolError> {
        let tier = input.valid_for_member_tier.as_deref();
        let as_of = input.as_of.as_deref();
        Viewer::check_inputs(cx, tier, as_of)?;
        if input.order_quantity == Some(0) {
            return Err(
                ToolError::invalid_input("`order_quantity` must be at least 1.")
                    .detail(json!({"field": "order_quantity", "reason": "range"})),
            );
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let viewer = Viewer::resolve(cx, &mut tx, tier, as_of).await?;
        let missing = || product_not_found("variant", &input.variant_id, "search_products");
        let v = repo::variant(&mut tx, &input.variant_id)
            .await?
            .filter(|v| v.offer_id.is_some())
            .ok_or_else(missing)?;
        let group = super::product::visible_group(cx, &mut tx, &v.product_group_id)
            .await?
            .ok_or_else(missing)?;
        let offer = v.offer_id.clone().unwrap_or_default();
        let prices = neighbors::sale_prices(
            &mut tx,
            std::slice::from_ref(&offer),
            viewer.tier.as_deref(),
            viewer.at,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let list = v.list_price().ok_or_else(missing)?;
        let best = model::best_sale_price(list.value, &prices);
        let mut out = json!({
            "variant_id": v.id, "product_id": group.id,
            "list_price": num(list.value),
            "member_price": best.map_or(Value::Null, |p| num(p.price)),
            "price_currency": list.currency.to_string(),
            "applied": best.map(|p| vec![json!({
                "promotion_id": p.promotion_id, "name": p.name,
                "valid_for_member_tier": p.valid_for_member_tier.as_deref().unwrap_or("everyone"),
                "valid_through": p.valid_through.map(|t| cx.time.format(t))})])
                .unwrap_or_default(),
        });
        if viewer.operator {
            let rule = PriceFloorRule::from_policy(&cx.policy)?;
            out["cost"] = v.cost().map_or(Value::Null, |c| num(c.value));
            out["price_floor"] = v.cost().map_or(Value::Null, |c| num(rule.floor(c).value));
        }
        if let Some(q) = input.order_quantity {
            let unit = best.map_or(list.value, |p| p.price);
            let total = Money::new(unit, list.currency)
                .times(Decimal::from(q))
                .rounded();
            out["order_quantity"] = json!(q);
            out["total"] = json!(total);
        }
        Ok(out)
    }
}

/// A price with its currency, as `set_price` takes it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PriceInput {
    /// Amount in major units, for example 89.00.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", example = 89.00)]
    pub price: Decimal,
    /// ISO 4217 code of `price`.
    pub price_currency: Currency,
    /// Always `list_price` here when given.
    #[schemars(example = &"list_price")]
    pub price_type: Option<PriceType>,
}

/// A unit cost, as `set_price` takes it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostInput {
    /// What one unit costs the store, in major units, for example 36.00.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", example = 36.00)]
    pub price: Decimal,
    /// ISO 4217 code of the cost.
    pub price_currency: Currency,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPriceInput {
    /// The variant whose list price to set.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_windbreaker_m_slate")]
    pub variant_id: String,
    /// The new list price in major units, to the cent.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", example = 85.00)]
    pub price: Decimal,
    /// ISO 4217 code of `price`; must be the offer's currency.
    pub price_currency: Currency,
    /// The reference list price (`price_type: list_price`). The list price is `price`, so when
    /// given it must be the same amount.
    #[serde(rename = "acom:list_price", alias = "list_price")]
    pub list_price: Option<PriceInput>,
    /// Operators only. The new unit cost; the base of the price floor.
    #[serde(rename = "acom:cost", alias = "cost")]
    pub cost: Option<CostInput>,
}

/// What `set_price` runs: the offer, the new amounts, and the old ones for undo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetPricePlan {
    pub variant_id: String,
    pub offer_id: String,
    pub price: Money,
    /// `Some` when the cost changes too.
    pub cost: Option<Money>,
    pub from_price: Money,
    pub from_cost: Option<Money>,
    /// The floor check passed in the preview.
    pub floor_ok: bool,
}

/// Set a variant's list price (and cost).
#[derive(Debug)]
pub struct SetPrice;

impl Operation for SetPrice {
    const META: OpMeta = OpMeta {
        address: "catalog.offer.set_price",
        tool: "set_price",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Put,
        path: "/v1/catalog/variants/{variant_id}/price",
        summary: "Set a variant's list price and, optionally, its cost. The server refuses prices under the floor (cost × 1.15). Member and sale prices are set_sale_prices.",
        consumes: &[Slot::one("variant_id")],
        produces: &[Slot::one("offer_id")],
        checks: &["price_floor"],
        errors: &[BELOW_PRICE_FLOOR, PRODUCT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetPriceInput;
    type Output = Value;
}

fn amount_error(field: &str, message: String) -> ToolError {
    ToolError::invalid_input(message).detail(json!({"field": field, "reason": "range"}))
}

/// Everything `set_price` decides, from the input and the offer as it is now.
pub(crate) fn plan_price(
    rule: &PriceFloorRule,
    input: &SetPriceInput,
    s: &OfferState,
) -> Result<(SetPricePlan, Check), ToolError> {
    if input.price.is_sign_negative() {
        return Err(amount_error("price", "`price` cannot be negative.".into()));
    }
    let current = model::money_of(s.price, &s.price_currency)
        .ok_or_else(|| ToolError::new(ErrorCode::INTERNAL, "offer currency is not ISO 4217"))?;
    if input.price_currency != current.currency {
        return Err(ToolError::invalid_input(format!(
            "This offer is priced in {}, not {}.",
            current.currency, input.price_currency
        ))
        .detail(json!({"field": "price_currency", "reason": "enum",
                       "allowed": [current.currency.to_string()]})));
    }
    if let Some(l) = &input.list_price
        && (l.price != input.price
            || l.price_currency != input.price_currency
            || l.price_type.is_some_and(|t| t != PriceType::ListPrice))
    {
        return Err(ToolError::invalid_input(
            "`acom:list_price` must be the same list price as `price`.",
        )
        .detail(json!({"field": "acom:list_price", "reason": "format"})));
    }
    if let Some(c) = &input.cost
        && c.price.is_sign_negative()
    {
        return Err(amount_error(
            "acom:cost",
            "The cost cannot be negative.".into(),
        ));
    }
    let price = Money::new(input.price, input.price_currency).rounded();
    let from_cost = s
        .cost_value
        .zip(s.cost_currency.as_deref())
        .and_then(|(v, c)| model::money_of(v, c));
    let cost = input
        .cost
        .as_ref()
        .map(|c| Money::new(c.price, c.price_currency).rounded());
    let base = cost.or(from_cost);
    let check = match base {
        Some(c) => {
            let floor = rule.floor(c);
            let ok = rule.allows(price, c);
            let markup = rule.markup_text().replacen("cost", &money_text(&c), 1);
            Check {
                rule: "price_floor".into(),
                passed: ok,
                detail: Some(format!(
                    "{} is {} the floor {} ({markup})",
                    money_text(&price),
                    if ok { "at or above" } else { "below" },
                    money_text(&floor),
                )),
                fix_tool: (!ok).then(|| "set_price".into()),
            }
        }
        None => Check {
            rule: "price_floor".into(),
            passed: true,
            detail: Some("No cost on file, so there is no floor to check".into()),
            fix_tool: None,
        },
    };
    Ok((
        SetPricePlan {
            variant_id: s.variant_id.clone(),
            offer_id: s.offer_id.clone(),
            price,
            cost,
            from_price: current,
            from_cost,
            floor_ok: check.passed,
        },
        check,
    ))
}

impl WriteOp<Cx> for SetPrice {
    type Plan = SetPricePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetPriceInput,
    ) -> Result<Planned<SetPricePlan>, ToolError> {
        if input.cost.is_some() && !is_operator(&cx.principal) {
            return Err(operator_only("acom:cost"));
        }
        let rule = PriceFloorRule::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let state = repo::offer_state(&mut tx, &input.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let s = state
            .ok_or_else(|| product_not_found("variant", &input.variant_id, "search_products"))?;
        let (plan, check) = plan_price(&rule, &input, &s)?;
        let mut from = json!({"price": num(plan.from_price.value)});
        let mut to = json!({"price": num(plan.price.value)});
        if let Some(c) = plan.cost {
            from["cost"] = plan.from_cost.map_or(Value::Null, |f| num(f.value));
            to["cost"] = num(c.value);
        }
        let approval = cx.policy.approval_for(
            Self::META.tool,
            &json!({"changed_offers": 1, "change_type": "structural"}),
        );
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "offer".into(),
                id: plan.offer_id.clone(),
                from,
                to,
                note: None,
            }],
            checks: vec![check],
            basis: vec![Basis {
                entity: "catalog.offer".into(),
                id: s.offer_id.clone(),
                version: i64::from(s.version),
            }],
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Changes 1 list price".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SetPricePlan) -> Result<Committed, ToolError> {
        if !plan.floor_ok {
            return Err(ToolError::new(
                BELOW_PRICE_FLOOR,
                format!(
                    "At {} {} would sell below its price floor.",
                    money_text(&plan.price),
                    plan.variant_id
                ),
            )
            .detail(json!({"rule": "price_floor", "items": [plan.variant_id]}))
            .next("set_price"));
        }
        let cost = plan
            .cost
            .map(|c| (Some(c.value), Some(c.currency.to_string())));
        let n = repo::set_offer_price(
            tx,
            &plan.offer_id,
            plan.price.value,
            plan.price.currency.as_str(),
            cost,
        )
        .await?;
        if n == 0 {
            return Err(product_not_found(
                "variant",
                &plan.variant_id,
                "search_products",
            ));
        }
        let mut produced = Map::new();
        produced.insert("offer_id".into(), json!(plan.offer_id));
        produced.insert("variant_id".into(), json!(plan.variant_id));
        Ok(Committed {
            id: plan.offer_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SetPricePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let state = repo::offer_state(&mut tx, &plan.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match state {
            None => mismatches.push(format!("{} has no offer", plan.variant_id)),
            Some(s) => {
                if s.price != plan.price.value || s.price_currency != plan.price.currency.as_str() {
                    mismatches.push(format!(
                        "{} is {} {} instead of {}",
                        plan.offer_id,
                        s.price,
                        s.price_currency,
                        money_text(&plan.price)
                    ));
                }
                if let Some(c) = plan.cost
                    && s.cost_value != Some(c.value)
                {
                    mismatches.push(format!("{} cost is not {}", plan.offer_id, money_text(&c)));
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
        plan: &SetPricePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        let cost = plan.cost.map(|_| {
            (
                plan.from_cost.map(|c| c.value),
                plan.from_cost.map(|c| c.currency.to_string()),
            )
        });
        repo::set_offer_price(
            tx,
            &plan.offer_id,
            plan.from_price.value,
            plan.from_price.currency.as_str(),
            cost,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap_or_default()
    }

    /// The Seersucker overshirt M Clay offer on Friday: $79.00, cost $58.00.
    fn overshirt() -> OfferState {
        OfferState {
            variant_id: "var_seersucker_overshirt_m_clay".into(),
            product_group_id: "prd_seersucker_overshirt".into(),
            is_managed: true,
            variant_version: 1,
            offer_id: "off_seersucker_overshirt_m_clay".into(),
            price: d("79.00"),
            price_currency: "USD".into(),
            availability: "in_stock".into(),
            inventory_level: 5,
            cost_value: Some(d("58.00")),
            cost_currency: Some("USD".into()),
            sold_out_at: None,
            availability_starts: None,
            version: 3,
        }
    }

    fn input(price: &str) -> SetPriceInput {
        SetPriceInput {
            variant_id: "var_seersucker_overshirt_m_clay".into(),
            price: d(price),
            price_currency: Currency::USD,
            list_price: None,
            cost: None,
        }
    }

    #[test]
    fn floor_blocks_prices_under_cost_plus_fifteen_percent() {
        let rule = PriceFloorRule::from_policy(&crate::testing::policies())
            .unwrap_or_else(|e| panic!("{e}"));
        let (plan, check) =
            plan_price(&rule, &input("63.20"), &overshirt()).unwrap_or_else(|e| panic!("{e}"));
        assert!(!check.passed && !plan.floor_ok);
        assert_eq!(check.fix_tool.as_deref(), Some("set_price"));
        assert_eq!(
            check.detail.as_deref(),
            Some("$63.20 is below the floor $66.70 ($58.00 + 15%)")
        );
        let (plan, check) =
            plan_price(&rule, &input("66.70"), &overshirt()).unwrap_or_else(|e| panic!("{e}"));
        assert!(check.passed && plan.floor_ok);
        assert_eq!(plan.from_price.value, d("79.00"));
        // A new, lower cost moves the floor with it.
        let mut cheaper = input("60.00");
        cheaper.cost = Some(CostInput {
            price: d("50.00"),
            price_currency: Currency::USD,
        });
        let (plan, check) =
            plan_price(&rule, &cheaper, &overshirt()).unwrap_or_else(|e| panic!("{e}"));
        assert!(check.passed);
        assert_eq!(plan.cost.map(|c| c.value), Some(d("50.00")));
    }

    #[test]
    fn refuses_other_currencies_and_a_different_reference_price() {
        let rule = PriceFloorRule::from_policy(&crate::testing::policies())
            .unwrap_or_else(|e| panic!("{e}"));
        let mut eur = input("80.00");
        eur.price_currency = Currency::parse("EUR").unwrap_or(Currency::USD);
        assert_eq!(
            plan_price(&rule, &eur, &overshirt()).unwrap_err().code,
            "invalid_input"
        );
        let mut other = input("80.00");
        other.list_price = Some(PriceInput {
            price: d("90.00"),
            price_currency: Currency::USD,
            price_type: None,
        });
        assert_eq!(
            plan_price(&rule, &other, &overshirt()).unwrap_err().code,
            "invalid_input"
        );
    }

    #[test]
    fn one_price_change_needs_no_approval_but_ten_do() {
        let p = crate::testing::policies();
        assert!(
            !p.approval_for("set_price", &json!({"changed_offers": 1}))
                .needed
        );
        assert!(
            p.approval_for("set_price", &json!({"changed_offers": 10}))
                .needed
        );
    }

    #[test]
    fn reads_the_spec_input() {
        let i: SetPriceInput = serde_json::from_str(
            r#"{"variant_id": "var_windbreaker_m_slate", "price": 85.00, "price_currency": "USD",
                "acom:cost": {"price": 36.00, "price_currency": "USD"}}"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(i.price.to_string(), "85.00");
        assert!(
            serde_json::from_str::<SetPriceInput>(
                r#"{"variant_id": "var_x", "price": 1, "price_currency": "USD", "x": 1}"#
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn shoppers_cannot_set_costs_or_tiers() {
        let mut i = input("80.00");
        i.cost = Some(CostInput {
            price: d("1.00"),
            price_currency: Currency::USD,
        });
        let e = SetPrice.preview(&testing::shopper(), i).await.unwrap_err();
        assert_eq!(e.code, "forbidden");
        let e = GetPriceBreakdown
            .read(
                &testing::shopper(),
                GetPriceBreakdownInput {
                    variant_id: "var_windbreaker_m_slate".into(),
                    valid_for_member_tier: None,
                    as_of: Some("2026-11-07T00:00".into()),
                    order_quantity: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn windbreaker_breakdown_shows_cost_and_floor_to_operators() {
        let cx = testing::operator_db().await;
        let out = GetPriceBreakdown
            .read(
                &cx,
                GetPriceBreakdownInput {
                    variant_id: "var_windbreaker_m_slate".into(),
                    valid_for_member_tier: None,
                    as_of: None,
                    order_quantity: Some(2),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["list_price"].to_string(), "89.00");
        assert_eq!(out["price_floor"].to_string(), "41.40");
        assert_eq!(out["total"]["value"].to_string(), "178.00");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn set_price_previews_runs_verifies_and_undoes() {
        let cx = testing::operator_db().await;
        let planned = SetPrice
            .preview(&cx, input("80.00"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes[0].from["price"].to_string(), "79.00");
        assert!(matches!(planned.approval, ApprovalNeed::NotNeeded));
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = SetPrice
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        SetPrice
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        drop(tx);
    }
}
