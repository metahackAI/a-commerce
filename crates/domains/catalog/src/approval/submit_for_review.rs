//! `submit_for_review` (docs/api/catalog.md#approval): a draft goes to review.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Committed, ErrorCode, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    NOTHING_TO_REVIEW, StatusPlan, basis, floor_check, invalid_state, load, move_status, note_of,
    publish_check, status_change, undo_status, verify_status,
};
use crate::PRODUCT_NOT_FOUND;
use crate::rules::{PriceFloorRule, ProductRules};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmitForReviewInput {
    /// The draft (or rejected) product to send to review.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_fw26_ot_014_chr")]
    pub product_id: String,
    /// A line for the reviewer.
    #[schemars(length(max = 500), example = &"Description written; ready to publish.")]
    pub note: Option<String>,
}

/// Send a product to review.
#[derive(Debug)]
pub struct SubmitForReview;

impl Operation for SubmitForReview {
    const META: OpMeta = OpMeta {
        address: "catalog.approval.submit",
        tool: "submit_for_review",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/products/{product_id}/submit",
        summary: "Send a draft (or rejected) product to review, once it has everything publishing needs. Publishing it is approve_product.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("product_id")],
        checks: &["product.publish_requires", "price_floor"],
        errors: &[
            ErrorCode::INVALID_STATE,
            NOTHING_TO_REVIEW,
            PRODUCT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SubmitForReviewInput;
    type Output = Value;
}

impl WriteOp<Cx> for SubmitForReview {
    type Plan = StatusPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SubmitForReviewInput,
    ) -> Result<Planned<StatusPlan>, ToolError> {
        let rules = ProductRules::from_policy(&cx.policy)?;
        let floor = PriceFloorRule::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let f = load(&mut tx, &input.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let from = f.group.approval_status.clone();
        if from != "draft" && from != "rejected" {
            return Err(invalid_state(
                &f.group.id,
                &from,
                &["draft", "rejected"],
                "get_product_group",
            ));
        }
        if f.variants.is_empty() {
            return Err(ToolError::new(
                NOTHING_TO_REVIEW,
                format!(
                    "{} has no variants yet, so there is nothing to review.",
                    f.group.id
                ),
            )
            .detail(json!({"entity": "product", "id": f.group.id}))
            .next("set_variants"));
        }
        let plan = StatusPlan {
            product_id: f.group.id.clone(),
            from,
            to: "pending_review".into(),
            release_date: None,
            from_release_date: None,
            note: note_of(input.note),
        };
        let approval = cx
            .policy
            .approval_for(Self::META.tool, &json!({"change_type": "content"}));
        Ok(Planned {
            changes: vec![status_change(&plan)],
            checks: vec![publish_check(&rules, &f), floor_check(&floor, &f)],
            basis: basis(&f.group),
            approval: match approval.reason {
                Some(reason) if approval.needed => ApprovalNeed::Needed { reason },
                _ => ApprovalNeed::NotNeeded,
            },
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
