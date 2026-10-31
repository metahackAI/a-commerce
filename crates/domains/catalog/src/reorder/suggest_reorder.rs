//! `suggest_reorder` (docs/api/catalog.md#reorder): how many of each variant of a product to
//! reorder, from the units each sold lately.
//!
//! A variant should hold enough for the days until a reorder placed today arrives
//! (`reorder.lead_time_days`) and the days it should last after that (`reorder.cover_days`), at
//! the rate it sold over the stock-out watcher's window (`platform.yml
//! watchers.stock_out_forecast.lookback_days`, so the numbers agree with the task that asked),
//! plus its safety stock. The suggestion is what is missing from that after its stock and what
//! is already on order. The tool only reads; `create_reorder` drafts the order.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::Cx;
use rust_decimal::{Decimal, RoundingStrategy};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{self, num};
use crate::rules::ReorderRules;
use crate::{PRODUCT_NOT_FOUND, neighbors, product_not_found, repo, visible_group};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestReorderInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_linen_shirt")]
    pub product_id: String,
}

/// How many of each variant to reorder.
#[derive(Debug)]
pub struct SuggestReorder;

impl Operation for SuggestReorder {
    const META: OpMeta = OpMeta {
        address: "catalog.reorder.suggest",
        tool: "suggest_reorder",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/products/{product_id}/reorder-suggestion",
        summary: "Which sizes of a product will run out and how many of each to reorder: units each variant sold lately, days until it runs out, and an order quantity that lasts through the supplier's lead time and the store's cover days. Its `lines` go to `create_reorder` as they are.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("variant_id")],
        checks: &[],
        errors: &[PRODUCT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SuggestReorderInput;
    type Output = Value;
}

/// What one variant needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Need {
    /// Stock ÷ daily sales, to one decimal; `None` when it sold nothing.
    pub days_until_stockout: Option<Decimal>,
    pub order_quantity: i64,
}

/// The order quantity of one variant: units for `lead_time_days + cover_days` at the rate of
/// `sold` over `days`, rounded up, plus `safety`, less `stock` and `on_order`; never below 0 nor
/// over the line limit, and 0 for a variant that cannot be ordered.
pub(crate) fn need(
    stock: i64,
    safety: i64,
    on_order: i64,
    sold: i64,
    days: u32,
    rules: &ReorderRules,
    orderable: bool,
) -> Need {
    let days = i64::from(days.max(1));
    let days_until_stockout = (sold > 0).then(|| {
        (Decimal::from(stock) * Decimal::from(days) / Decimal::from(sold))
            .round_dp_with_strategy(1, RoundingStrategy::MidpointAwayFromZero)
    });
    let horizon = i64::from(rules.lead_time_days) + i64::from(rules.cover_days);
    let selling = (sold * horizon + days - 1) / days;
    let missing = selling + safety - stock - on_order;
    Need {
        days_until_stockout,
        order_quantity: if orderable {
            missing.clamp(0, rules.max_units_per_line)
        } else {
            0
        },
    }
}

impl ReadOp<Cx> for SuggestReorder {
    async fn read(&self, cx: &Cx, input: SuggestReorderInput) -> Result<Value, ToolError> {
        let rules = ReorderRules::from_policy(&cx.policy)?;
        let days = cx
            .policy
            .watchers()
            .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?
            .stock_out_forecast
            .lookback_days;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = visible_group(cx, &mut tx, &input.product_id)
            .await?
            .ok_or_else(|| product_not_found("product", &input.product_id, "search_products"))?;
        let mut variants = repo::variants(&mut tx, std::slice::from_ref(&group.id)).await?;
        variants.retain(|v| v.offer_id.is_some());
        model::order_variants(&mut variants);
        let ids: Vec<String> = variants.iter().map(|v| v.id.clone()).collect();
        let ordered = crate::reorder::on_order(cx, &mut tx, &group.id).await?;
        let sold = neighbors::units_sold(&mut tx, &ids, days).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let mut rows = Vec::new();
        let mut lines = Vec::new();
        for v in &variants {
            let stock = v.stock();
            let safety = i64::from(v.safety_stock.unwrap_or(0));
            let on_order = ordered.by_variant.get(&v.id).copied().unwrap_or(0);
            let units = sold
                .as_ref()
                .and_then(|s| s.by_variant.get(&v.id).copied())
                .unwrap_or(0);
            let orderable = v.is_managed
                && rules
                    .orderable_availability
                    .iter()
                    .any(|a| Some(a.as_str()) == v.availability.as_deref());
            let n = need(stock, safety, on_order, units, days, &rules, orderable);
            if n.order_quantity > 0 {
                lines.push(json!({"variant_id": v.id, "order_quantity": n.order_quantity}));
            }
            rows.push(json!({"variant_id": v.id, "size": v.size, "color": v.color,
                "inventory_level": stock, "acom:safety_stock": safety, "on_order": on_order,
                "units_sold": units,
                "days_until_stockout": n.days_until_stockout.map_or(Value::Null, num),
                "order_quantity": n.order_quantity}));
        }
        let total: i64 = lines
            .iter()
            .filter_map(|l| l["order_quantity"].as_i64())
            .sum();
        let window = sold.map_or(
            Value::Null,
            |s| json!({"valid_from": s.valid_from, "valid_through": s.valid_through, "days": days}),
        );
        Ok(json!({"product_id": input.product_id, "name": group.name,
            "sales_window": window, "lead_time_days": rules.lead_time_days,
            "cover_days": rules.cover_days, "variants": rows, "lines": lines,
            "total_units": total}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn rules() -> ReorderRules {
        ReorderRules {
            lead_time_days: 7,
            cover_days: 14,
            max_units_per_line: 500,
            orderable_availability: vec!["in_stock".into(), "out_of_stock".into()],
        }
    }

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<SuggestReorderInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["product_id"]));
    }

    #[test]
    fn a_variant_is_filled_for_the_lead_time_and_cover_days_at_its_sales_rate() {
        // 10 sold in 7 days: 21 days need 30, plus 2 safety, less 4 on hand.
        let n = need(4, 2, 0, 10, 7, &rules(), true);
        assert_eq!(n.order_quantity, 28);
        assert_eq!(n.days_until_stockout, Some(Decimal::new(28, 1)));
        // What is already on order counts.
        assert_eq!(need(4, 2, 20, 10, 7, &rules(), true).order_quantity, 8);
        // Enough on hand: nothing.
        assert_eq!(need(40, 2, 0, 10, 7, &rules(), true).order_quantity, 0);
    }

    #[test]
    fn a_variant_that_sold_nothing_gets_its_safety_stock_and_no_forecast() {
        let n = need(0, 2, 0, 0, 7, &rules(), true);
        assert_eq!((n.order_quantity, n.days_until_stockout), (2, None));
        assert_eq!(need(5, 2, 0, 0, 7, &rules(), true).order_quantity, 0);
    }

    #[test]
    fn the_line_limit_and_unorderable_variants_cap_the_suggestion() {
        assert_eq!(need(0, 0, 0, 700, 7, &rules(), true).order_quantity, 500);
        assert_eq!(need(0, 2, 0, 10, 7, &rules(), false).order_quantity, 0);
    }

    #[test]
    fn reads_the_cover_days() {
        let r = ReorderRules::from_policy(&crate::testing::policies()).unwrap();
        assert_eq!((r.lead_time_days, r.cover_days), (7, 14));
    }
}
