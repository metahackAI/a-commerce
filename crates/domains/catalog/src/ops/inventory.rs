//! `get_inventory`, `set_variant_stock` (docs/api/catalog.md#inventory).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, ReadOp, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::product::visible_group;
use super::product_not_found;
use crate::model::{self, StockState};
use crate::repo::{self, OfferState, StockWrite};
use crate::rules::StockRules;
use crate::{NEGATIVE_STOCK, PRODUCT_NOT_FOUND, project};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetInventoryInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_chore_coat")]
    pub product_id: String,
    /// Only these top-level fields.
    #[schemars(example = ["product_id", "variants"])]
    pub fields: Option<Vec<String>>,
}

/// Stock per variant.
#[derive(Debug)]
pub struct GetInventory;

impl Operation for GetInventory {
    const META: OpMeta = OpMeta {
        address: "catalog.inventory.get",
        tool: "get_inventory",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/products/{product_id}/inventory",
        summary: "Sellable stock of each variant of a product, its safety stock, whether stock is tracked, and the availability that follows; with the product total.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("variant_id")],
        checks: &[],
        errors: &[PRODUCT_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetInventoryInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetInventory {
    async fn read(&self, cx: &Cx, input: GetInventoryInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = visible_group(cx, &mut tx, &input.product_id).await?;
        let (mut variants, ordered) = match &group {
            Some(g) => (
                repo::variants(&mut tx, std::slice::from_ref(&g.id)).await?,
                Some(super::reorder::on_order(cx, &mut tx, &g.id).await?),
            ),
            None => (Vec::new(), None),
        };
        tx.commit().await.map_err(ToolError::from)?;
        let Some(ordered) = ordered else {
            return Err(product_not_found(
                "product",
                &input.product_id,
                "search_products",
            ));
        };
        variants.retain(|v| v.offer_id.is_some());
        model::order_variants(&mut variants);
        let summary = model::summarize(&variants);
        let lines: Vec<Value> = variants
            .iter()
            .map(|v| {
                json!({"variant_id": v.id, "sku": v.sku, "size": v.size, "color": v.color,
                       "inventory_level": v.stock(),
                       "acom:safety_stock": v.safety_stock.unwrap_or(0),
                       "acom:managed": v.is_managed, "availability": v.availability,
                       "on_order": ordered.by_variant.get(&v.id).copied().unwrap_or(0)})
            })
            .collect();
        let out = json!({"product_id": input.product_id,
                         "inventory_level": summary.inventory_level,
                         "availability": summary.availability, "variants": lines,
                         "reorders": ordered.reorders});
        Ok(project(out, &input.fields.unwrap_or_default()))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetVariantStockInput {
    /// The variant.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_chore_coat_m_olive")]
    pub variant_id: String,
    /// Units that can be sold now (an absolute number, not a change).
    #[schemars(range(min = 0), example = 12)]
    pub inventory_level: i64,
    /// Whether stock is tracked. Untracked variants never run out by themselves.
    #[serde(rename = "acom:managed", alias = "managed")]
    #[schemars(example = true)]
    pub managed: Option<bool>,
    /// Why, in a sentence, for the change history.
    #[schemars(length(max = 500), example = &"Recount after the Nov 12 delivery.")]
    pub reason: Option<String>,
}

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

/// Set a variant's sellable stock.
#[derive(Debug)]
pub struct SetVariantStock;

impl Operation for SetVariantStock {
    const META: OpMeta = OpMeta {
        address: "catalog.inventory.set_stock",
        tool: "set_variant_stock",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Put,
        path: "/v1/catalog/variants/{variant_id}/inventory",
        summary: "Set the sellable stock of a variant. At 0 it goes out of stock by itself, and only a variant that ran out by itself comes back in stock when stock returns.",
        consumes: &[Slot::one("variant_id")],
        produces: &[Slot::one("variant_id")],
        checks: &["inventory.auto_out_of_stock", "inventory.auto_restore"],
        errors: &[NEGATIVE_STOCK, ErrorCode::INVALID_STATE, PRODUCT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetVariantStockInput;
    type Output = Value;
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

fn snapshot_json(s: &StockSnapshot, with_managed: bool) -> Value {
    let mut v = json!({"inventory_level": s.inventory_level, "availability": s.availability});
    if with_managed {
        v["acom:managed"] = json!(s.managed);
    }
    v
}

async fn write(tx: &mut TxCx, plan: &StockPlan, s: &StockSnapshot) -> Result<(), ToolError> {
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

impl WriteOp<Cx> for SetVariantStock {
    type Plan = StockPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetVariantStockInput,
    ) -> Result<Planned<StockPlan>, ToolError> {
        let rules = StockRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let state = repo::offer_state(&mut tx, &input.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let s = state
            .ok_or_else(|| product_not_found("variant", &input.variant_id, "search_products"))?;
        let (plan, check) = plan_stock(
            &rules,
            &s,
            input.inventory_level,
            input.managed,
            cx.clock.now(),
        )?;
        let managed_moves = plan.from.managed != plan.to.managed;
        let mut basis = vec![Basis {
            entity: "catalog.offer".into(),
            id: s.offer_id.clone(),
            version: i64::from(s.version),
        }];
        if managed_moves {
            basis.push(Basis {
                entity: "catalog.product".into(),
                id: s.variant_id.clone(),
                version: i64::from(s.variant_version),
            });
        }
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "inventory_level".into(),
                id: plan.variant_id.clone(),
                from: snapshot_json(&plan.from, managed_moves),
                to: snapshot_json(&plan.to, managed_moves),
                note: input
                    .reason
                    .map(|r| r.trim().to_owned())
                    .filter(|r| !r.is_empty()),
            }],
            checks: check.into_iter().collect(),
            basis,
            approval: match approval.reason {
                Some(reason) if approval.needed => ApprovalNeed::Needed { reason },
                _ => ApprovalNeed::NotNeeded,
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StockPlan) -> Result<Committed, ToolError> {
        write(tx, plan, &plan.to).await?;
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
        plan: &StockPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let state = repo::offer_state(&mut tx, &plan.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match state {
            None => vec![format!("{} has no offer", plan.variant_id)],
            Some(s) => {
                let mut m = Vec::new();
                if s.inventory_level != plan.to.inventory_level {
                    m.push(format!(
                        "{} has {} units instead of {}",
                        plan.variant_id, s.inventory_level, plan.to.inventory_level
                    ));
                }
                if s.availability != plan.to.availability {
                    m.push(format!(
                        "{} is {} instead of {}",
                        plan.variant_id, s.availability, plan.to.availability
                    ));
                }
                if s.is_managed != plan.to.managed {
                    m.push(format!("{} stock tracking did not change", plan.variant_id));
                }
                m
            }
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &StockPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        // Only take the count back if nothing sold or arrived since; otherwise the old number
        // would be wrong.
        let now = repo::offer_state(tx, &plan.variant_id).await?;
        if now.as_ref().map(|s| s.inventory_level) != Some(plan.to.inventory_level) {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "The stock of {} changed since this was set; set it again instead.",
                    plan.variant_id
                ),
            )
            .detail(json!({"entity": "inventory_level", "id": plan.variant_id}))
            .next("set_variant_stock"));
        }
        write(tx, plan, &plan.from).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;
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

    #[test]
    fn stock_changes_need_no_approval() {
        let p = crate::testing::policies();
        assert!(
            !p.approval_for(SetVariantStock::META.tool, &json!({}))
                .needed
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn chore_coat_inventory_reads_back() {
        let cx = testing::operator_db().await;
        let out = GetInventory
            .read(
                &cx,
                GetInventoryInput {
                    product_id: "prd_chore_coat".into(),
                    fields: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["inventory_level"], json!(7));
        let planned = SetVariantStock
            .preview(
                &cx,
                SetVariantStockInput {
                    variant_id: "var_chore_coat_m_olive".into(),
                    inventory_level: 12,
                    managed: None,
                    reason: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes[0].to["availability"], "in_stock");
    }
}
