//! `pause_promotion` (docs/api/promotion.md#promotion).

use acom_contract::{
    ActionType, Audience, Committed, ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned,
    Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::PromotionStatus;
use acom_store::{Cx, TxCx};
use serde_json::Value;

use super::{
    PromotionStateInput, StatusPlan, commit_status, preview_status, undo_status, verify_status,
};

pub struct PausePromotion;

impl Operation for PausePromotion {
    const META: OpMeta = OpMeta {
        address: "promotion.promotion.pause",
        tool: "pause_promotion",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/promotions/{promotion_id}/pause",
        summary: "Pause a running promotion (running → paused). While paused its member prices and coupons do not apply.",
        consumes: &[Slot::one("promotion_id")],
        produces: &[Slot::one("promotion_id")],
        checks: &[],
        errors: &[ErrorCode::INVALID_STATE, ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = PromotionStateInput;
    type Output = Value;
}

impl WriteOp<Cx> for PausePromotion {
    type Plan = StatusPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: PromotionStateInput,
    ) -> Result<Planned<StatusPlan>, ToolError> {
        preview_status(
            cx,
            Self::META.tool,
            input,
            PromotionStatus::Paused,
            &[PromotionStatus::Running],
        )
        .await
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StatusPlan) -> Result<Committed, ToolError> {
        commit_status(tx, plan).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &StatusPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        verify_status(cx, plan).await
    }

    async fn undo(&self, tx: &mut TxCx, plan: &StatusPlan, _: &Committed) -> Result<(), ToolError> {
        undo_status(tx, plan).await
    }
}
