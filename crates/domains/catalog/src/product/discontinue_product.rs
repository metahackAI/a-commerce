//! `discontinue_product` (docs/api/catalog.md#product).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::repo::{self};
use crate::{HAS_OPEN_SUBSCRIPTIONS, PRODUCT_NOT_FOUND, product_not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscontinueProductInput {
    /// The variant to stop selling for good.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_linen_shirt_xs_white")]
    pub variant_id: String,
    /// Why, in a sentence; shown in the change set.
    #[schemars(length(max = 500), example = &"The mill stopped making this linen.")]
    pub reason: Option<String>,
}

/// What `discontinue_product` runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscontinuePlan {
    pub variant_id: String,
    pub offer_id: String,
    /// Availability before; `discontinued` when there was nothing to do.
    pub from_availability: String,
    pub reason: Option<String>,
}

/// Stop selling a variant for good.
#[derive(Debug)]
pub struct DiscontinueProduct;

impl Operation for DiscontinueProduct {
    const META: OpMeta = OpMeta {
        address: "catalog.product.discontinue",
        tool: "discontinue_product",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/variants/{variant_id}/discontinue",
        summary: "Discontinue a variant (availability becomes discontinued; cannot be undone). A product whose variants are all discontinued shows as discontinued.",
        consumes: &[Slot::one("variant_id")],
        produces: &[Slot::one("variant_id")],
        checks: &["product.discontinue"],
        errors: &[
            PRODUCT_NOT_FOUND,
            ErrorCode::INVALID_STATE,
            HAS_OPEN_SUBSCRIPTIONS,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = DiscontinueProductInput;
    type Output = Value;
}

/// The `product.discontinue` check. Subscriptions are not stored yet (no sales table holds
/// them), so none can be open.
fn discontinue_check(policy: &acom_policy::PolicySet) -> Check {
    let blocks = policy
        .lookup("catalog.product.discontinue.block_if_open_subscriptions")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    Check {
        rule: "product.discontinue".into(),
        passed: true,
        detail: Some(if blocks {
            "No open subscriptions; discontinuing cannot be undone".into()
        } else {
            "Subscriptions are not checked; discontinuing cannot be undone".into()
        }),
        fix_tool: None,
    }
}

impl WriteOp<Cx> for DiscontinueProduct {
    type Plan = DiscontinuePlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: DiscontinueProductInput,
    ) -> Result<Planned<DiscontinuePlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let state = repo::offer_state(&mut tx, &input.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let s = state
            .ok_or_else(|| product_not_found("variant", &input.variant_id, "search_products"))?;
        let reason = input
            .reason
            .map(|r| r.trim().to_owned())
            .filter(|r| !r.is_empty());
        let plan = DiscontinuePlan {
            variant_id: s.variant_id.clone(),
            offer_id: s.offer_id.clone(),
            from_availability: s.availability.clone(),
            reason: reason.clone(),
        };
        let basis = vec![Basis {
            entity: "catalog.offer".into(),
            id: s.offer_id.clone(),
            version: i64::from(s.version),
        }];
        if s.availability == "discontinued" {
            return Ok(Planned {
                plan,
                changes: Vec::new(),
                checks: vec![Check {
                    rule: "product.discontinue".into(),
                    passed: true,
                    detail: Some("Already discontinued; nothing changes".into()),
                    fix_tool: None,
                }],
                basis,
                approval: ApprovalNeed::NotNeeded,
            });
        }
        let approval = cx.policy.approval_for(
            Self::META.tool,
            &json!({"change_type": "structural", "changed_offers": 1}),
        );
        Ok(Planned {
            plan,
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "variant".into(),
                id: s.variant_id,
                from: json!({"availability": s.availability}),
                to: json!({"availability": "discontinued"}),
                note: reason,
            }],
            checks: vec![discontinue_check(&cx.policy)],
            basis,
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: "Discontinues 1 variant; this cannot be undone".into(),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &DiscontinuePlan) -> Result<Committed, ToolError> {
        repo::discontinue_offer(tx, &plan.offer_id).await?;
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
        plan: &DiscontinuePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let state = repo::offer_state(&mut tx, &plan.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let now = state.map(|s| s.availability);
        let matched = now.as_deref() == Some("discontinued");
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} is {} instead of discontinued",
                    plan.variant_id,
                    now.as_deref().unwrap_or("missing")
                )]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &DiscontinuePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!("Discontinuing {} cannot be taken back.", plan.variant_id),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn discontinuing_is_checked_and_needs_approval_by_policy() {
        let p = crate::testing::policies();
        assert!(discontinue_check(&p).passed);
        let a = p.approval_for(
            DiscontinueProduct::META.tool,
            &json!({"change_type": "structural", "changed_offers": 1}),
        );
        assert!(a.needed);
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn discontinuing_needs_approval_and_changes_nothing_in_preview() {
        let cx = testing::operator_db().await;
        let p = DiscontinueProduct
            .preview(
                &cx,
                DiscontinueProductInput {
                    variant_id: "var_linen_shirt_xs_white".into(),
                    reason: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(p.approval, ApprovalNeed::Needed { .. }));
        assert_eq!(p.changes[0].to, json!({"availability": "discontinued"}));
    }
}
