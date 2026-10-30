//! `set_safety_stock` (docs/api/catalog.md#inventory): the level a variant's stock alerts at.
//! Alert only (`inventory.safety_stock.alert_only`): availability never changes because of it.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorClass, ErrorCode, Group,
    Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::product_not_found;
use crate::{PRODUCT_NOT_FOUND, repo, write};

/// A safety stock under zero.
pub const NEGATIVE_VALUE: ErrorCode = ErrorCode::new("negative_value", ErrorClass::InvalidInput);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetSafetyStockInput {
    /// The variant.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_chore_coat_m_olive")]
    pub variant_id: String,
    /// Units at or under which the variant is flagged as running low.
    #[serde(rename = "acom:safety_stock", alias = "safety_stock")]
    #[schemars(range(min = 0), example = 3)]
    pub safety_stock: i64,
    /// false turns the alert off (the level is set to 0). Default true.
    #[schemars(example = true)]
    pub enabled: Option<bool>,
}

/// What `set_safety_stock` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyPlan {
    pub variant_id: String,
    pub offer_id: String,
    pub level: i32,
    pub from: i32,
}

/// Set the stock level a variant alerts at.
#[derive(Debug)]
pub struct SetSafetyStock;

impl Operation for SetSafetyStock {
    const META: OpMeta = OpMeta {
        address: "catalog.inventory.set_safety_stock",
        tool: "set_safety_stock",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Put,
        path: "/v1/catalog/variants/{variant_id}/inventory/safety",
        summary: "Set the stock level at which a variant is flagged as running low. An alert only: it never changes whether the variant can be bought.",
        consumes: &[Slot::one("variant_id")],
        produces: &[Slot::one("variant_id")],
        checks: &["inventory.safety_stock"],
        errors: &[NEGATIVE_VALUE, PRODUCT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetSafetyStockInput;
    type Output = Value;
}

async fn current(tx: &mut TxCx, variant_id: &str) -> Result<Option<(String, i32, i32)>, ToolError> {
    sqlx::query_as::<_, (String, i32, i32)>(
        "SELECT o.id, o.safety_stock, o.version FROM catalog.offer o
         WHERE o.tenant_id = $1 AND o.product_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(variant_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(crate::db)
}

impl WriteOp<Cx> for SetSafetyStock {
    type Plan = SafetyPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetSafetyStockInput,
    ) -> Result<Planned<SafetyPlan>, ToolError> {
        if input.safety_stock < 0 {
            return Err(
                ToolError::new(NEGATIVE_VALUE, "Safety stock cannot be below zero.")
                    .detail(json!({"field": "acom:safety_stock", "reason": "range"}))
                    .next("set_safety_stock"),
            );
        }
        let enabled = cx
            .policy
            .lookup("catalog.inventory.safety_stock.enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let level = if input.enabled == Some(false) {
            0
        } else {
            i32::try_from(input.safety_stock).map_err(|_| {
                ToolError::invalid_input("`acom:safety_stock` is too large.")
                    .detail(json!({"field": "acom:safety_stock", "reason": "range"}))
            })?
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let found = current(&mut tx, &input.variant_id).await?;
        let variant = repo::variant(&mut tx, &input.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let (offer_id, from, version) = found
            .ok_or_else(|| product_not_found("variant", &input.variant_id, "search_products"))?;
        let stock = variant.and_then(|v| v.inventory_level).unwrap_or(0);
        let plan = SafetyPlan {
            variant_id: input.variant_id.clone(),
            offer_id: offer_id.clone(),
            level,
            from,
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "inventory_level".into(),
                id: input.variant_id,
                from: json!({"acom:safety_stock": from}),
                to: json!({"acom:safety_stock": level}),
                note: None,
            }],
            checks: vec![Check {
                rule: "inventory.safety_stock".into(),
                passed: true,
                detail: Some(if !enabled {
                    "Safety stock alerts are off for this store; the level is kept".into()
                } else if level == 0 {
                    "No alert for this variant".into()
                } else {
                    format!(
                        "Alerts at {level} units or fewer (now {stock}); whether it can be bought does not change"
                    )
                }),
                fix_tool: None,
            }],
            basis: vec![Basis {
                entity: "catalog.offer".into(),
                id: offer_id,
                version: i64::from(version),
            }],
            approval: crate::policy_approval(
                cx,
                Self::META.tool,
                &json!({"change_type": "content"}),
            ),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SafetyPlan) -> Result<Committed, ToolError> {
        if write::set_safety_stock(tx, &plan.offer_id, plan.level).await? == 0 {
            return Err(product_not_found(
                "variant",
                &plan.variant_id,
                "search_products",
            ));
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
        plan: &SafetyPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let now = current(&mut tx, &plan.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = now.is_some_and(|(_, level, _)| level == plan.level);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} safety stock is not {}",
                    plan.variant_id, plan.level
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SafetyPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        write::set_safety_stock(tx, &plan.offer_id, plan.from)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;

    #[tokio::test]
    async fn negative_levels_are_refused() {
        let e = SetSafetyStock
            .preview(
                &testing::cx(acom_core::enums::ActorKind::Operator, "adm_sj"),
                serde_json::from_value(json!({"variant_id": "var_x", "acom:safety_stock": -1}))
                    .unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("negative_value", Some("set_safety_stock"))
        );
        let schema = acom_contract::schema::closed_schema_for::<SetSafetyStockInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }
}
