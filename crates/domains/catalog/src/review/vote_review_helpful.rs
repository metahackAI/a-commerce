//! `vote_review_helpful` (docs/api/catalog.md#review).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::shopper_id;
use crate::REVIEW_NOT_FOUND;
use crate::repo::{self};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoteReviewHelpfulInput {
    /// The review.
    #[schemars(regex(pattern = r"^rev_[a-z0-9_]+$"), example = &"rev_8812")]
    pub review_id: String,
    /// `true` marks it helpful; `false` takes the vote back.
    #[schemars(example = true)]
    pub helpful: bool,
}

/// What `vote_review_helpful` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VotePlan {
    pub review_id: String,
    pub customer_id: String,
    pub helpful: bool,
    /// The shopper's vote before, if any.
    pub from_vote: Option<bool>,
}

/// Mark a review helpful, or take that back.
#[derive(Debug)]
pub struct VoteReviewHelpful;

impl Operation for VoteReviewHelpful {
    const META: OpMeta = OpMeta {
        address: "catalog.review.vote_helpful",
        tool: "vote_review_helpful",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/reviews/{review_id}/helpful",
        summary: "The signed-in shopper marks a review as helpful, or takes the vote back. One vote per shopper and review.",
        consumes: &[Slot::one("review_id")],
        produces: &[Slot::one("review_id")],
        checks: &[],
        errors: &[REVIEW_NOT_FOUND],
        audience: Audience::Shopper,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = VoteReviewHelpfulInput;
    type Output = Value;
}

/// The helpful count after a shopper's vote moves from `from` to `helpful`.
pub(crate) fn helpful_after(count: i64, from: Option<bool>, helpful: bool) -> i64 {
    count - i64::from(from == Some(true)) + i64::from(helpful)
}

impl WriteOp<Cx> for VoteReviewHelpful {
    type Plan = VotePlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: VoteReviewHelpfulInput,
    ) -> Result<Planned<VotePlan>, ToolError> {
        let customer = shopper_id(cx, "vote on reviews")?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let head = repo::review_head(&mut tx, &input.review_id)
            .await?
            .filter(|r| r.creative_work_status != "hidden");
        let from_vote = match &head {
            Some(_) => repo::vote(&mut tx, &input.review_id, &customer).await?,
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;
        let head = head.ok_or_else(|| {
            ToolError::new(
                REVIEW_NOT_FOUND,
                format!("There is no review {}.", input.review_id),
            )
            .detail(json!({"entity": "review", "id": input.review_id}))
            .next("list_reviews")
        })?;
        let after = helpful_after(head.helpful_count, from_vote, input.helpful);
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "review".into(),
                id: head.id.clone(),
                from: json!({"helpful_count": head.helpful_count}),
                to: json!({"helpful_count": after}),
                note: None,
            }],
            checks: Vec::new(),
            basis: vec![Basis {
                entity: "catalog.review".into(),
                id: head.id.clone(),
                version: i64::from(head.version),
            }],
            approval: match approval.reason {
                Some(reason) if approval.needed => ApprovalNeed::Needed { reason },
                _ => ApprovalNeed::NotNeeded,
            },
            plan: VotePlan {
                review_id: head.id,
                customer_id: customer,
                helpful: input.helpful,
                from_vote,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &VotePlan) -> Result<Committed, ToolError> {
        repo::set_vote(tx, &plan.review_id, &plan.customer_id, plan.helpful).await?;
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
        plan: &VotePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let vote = repo::vote(&mut tx, &plan.review_id, &plan.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = vote == Some(plan.helpful);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("the vote on {} did not change", plan.review_id)]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &VotePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!("A vote on {} is changed by voting again.", plan.review_id),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::create_review::{CreateReview, CreateReviewInput};
    use crate::testing;
    use acom_core::enums::FitVote;

    #[test]
    fn helpful_votes_count_once_per_shopper() {
        assert_eq!(helpful_after(4, None, true), 5);
        assert_eq!(helpful_after(5, Some(true), true), 5);
        assert_eq!(helpful_after(5, Some(true), false), 4);
        assert_eq!(helpful_after(4, Some(false), false), 4);
    }

    #[tokio::test]
    async fn guests_must_sign_in_to_review_or_vote() {
        let e = VoteReviewHelpful
            .preview(
                &testing::guest(),
                VoteReviewHelpfulInput {
                    review_id: "rev_8812".into(),
                    helpful: true,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let e = CreateReview
            .preview(
                &testing::guest(),
                CreateReviewInput {
                    product_id: "prd_windbreaker".into(),
                    order_id: "ord_20268".into(),
                    review_rating: 5,
                    review_body: "Great.".into(),
                    image: None,
                    fit: None,
                    height_cm: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_votes_and_reviews_the_windbreaker() {
        let cx = testing::shopper_db().await;
        let vote = VoteReviewHelpful
            .preview(
                &cx,
                VoteReviewHelpfulInput {
                    review_id: "rev_8812".into(),
                    helpful: true,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(vote.changes[0].to["helpful_count"], json!(1));
        let review = CreateReview
            .preview(
                &cx,
                CreateReviewInput {
                    product_id: "prd_windbreaker".into(),
                    order_id: "ord_20268".into(),
                    review_rating: 5,
                    review_body: "Packs into its pocket and kept the drizzle off.".into(),
                    image: None,
                    fit: Some(FitVote::TrueToSize),
                    height_cm: Some(178),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(review.checks.len(), 3);
    }
}
