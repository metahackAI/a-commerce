//! `unmap_category` (docs/api/merchandising.md#category).

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::*;
use crate::check_id;
use crate::repo;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnmapCategoryInput {
    /// The mapping to remove (`get_category` lists them).
    #[schemars(regex(pattern = r"^map_[a-z0-9_]+$"), example = &"map_summer_outerwear_travel")]
    pub category_mapping_id: String,
}

/// What `unmap_category` runs: the mapping as it was, to put back on undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmapPlan {
    pub mapping: Mapping,
}

/// Stop showing a standard category's products in a display category.
pub struct UnmapCategory;

impl Operation for UnmapCategory {
    const META: OpMeta = OpMeta {
        address: "merchandising.category.unmap",
        tool: "unmap_category",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Delete,
        path: "/v1/merchandising/category-mappings/{category_mapping_id}",
        summary: "Remove a standard-to-display category mapping. A primary mapping goes only when it is the last one; otherwise make another primary first. Then call invalidate_display_cache.",
        consumes: &[Slot::one("category_mapping_id")],
        produces: &[],
        checks: &["category.primary_display_per_standard"],
        errors: &[MAPPING_NOT_FOUND, PRIMARY_NEEDS_REASSIGN],
        audience: Audience::Operator,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = UnmapCategoryInput;
    type Output = Value;
}

impl WriteOp<Cx> for UnmapCategory {
    type Plan = UnmapPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UnmapCategoryInput,
    ) -> Result<Planned<UnmapPlan>, ToolError> {
        check_id("category_mapping_id", "map", &input.category_mapping_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let m = mapping(&mut tx, &input.category_mapping_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    MAPPING_NOT_FOUND,
                    format!("There is no mapping {}.", input.category_mapping_id),
                )
                .detail(json!({"entity": "category_mapping", "id": input.category_mapping_id}))
                .next("get_category")
            })?;
        let others = mappings_of(&mut tx, &m.standard_category_id)
            .await?
            .into_iter()
            .filter(|o| o.id != m.id && o.standard_category_id == m.standard_category_id)
            .count();
        let products = repo::product_count(&mut tx, &m.standard_category_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let blocked = m.is_primary && others > 0;
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Delete,
                entity: "category_mapping".into(),
                id: m.id.clone(),
                from: json!({"standard_category_id": m.standard_category_id,
                             "display_category_id": m.display_category_id, "is_primary": m.is_primary}),
                to: Value::Null,
                note: Some(format!(
                    "{products} products of {} stop showing under {}",
                    m.standard_category_id, m.display_category_id
                )),
            }],
            checks: vec![Check {
                rule: "category.primary_display_per_standard".into(),
                passed: !blocked,
                detail: Some(if blocked {
                    format!(
                        "It is the primary mapping and {others} more remain; make one of them primary first"
                    )
                } else if m.is_primary {
                    "The last mapping of this standard category; it will have no primary path"
                        .into()
                } else {
                    "Not the primary mapping".into()
                }),
                fix_tool: blocked.then(|| "map_category".into()),
            }],
            basis: Vec::new(),
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan: UnmapPlan { mapping: m },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &UnmapPlan) -> Result<Committed, ToolError> {
        delete_mapping(tx, &plan.mapping.id).await?;
        let mut produced = Map::new();
        produced.insert("category_mapping_id".into(), json!(plan.mapping.id));
        Ok(Committed {
            id: plan.mapping.id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &UnmapPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let still = mapping(&mut tx, &plan.mapping.id).await?.is_some();
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: !still,
            mismatches: if still {
                vec![format!("{} is still there", plan.mapping.id)]
            } else {
                Vec::new()
            },
        })
    }

    async fn undo(&self, tx: &mut TxCx, plan: &UnmapPlan, _d: &Committed) -> Result<(), ToolError> {
        insert_mapping(tx, &plan.mapping).await
    }
}
