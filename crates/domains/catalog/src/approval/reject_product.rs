//! `reject_product` (docs/api/catalog.md#approval): a product under review goes back for editing.

use acom_contract::{
    ActionType, Audience, Committed, ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned,
    Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    StatusPlan, basis, invalid_state, move_status, note_of, status_change, undo_status,
    verify_status,
};
use crate::repo::{self};
use crate::{PRODUCT_NOT_FOUND, product_not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RejectProductInput {
    /// The product under review to send back.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_fw26_pt_011_snd")]
    pub product_id: String,
    /// Why, for whoever edits it next.
    #[schemars(length(min = 1, max = 500), example = &"The photo shows two trousers; shoot this color alone.")]
    pub reason: String,
}

/// Send a product under review back for editing.
#[derive(Debug)]
pub struct RejectProduct;

impl Operation for RejectProduct {
    const META: OpMeta = OpMeta {
        address: "catalog.approval.reject",
        tool: "reject_product",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/products/{product_id}/reject",
        summary: "Send a product under review back with a reason; it can be edited and submitted again.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("product_id")],
        checks: &[],
        errors: &[ErrorCode::INVALID_STATE, PRODUCT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::REJECT,
        person_scope: "",
    };
    type Input = RejectProductInput;
    type Output = Value;
}

impl WriteOp<Cx> for RejectProduct {
    type Plan = StatusPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RejectProductInput,
    ) -> Result<Planned<StatusPlan>, ToolError> {
        let Some(reason) = note_of(Some(input.reason)) else {
            return Err(ToolError::invalid_input("Say why the product goes back.")
                .detail(json!({"field": "reason", "reason": "missing"})));
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let g = repo::group(&mut tx, &input.product_id)
            .await?
            .ok_or_else(|| {
                product_not_found("product", &input.product_id, "search_product_groups")
            })?;
        tx.commit().await.map_err(ToolError::from)?;
        if g.approval_status != "pending_review" {
            return Err(invalid_state(
                &g.id,
                &g.approval_status,
                &["pending_review"],
                "get_change_history",
            ));
        }
        let plan = StatusPlan {
            product_id: g.id.clone(),
            from: "pending_review".into(),
            to: "rejected".into(),
            release_date: None,
            from_release_date: None,
            note: Some(reason),
        };
        Ok(Planned {
            changes: vec![status_change(&plan)],
            checks: Vec::new(),
            basis: basis(&g),
            approval: crate::policy_approval(
                cx,
                Self::META.tool,
                &json!({"change_type": "content"}),
            ),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StatusPlan) -> Result<Committed, ToolError> {
        move_status(tx, plan, false).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &StatusPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        verify_status(cx, plan).await
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &StatusPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        undo_status(tx, plan, false).await
    }
}
