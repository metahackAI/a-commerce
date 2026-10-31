//! The `inventory` resource (docs/api/catalog.md#inventory): sellable stock per variant
//! and the availability that follows from it.

pub mod get_inventory;
pub mod set_safety_stock;
pub mod set_variant_stock;

use acom_contract::{Check, ErrorCode, ToolError};
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{self, StockState};
use crate::repo::{self, OfferState, StockWrite};
use crate::rules::StockRules;
use crate::{NEGATIVE_STOCK, product_not_found};

/// Stock and the availability fields that move with it, before or after a change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockSnapshot {
    pub inventory_level: i32,
    pub availability: String,
    pub sold_out_at: Option<DateTime<Utc>>,
    pub availability_starts: Option<DateTime<Utc>>,
    pub managed: bool,
}

/// What `set_variant_stock` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockPlan {
    pub variant_id: String,
    pub offer_id: String,
    pub from: StockSnapshot,
    pub to: StockSnapshot,
}

/// The stock plan and the rule that moved availability, if any.
pub(crate) fn plan_stock(
    rules: &StockRules,
    s: &OfferState,
    level: i64,
    managed: Option<bool>,
    now: DateTime<Utc>,
) -> Result<(StockPlan, Option<Check>), ToolError> {
    if level < 0 {
        return Err(
            ToolError::new(NEGATIVE_STOCK, "Stock cannot be below zero.")
                .detail(json!({"field": "inventory_level", "reason": "range"}))
                .next("set_variant_stock"),
        );
    }
    let level32 = i32::try_from(level).map_err(|_| {
        ToolError::invalid_input("`inventory_level` is too large.")
            .detail(json!({"field": "inventory_level", "reason": "range"}))
    })?;
    if s.availability == "discontinued" {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("{} is discontinued; its stock is not sold.", s.variant_id),
        )
        .detail(json!({"entity": "variant", "from": "discontinued"}))
        .next("get_product"));
    }
    let managed = managed.unwrap_or(s.is_managed);
    let before = StockState {
        inventory_level: i64::from(s.inventory_level),
        availability: s.availability.clone(),
        auto_out_of_stock: s.availability == "out_of_stock" && s.sold_out_at.is_some(),
    };
    let after = model::apply_stock(rules, &before, level, managed);
    let moved = after.availability != before.availability;
    let (sold_out_at, availability_starts) = match (moved, after.auto_out_of_stock) {
        (true, true) => (Some(now), s.availability_starts),
        (true, false) => (None, None),
        (false, _) => (s.sold_out_at, s.availability_starts),
    };
    let check = moved.then(|| {
        if after.auto_out_of_stock {
            Check {
                rule: "inventory.auto_out_of_stock".into(),
                passed: true,
                detail: Some(format!("At {level} units the variant goes out of stock")),
                fix_tool: None,
            }
        } else {
            Check {
                rule: "inventory.auto_restore".into(),
                passed: true,
                detail: Some(format!(
                    "It ran out by itself, so at {level} units it is back {}",
                    after.availability.replace('_', " ")
                )),
                fix_tool: None,
            }
        }
    });
    let plan = StockPlan {
        variant_id: s.variant_id.clone(),
        offer_id: s.offer_id.clone(),
        from: StockSnapshot {
            inventory_level: s.inventory_level,
            availability: s.availability.clone(),
            sold_out_at: s.sold_out_at,
            availability_starts: s.availability_starts,
            managed: s.is_managed,
        },
        to: StockSnapshot {
            inventory_level: level32,
            availability: after.availability,
            sold_out_at,
            availability_starts,
            managed,
        },
    };
    Ok((plan, check))
}

impl StockPlan {
    /// Whether the change brings a sold-out variant back on sale: the moment the shoppers
    /// waiting for it hear (`inventory.auto_restore.notify_restock_alerts`).
    pub fn restores(&self) -> bool {
        self.from.availability != self.to.availability && self.to.availability == "in_stock"
    }
}

pub(crate) fn snapshot_json(s: &StockSnapshot, with_managed: bool) -> Value {
    let mut v = json!({"inventory_level": s.inventory_level, "availability": s.availability});
    if with_managed {
        v["acom:managed"] = json!(s.managed);
    }
    v
}

pub(crate) async fn write(
    tx: &mut TxCx,
    plan: &StockPlan,
    s: &StockSnapshot,
) -> Result<(), ToolError> {
    let n = repo::set_stock(
        tx,
        &StockWrite {
            offer_id: &plan.offer_id,
            inventory_level: s.inventory_level,
            availability: &s.availability,
            sold_out_at: s.sold_out_at,
            availability_starts: s.availability_starts,
        },
    )
    .await?;
    if n == 0 {
        return Err(product_not_found(
            "variant",
            &plan.variant_id,
            "search_products",
        ));
    }
    repo::set_managed(tx, &plan.variant_id, s.managed).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use rust_decimal::Decimal;

    /// The chore coat M on Friday: sold out by itself on Wed Nov 4 15:00, back Nov 12.
    fn chore_coat_m() -> OfferState {
        let time = acom_core::StoreTime::new(chrono_tz::America::New_York);
        OfferState {
            variant_id: "var_chore_coat_m_olive".into(),
            product_group_id: "prd_chore_coat".into(),
            is_managed: true,
            variant_version: 1,
            offer_id: "off_chore_coat_m_olive".into(),
            price: Decimal::new(14200, 2),
            price_currency: "USD".into(),
            availability: "out_of_stock".into(),
            inventory_level: 0,
            cost_value: None,
            cost_currency: None,
            sold_out_at: time.parse("2026-11-04T15:00").ok(),
            availability_starts: time.parse("2026-11-12").ok(),
            version: 2,
        }
    }

    fn rules() -> StockRules {
        StockRules::from_policy(&crate::testing::policies()).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn restock_brings_a_sold_out_size_back() {
        let now = DateTime::from_timestamp(1_793_980_000, 0).unwrap_or_default();
        let (plan, check) =
            plan_stock(&rules(), &chore_coat_m(), 12, None, now).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(plan.to.availability, "in_stock");
        assert_eq!(
            (plan.to.sold_out_at, plan.to.availability_starts),
            (None, None)
        );
        assert_eq!(check.map(|c| c.rule), Some("inventory.auto_restore".into()));
        // Selling the last unit sends it out of stock and records when.
        let mut on_sale = chore_coat_m();
        on_sale.availability = "in_stock".into();
        on_sale.inventory_level = 1;
        on_sale.sold_out_at = None;
        let (plan, _) =
            plan_stock(&rules(), &on_sale, 0, None, now).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(plan.to.availability, "out_of_stock");
        assert_eq!(plan.to.sold_out_at, Some(now));
    }

    #[test]
    fn refuses_negative_stock_and_discontinued_variants() {
        let e = plan_stock(&rules(), &chore_coat_m(), -1, None, DateTime::default()).unwrap_err();
        assert_eq!(e.code, "negative_stock");
        let mut ended = chore_coat_m();
        ended.availability = "discontinued".into();
        let e = plan_stock(&rules(), &ended, 3, None, DateTime::default()).unwrap_err();
        assert_eq!(e.code, "invalid_state");
    }
}
