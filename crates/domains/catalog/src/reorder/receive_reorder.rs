//! `receive_reorder` (docs/api/catalog.md#reorder).

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::OPEN;
use crate::inventory::{StockPlan, plan_stock, snapshot_json, write as write_stock};
use crate::rules::StockRules;
use crate::{db, product_not_found, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReceiveReorderInput {
    /// The reorder that arrived (`get_inventory` `reorders`).
    #[schemars(regex(pattern = r"^ro_[a-z0-9_]+$"), example = &"ro_01jd2k8")]
    pub reorder_id: String,
}

/// What `receive_reorder` runs: each variant's stock before and after, and the reorder's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceivePlan {
    pub reorder_id: String,
    pub product_id: String,
    /// `processing` or `in_transit`, to go back to on undo.
    pub from_status: String,
    pub stock: Vec<StockPlan>,
}

/// Take a reorder in: its units become sellable stock and it is delivered.
#[derive(Debug)]
pub struct ReceiveReorder;

impl Operation for ReceiveReorder {
    const META: OpMeta = OpMeta {
        address: "catalog.reorder.receive",
        tool: "receive_reorder",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/reorders/{reorder_id}/receive",
        summary: "Take in a reorder that arrived: its units are added to each variant's sellable stock (a variant that ran out by itself is back in stock) and the reorder is delivered.",
        consumes: &[Slot::one("reorder_id")],
        produces: &[Slot::many("variant_id")],
        checks: &["inventory.auto_restore"],
        errors: &[ErrorCode::NOT_FOUND, ErrorCode::INVALID_STATE],
        audience: Audience::Operator,
        action_type: ActionType::RECEIVE,
        person_scope: "",
    };
    type Input = ReceiveReorderInput;
    type Output = Value;
}

/// The receipt of `reorder_id` as it would run now, read in `tx`.
pub(crate) async fn plan_receipt(
    tx: &mut TxCx,
    rules: &StockRules,
    reorder_id: &str,
) -> Result<(ReceivePlan, Vec<Check>, Vec<Basis>), ToolError> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT product_group_id, order_status FROM catalog.reorder
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(reorder_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let (product_id, status) = row.ok_or_else(|| {
        ToolError::new(
            ErrorCode::NOT_FOUND,
            format!("There is no reorder {reorder_id}."),
        )
        .detail(json!({"entity": "reorder", "id": reorder_id}))
        .next("get_inventory")
    })?;
    if !OPEN.contains(&status.as_str()) {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("{reorder_id} is {status}; only a reorder still on its way can be received."),
        )
        .detail(json!({"entity": "reorder", "id": reorder_id, "from": status}))
        .next("get_inventory"));
    }
    let items: Vec<(String, i32)> = sqlx::query_as(
        "SELECT product_id, order_quantity FROM catalog.reorder_item
         WHERE tenant_id = $1 AND reorder_id = $2 ORDER BY position",
    )
    .bind(tx.tenant.as_str())
    .bind(reorder_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    let now = tx.clock.now();
    let mut stock = Vec::new();
    let mut checks = Vec::new();
    let mut basis = Vec::new();
    for (variant_id, quantity) in items {
        let s = repo::offer_state(tx, &variant_id)
            .await?
            .ok_or_else(|| product_not_found("variant", &variant_id, "get_inventory"))?;
        let level = i64::from(s.inventory_level) + i64::from(quantity);
        let (plan, check) = plan_stock(rules, &s, level, None, now)?;
        checks.extend(check);
        basis.push(Basis {
            entity: "catalog.offer".into(),
            id: s.offer_id.clone(),
            version: i64::from(s.version),
        });
        stock.push(plan);
    }
    Ok((
        ReceivePlan {
            reorder_id: reorder_id.to_owned(),
            product_id,
            from_status: status,
            stock,
        },
        checks,
        basis,
    ))
}

async fn set_status(
    tx: &mut TxCx,
    reorder_id: &str,
    from: &str,
    to: &str,
) -> Result<bool, ToolError> {
    let n = sqlx::query(
        "UPDATE catalog.reorder SET order_status = $4, updated_at = $5, version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND order_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(reorder_id)
    .bind(from)
    .bind(to)
    .bind(tx.clock.now())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected();
    Ok(n == 1)
}

impl WriteOp<Cx> for ReceiveReorder {
    type Plan = ReceivePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ReceiveReorderInput,
    ) -> Result<Planned<ReceivePlan>, ToolError> {
        let rules = StockRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (plan, checks, basis) = plan_receipt(&mut tx, &rules, &input.reorder_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut changes = vec![Change {
            op: ChangeOp::Update,
            entity: "reorder".into(),
            id: plan.reorder_id.clone(),
            from: json!({"order_status": plan.from_status}),
            to: json!({"order_status": "delivered"}),
            note: None,
        }];
        changes.extend(plan.stock.iter().map(|p| Change {
            op: ChangeOp::Update,
            entity: "inventory_level".into(),
            id: p.variant_id.clone(),
            from: snapshot_json(&p.from, false),
            to: snapshot_json(&p.to, false),
            note: None,
        }));
        Ok(Planned {
            changes,
            checks,
            basis,
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ReceivePlan) -> Result<Committed, ToolError> {
        if !set_status(tx, &plan.reorder_id, &plan.from_status, "delivered").await? {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} changed since it was previewed; preview it again.",
                    plan.reorder_id
                ),
            )
            .detail(json!({"entity": "reorder", "id": plan.reorder_id}))
            .next("get_inventory"));
        }
        for p in &plan.stock {
            write_stock(tx, p, &p.to).await?;
        }
        let mut produced = Map::new();
        produced.insert(
            "variant_id".into(),
            json!(
                plan.stock
                    .iter()
                    .map(|p| p.variant_id.clone())
                    .collect::<Vec<_>>()
            ),
        );
        Ok(Committed {
            id: plan.reorder_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ReceivePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let status: Option<String> = sqlx::query_scalar(
            "SELECT order_status FROM catalog.reorder WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.reorder_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let mut mismatches = Vec::new();
        if status.as_deref() != Some("delivered") {
            mismatches.push(format!("{} is not delivered", plan.reorder_id));
        }
        for p in &plan.stock {
            let now = repo::offer_state(&mut tx, &p.variant_id).await?;
            if now.map(|s| s.inventory_level) != Some(p.to.inventory_level) {
                mismatches.push(format!(
                    "{} does not have {} units",
                    p.variant_id, p.to.inventory_level
                ));
            }
        }
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ReceivePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        // Only put the counts back if nothing sold or was counted since; otherwise they are wrong.
        for p in &plan.stock {
            let now = repo::offer_state(tx, &p.variant_id).await?;
            if now.map(|s| s.inventory_level) != Some(p.to.inventory_level) {
                return Err(ToolError::new(
                    ErrorCode::INVALID_STATE,
                    format!(
                        "The stock of {} changed since the reorder was received; correct it with set_variant_stock.",
                        p.variant_id
                    ),
                )
                .detail(json!({"entity": "inventory_level", "id": p.variant_id}))
                .next("set_variant_stock"));
            }
        }
        for p in &plan.stock {
            write_stock(tx, p, &p.from).await?;
        }
        if set_status(tx, &plan.reorder_id, "delivered", &plan.from_status).await? {
            Ok(())
        } else {
            Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("{} is no longer delivered.", plan.reorder_id),
            )
            .detail(json!({"entity": "reorder", "id": plan.reorder_id})))
        }
    }
}
