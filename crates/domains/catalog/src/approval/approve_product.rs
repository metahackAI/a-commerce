//! `approve_product` (docs/api/catalog.md#approval): a person publishes a reviewed product in a change set.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Committed, ErrorCode, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    StatusPlan, basis, floor_check, gift_card_check, invalid_state, load, move_status, note_of,
    publish_check, required_check, status_change, undo_status, verify_status,
};
use crate::rules::{PriceFloorRule, ProductRules};
use crate::{BELOW_PRICE_FLOOR, PRODUCT_NOT_FOUND, parse_time};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApproveProductInput {
    /// The product under review to publish.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_fw26_ot_014_chr")]
    pub product_id: String,
    /// The release date shoppers sort "newest" by, in store time. Default today. The product is
    /// shown as soon as the change set runs.
    #[schemars(example = &"2026-11-09T00:00")]
    pub valid_from: Option<String>,
    /// A line for the change history.
    #[schemars(length(max = 500), example = &"FW26 drop")]
    pub note: Option<String>,
}

/// Publish a reviewed product.
#[derive(Debug)]
pub struct ApproveProduct;

impl Operation for ApproveProduct {
    const META: OpMeta = OpMeta {
        address: "catalog.approval.approve",
        tool: "approve_product",
        group: Group::Offer,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/catalog/products/{product_id}/approve",
        summary: "Publish a product under review so shoppers can find and buy it. It reaches customers, so it always runs in a change set a person approves.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("product_id")],
        checks: &[
            "price_floor",
            "product.required_on_create",
            "product.publish_requires",
            "gift_card.disallow",
        ],
        errors: &[
            ErrorCode::INVALID_STATE,
            BELOW_PRICE_FLOOR,
            PRODUCT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::AUTHORIZE,
        person_scope: "",
    };
    type Input = ApproveProductInput;
    type Output = Value;
}

impl WriteOp<Cx> for ApproveProduct {
    type Plan = StatusPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ApproveProductInput,
    ) -> Result<Planned<StatusPlan>, ToolError> {
        let rules = ProductRules::from_policy(&cx.policy)?;
        let floor = PriceFloorRule::from_policy(&cx.policy)?;
        let release = parse_time(cx, "valid_from", input.valid_from.as_deref())?
            .unwrap_or_else(|| cx.clock.now());
        let release_date = release.with_timezone(&cx.time.zone).date_naive();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let f = load(&mut tx, &input.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if f.group.approval_status != "pending_review" {
            return Err(invalid_state(
                &f.group.id,
                &f.group.approval_status,
                &["pending_review"],
                "submit_for_review",
            ));
        }
        let plan = StatusPlan {
            product_id: f.group.id.clone(),
            from: "pending_review".into(),
            to: "active".into(),
            release_date: Some(release_date),
            from_release_date: f.group.release_date,
            note: note_of(input.note),
        };
        let mut change = status_change(&plan);
        change.to["release_date"] = json!(release_date.to_string());
        change.to["variants"] = json!(f.variants.len());
        Ok(Planned {
            changes: vec![change],
            checks: vec![
                floor_check(&floor, &f),
                required_check(&rules, &f),
                publish_check(&rules, &f),
                gift_card_check(&cx.policy, &f),
            ],
            basis: basis(&f.group),
            approval: ApprovalNeed::Needed {
                reason: format!("Publishes 1 product: {}", f.group.name),
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StatusPlan) -> Result<Committed, ToolError> {
        move_status(tx, plan, true).await
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
        undo_status(tx, plan, true).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    use crate::approval::reject_product::RejectProductInput;
    use crate::approval::{StatusPlan, status_change};

    #[test]
    fn approving_always_waits_for_a_person_and_rejecting_needs_a_reason() {
        assert_eq!(ApproveProduct::META.kind, Kind::WriteApproval);
        assert!(
            serde_json::from_value::<RejectProductInput>(json!({"product_id": "prd_x"})).is_err()
        );
        let undo = StatusPlan {
            product_id: "prd_x".into(),
            from: "pending_review".into(),
            to: "active".into(),
            release_date: NaiveDate::from_ymd_opt(2026, 11, 6),
            from_release_date: None,
            note: None,
        };
        assert_eq!(status_change(&undo).to["approval_status"], "active");
    }
}
