//! `update_review` (docs/api/catalog.md#review): the shopper who wrote a review changes its
//! stars, text or photos. The same word filter as a new review applies.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{review_not_found, shopper_id, writers_review};
use crate::model;
use crate::repo;
use crate::rules::ReviewRules;
use crate::{NOT_AUTHOR, PROFANITY_BLOCKED, REVIEW_NOT_FOUND, blocked_words_check};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateReviewInput {
    /// The review to change; it must be the signed-in shopper's own (`list_reviews`).
    #[schemars(regex(pattern = r"^rev_[a-z0-9_]+$"), example = &"rev_01jd4m2k7c")]
    pub review_id: String,
    /// New stars, 1 (worst) to 5 (best). Leave out to keep them.
    #[schemars(range(min = 1, max = 5), example = 4)]
    pub review_rating: Option<u8>,
    /// New review text. Leave out to keep it.
    #[schemars(length(min = 1, max = 4000), example = &"Packs into its pocket. The hood is a bit small over a cap.")]
    pub review_body: Option<String>,
    /// New photo URLs, replacing the old ones; an empty list removes them. Leave out to keep them.
    #[schemars(example = ["https://cdn.northwind.example/r/wb_jamie_2.webp"])]
    pub image: Option<Vec<String>>,
}

/// What a review reads as before or after the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewText {
    pub rating: i16,
    pub body: String,
    pub image: Vec<String>,
}

/// What `update_review` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateReviewPlan {
    pub review_id: String,
    pub author_id: String,
    pub from: ReviewText,
    pub to: ReviewText,
    /// The mention phrases the new text uses.
    pub keywords: Vec<String>,
    /// The moderation state after the change: `reported` when a blocked word is held for review.
    pub status: String,
    /// The blocked word found, when the store refuses such text.
    pub blocked_word: Option<String>,
}

/// A shopper changes their own review.
#[derive(Debug)]
pub struct UpdateReview;

impl Operation for UpdateReview {
    const META: OpMeta = OpMeta {
        address: "catalog.review.update",
        tool: "update_review",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/catalog/reviews/{review_id}",
        summary: "The signed-in shopper changes the stars, text or photos of a review they wrote. Blocked words are refused as in a new review.",
        consumes: &[Slot::one("review_id")],
        produces: &[Slot::one("review_id")],
        checks: &["review.profanity_filter"],
        errors: &[NOT_AUTHOR, REVIEW_NOT_FOUND, PROFANITY_BLOCKED],
        audience: Audience::Shopper,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateReviewInput;
    type Output = Value;
}

/// Photo URLs as given: trimmed, empty ones dropped, each an https URL.
pub(crate) fn photo_urls(list: Vec<String>) -> Result<Vec<String>, ToolError> {
    let image: Vec<String> = list
        .into_iter()
        .map(|u| u.trim().to_owned())
        .filter(|u| !u.is_empty())
        .collect();
    if let Some(bad) = image.iter().find(|u| !u.starts_with("https://")) {
        return Err(
            ToolError::invalid_input(format!("`{bad}` is not an https URL."))
                .detail(json!({"field": "image", "reason": "format"})),
        );
    }
    Ok(image)
}

impl WriteOp<Cx> for UpdateReview {
    type Plan = UpdateReviewPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateReviewInput,
    ) -> Result<Planned<UpdateReviewPlan>, ToolError> {
        let author = shopper_id(cx, "change a review")?;
        if input.review_rating.is_none() && input.review_body.is_none() && input.image.is_none() {
            return Err(ToolError::invalid_input(
                "Give the new `review_rating`, `review_body` or `image`.",
            )
            .detail(json!({"field": "review_body", "reason": "missing"}))
            .next("update_review"));
        }
        if input.review_rating.is_some_and(|r| !(1..=5).contains(&r)) {
            return Err(ToolError::invalid_input("`review_rating` is 1–5.")
                .detail(json!({"field": "review_rating", "reason": "range"})));
        }
        let body = input.review_body.map(|b| b.trim().to_owned());
        if body.as_deref().is_some_and(str::is_empty) {
            return Err(ToolError::invalid_input("The review text is empty.")
                .detail(json!({"field": "review_body", "reason": "missing"})));
        }
        let image = input.image.map(photo_urls).transpose()?;
        let rules = ReviewRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = writers_review(&mut tx, &input.review_id, &author).await?;
        let known = repo::known_mentions(&mut tx, &r.product_group_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let from = ReviewText {
            rating: r.rating_value,
            body: r.review_body.clone(),
            image: r.image.clone(),
        };
        let to = ReviewText {
            rating: input.review_rating.map_or(from.rating, i16::from),
            body: body.unwrap_or_else(|| from.body.clone()),
            image: image.unwrap_or_else(|| from.image.clone()),
        };
        let blocked = model::blocked_word(&to.body, &rules.blocked_words);
        let blocks = rules.profanity_filter == "block";
        let check = blocked_words_check(
            "review.profanity_filter",
            blocks,
            blocked.as_deref(),
            "update_review",
        );
        let held = blocked.is_some() && !blocks;
        let status = if held && r.creative_work_status == "visible" {
            "reported".to_owned()
        } else {
            r.creative_work_status.clone()
        };
        let mut before = Map::new();
        let mut after = Map::new();
        if from.rating != to.rating {
            before.insert("review_rating".into(), json!(from.rating));
            after.insert("review_rating".into(), json!(to.rating));
        }
        if from.body != to.body {
            before.insert("review_body".into(), json!(from.body));
            after.insert("review_body".into(), json!(to.body));
        }
        if from.image != to.image {
            before.insert("image".into(), json!(from.image));
            after.insert("image".into(), json!(to.image));
        }
        if status != r.creative_work_status {
            before.insert("status".into(), json!(r.creative_work_status));
            after.insert("status".into(), json!(status));
        }
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "review".into(),
                id: r.id.clone(),
                from: Value::Object(before),
                to: Value::Object(after),
                note: (from == to).then(|| "Nothing changes".into()),
            }],
            checks: vec![check],
            basis: vec![Basis {
                entity: "catalog.review".into(),
                id: r.id.clone(),
                version: i64::from(r.version),
            }],
            approval: ApprovalNeed::NotNeeded,
            plan: UpdateReviewPlan {
                review_id: r.id,
                author_id: author,
                keywords: model::mentions_in(&to.body, &known),
                from,
                to,
                status,
                blocked_word: blocked.filter(|_| blocks),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &UpdateReviewPlan) -> Result<Committed, ToolError> {
        if let Some(w) = &plan.blocked_word {
            return Err(crate::profanity_blocked(
                "review.profanity_filter",
                w,
                "update_review",
            ));
        }
        let n = super::edit_review(
            tx,
            &super::ReviewEdit {
                id: &plan.review_id,
                author_id: &plan.author_id,
                rating: plan.to.rating,
                body: &plan.to.body,
                image: &plan.to.image,
                keywords: &plan.keywords,
                status: &plan.status,
            },
        )
        .await?;
        if n == 0 {
            return Err(review_not_found(&plan.review_id));
        }
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
        plan: &UpdateReviewPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = super::owned_review(&mut tx, &plan.review_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = r.is_some_and(|r| {
            r.rating_value == plan.to.rating
                && r.review_body == plan.to.body
                && r.image == plan.to.image
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} does not read back as changed", plan.review_id)]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &UpdateReviewPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} is the shopper's own review; they change it again with update_review.",
                plan.review_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn input_is_closed_and_photos_are_https() {
        let schema = acom_contract::schema::closed_schema_for::<UpdateReviewInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            photo_urls(vec![" https://x.example/a.webp ".into(), String::new()]).ok(),
            Some(vec!["https://x.example/a.webp".to_owned()])
        );
        assert!(photo_urls(vec!["http://x.example/a.webp".into()]).is_err());
    }

    #[tokio::test]
    async fn guests_must_sign_in_and_something_must_change() {
        let input = || UpdateReviewInput {
            review_id: "rev_8812".into(),
            review_rating: None,
            review_body: None,
            image: None,
        };
        let e = UpdateReview
            .preview(&testing::guest(), input())
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let e = UpdateReview
            .preview(&testing::shopper(), input())
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        let mut bad = input();
        bad.review_rating = Some(7);
        let e = UpdateReview
            .preview(&testing::shopper(), bad)
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn only_the_writer_may_change_a_review() {
        // rev_8812 was imported (no author): nobody may change it through the API.
        let cx = testing::shopper_db().await;
        let e = UpdateReview
            .preview(
                &cx,
                UpdateReviewInput {
                    review_id: "rev_8812".into(),
                    review_rating: Some(4),
                    review_body: None,
                    image: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "not_author");
        let e = UpdateReview
            .preview(
                &cx,
                UpdateReviewInput {
                    review_id: "rev_nothing".into(),
                    review_rating: Some(4),
                    review_body: None,
                    image: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "review_not_found");
    }
}
