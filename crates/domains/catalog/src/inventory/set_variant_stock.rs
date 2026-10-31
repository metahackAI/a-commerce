//! `set_variant_stock` (docs/api/catalog.md#inventory).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{StockPlan, plan_stock, snapshot_json, write};
use crate::repo::{self};
use crate::rules::StockRules;
use crate::{NEGATIVE_STOCK, PRODUCT_NOT_FOUND, product_not_found};

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
        let s = state
            .ok_or_else(|| product_not_found("variant", &input.variant_id, "search_products"))?;
        let (plan, check) = plan_stock(
            &rules,
            &s,
            input.inventory_level,
            input.managed,
            cx.clock.now(),
        )?;
        // Shoppers waiting for the variant hear about it when this runs.
        let alerts = crate::restock_alert::notify_preview(
            &mut tx,
            &plan.variant_id,
            plan.restores(),
            cx.clock.now(),
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
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
            changes: std::iter::once(Change {
                op: ChangeOp::Update,
                entity: "inventory_level".into(),
                id: plan.variant_id.clone(),
                from: snapshot_json(&plan.from, managed_moves),
                to: snapshot_json(&plan.to, managed_moves),
                note: input
                    .reason
                    .map(|r| r.trim().to_owned())
                    .filter(|r| !r.is_empty()),
            })
            .chain(alerts)
            .collect(),
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
        if plan.restores() {
            crate::restock_alert::notify_back_in_stock(tx, &plan.variant_id).await?;
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
    use crate::testing;
    use acom_contract::ReadOp;

    use crate::inventory::get_inventory::{GetInventory, GetInventoryInput};

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

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn the_chore_coat_m_back_in_stock_tells_the_two_shoppers_waiting() {
        let cx = testing::operator_db().await;
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
        // rsa_204 (Jamie, email) and rsa_205 (Grace, push) wait for it.
        let send = &planned.changes[1];
        assert_eq!(send.entity, "restock_alert");
        assert_eq!(send.to["shoppers"], 2);
        assert_eq!(send.to["by_channel"], json!({"email": 1, "push": 1}));
        // Run it in a transaction that is rolled back, so the store is left as it was.
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        SetVariantStock
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let sent: Vec<(String, String)> = sqlx::query_as(
            "SELECT object_id, payload->>'channel' FROM platform.outbox
             WHERE tenant_id = $1 AND topic = 'restock_alert.notify' ORDER BY object_id",
        )
        .bind(tx.tenant.as_str())
        .fetch_all(&mut *tx.tx)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            sent,
            [
                ("rsa_204".to_owned(), "email".to_owned()),
                ("rsa_205".to_owned(), "push".to_owned())
            ]
        );
        let waiting =
            crate::restock_alert::waiting(&mut tx, "var_chore_coat_m_olive", cx.clock.now())
                .await
                .unwrap_or_else(|e| panic!("{e}"));
        assert!(waiting.is_empty(), "{waiting:?}");
        // Running the same plan again sends nothing more.
        SetVariantStock
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
    }
}
