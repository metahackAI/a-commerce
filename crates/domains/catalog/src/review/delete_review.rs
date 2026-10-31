//! `delete_review` (docs/api/catalog.md#review): the shopper who wrote a review removes it, with
//! its helpful votes and reports. A review that is gone already gives the same result.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{shopper_id, writers_review};
use crate::{NOT_AUTHOR, REVIEW_NOT_FOUND};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteReviewInput {
    /// The review to remove; it must be the signed-in shopper's own (`list_reviews`).
    #[schemars(regex(pattern = r"^rev_[a-z0-9_]+$"), example = &"rev_01jd4m2k7c")]
    pub review_id: String,
}

/// What `delete_review` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteReviewPlan {
    pub review_id: String,
    pub author_id: String,
    pub product_id: String,
}

/// A shopper removes their own review.
#[derive(Debug)]
pub struct DeleteReview;

impl Operation for DeleteReview {
    const META: OpMeta = OpMeta {
        address: "catalog.review.delete",
        tool: "delete_review",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Delete,
        path: "/v1/catalog/reviews/{review_id}",
        summary: "The signed-in shopper removes a review they wrote, with its helpful votes; it leaves the product page and the review summary.",
        consumes: &[Slot::one("review_id")],
        produces: &[],
        checks: &[],
        errors: &[NOT_AUTHOR, REVIEW_NOT_FOUND],
        audience: Audience::Shopper,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = DeleteReviewInput;
    type Output = Value;
}

impl WriteOp<Cx> for DeleteReview {
    type Plan = DeleteReviewPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: DeleteReviewInput,
    ) -> Result<Planned<DeleteReviewPlan>, ToolError> {
        let author = shopper_id(cx, "remove a review")?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = writers_review(&mut tx, &input.review_id, &author).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Delete,
                entity: "review".into(),
                id: r.id.clone(),
                from: json!({"product_id": r.product_group_id, "review_rating": r.rating_value,
                             "review_body": r.review_body, "status": r.creative_work_status}),
                to: Value::Null,
                note: Some(
                    "Its helpful votes and reports go with it; it cannot be brought back".into(),
                ),
            }],
            checks: Vec::new(),
            basis: vec![Basis {
                entity: "catalog.review".into(),
                id: r.id.clone(),
                version: i64::from(r.version),
            }],
            approval: ApprovalNeed::NotNeeded,
            plan: DeleteReviewPlan {
                review_id: r.id,
                author_id: author,
                product_id: r.product_group_id,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &DeleteReviewPlan) -> Result<Committed, ToolError> {
        // Gone already (the same plan run twice) is the same result.
        super::remove_review(tx, &plan.review_id, &plan.author_id).await?;
        let mut produced = Map::new();
        produced.insert("review_id".into(), json!(plan.review_id));
        Ok(Committed {
            id: plan.review_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &DeleteReviewPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = super::owned_review(&mut tx, &plan.review_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: r.is_none(),
            mismatches: match r {
                None => Vec::new(),
                Some(_) => vec![format!("{} is still there", plan.review_id)],
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &DeleteReviewPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} was removed by the shopper who wrote it; it cannot be brought back.",
                plan.review_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    async fn guests_cannot_delete_reviews() {
        let e = DeleteReview
            .preview(
                &testing::guest(),
                DeleteReviewInput {
                    review_id: "rev_8812".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let schema = acom_contract::schema::closed_schema_for::<DeleteReviewInput>();
        assert!(acom_contract::schema::violations(&schema).is_empty());
    }
}
