//! `get_review_summary` (docs/api/catalog.md#review).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::Cx;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{self};
use crate::repo::{self};
use crate::{NO_REVIEWS, not_found, visible_group};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetReviewSummaryInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_wool_overcoat")]
    pub product_id: String,
    /// Only reviewers at least this tall, in cm (inclusive). Give it with `height_max_cm`.
    #[schemars(range(min = 100, max = 230), example = 175)]
    pub height_min_cm: Option<u32>,
    /// Only reviewers at most this tall, in cm (inclusive).
    #[schemars(range(min = 100, max = 230), example = 182)]
    pub height_max_cm: Option<u32>,
    /// Only reviewers who bought this size.
    #[schemars(length(max = 10), example = &"M")]
    pub size: Option<String>,
    /// How many reviews to return as evidence, 0–5. Default 3.
    #[schemars(range(min = 0, max = 5), example = 3)]
    pub sample_limit: Option<u32>,
}

/// Fit votes, repeated phrases and a few reviews, for fit questions.
#[derive(Debug)]
pub struct GetReviewSummary;

impl Operation for GetReviewSummary {
    const META: OpMeta = OpMeta {
        address: "catalog.review.summarize",
        tool: "get_review_summary",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/products/{product_id}/reviews/summary",
        summary: "Reviews of a product summed up: rating, fit votes (true to size, runs small, runs large), phrases reviewers repeat, and a few reviews as evidence; optionally only reviewers of a height range or size. Hidden reviews are not counted.",
        consumes: &[Slot::one("product_id")],
        produces: &[],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND, NO_REVIEWS],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetReviewSummaryInput;
    type Output = Value;
}

fn input_error(field: &str, message: &str) -> ToolError {
    ToolError::invalid_input(message)
        .detail(json!({"field": field, "reason": "range"}))
        .next("get_review_summary")
}

/// The height range: both ends or neither, low ≤ high, within 100–230 cm.
fn height_range(min: Option<u32>, max: Option<u32>) -> Result<Option<(u32, u32)>, ToolError> {
    match (min, max) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(input_error(
            "height_min_cm",
            "Give both `height_min_cm` and `height_max_cm`, or neither.",
        )),
        (Some(lo), Some(hi)) if lo > hi => Err(input_error(
            "height_min_cm",
            "`height_min_cm` is above `height_max_cm`.",
        )),
        (Some(lo), Some(hi)) if !(100..=230).contains(&lo) || !(100..=230).contains(&hi) => {
            Err(input_error("height_min_cm", "Heights are 100–230 cm."))
        }
        (Some(lo), Some(hi)) => Ok(Some((lo, hi))),
    }
}

impl ReadOp<Cx> for GetReviewSummary {
    async fn read(&self, cx: &Cx, input: GetReviewSummaryInput) -> Result<Value, ToolError> {
        let heights = height_range(input.height_min_cm, input.height_max_cm)?;
        let sample = input.sample_limit.unwrap_or(3);
        if sample > 5 {
            return Err(input_error("sample_limit", "`sample_limit` is 0–5."));
        }
        let size = input
            .size
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = visible_group(cx, &mut tx, &input.product_id).await?;
        let reviews = match &group {
            Some(g) => {
                let h = heights.map(|(lo, hi)| (Decimal::from(lo), Decimal::from(hi)));
                repo::review_facts(&mut tx, &g.id, h, size).await?
            }
            None => Vec::new(),
        };
        tx.commit().await.map_err(ToolError::from)?;
        if group.is_none() {
            return Err(not_found("product", &input.product_id, "search_products"));
        }
        let filter = json!({"height_min_cm": heights.map(|h| h.0),
                            "height_max_cm": heights.map(|h| h.1), "size": size});
        if reviews.is_empty() {
            return Err(ToolError::new(
                NO_REVIEWS,
                "No reviews match these conditions; widen the height range or size.",
            )
            .detail(json!({"product_id": input.product_id, "filter": filter}))
            .next("get_product_measurements"));
        }
        let mut out = json!({"product_id": input.product_id, "filter": filter});
        if let (Value::Object(o), Value::Object(s)) =
            (&mut out, model::review_summary(&reviews, sample as usize))
        {
            o.extend(s);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::list_reviews::{ListReviews, ListReviewsInput};
    use crate::testing;

    #[test]
    fn heights_come_in_pairs_and_in_order() {
        assert_eq!(
            height_range(Some(175), Some(182)).ok(),
            Some(Some((175, 182)))
        );
        assert!(height_range(Some(175), None).is_err());
        assert!(height_range(Some(190), Some(180)).is_err());
        assert_eq!(height_range(None, None).ok(), Some(None));
    }

    #[tokio::test]
    async fn a_half_height_range_is_refused() {
        let e = GetReviewSummary
            .read(
                &testing::guest(),
                GetReviewSummaryInput {
                    product_id: "prd_wool_overcoat".into(),
                    height_min_cm: Some(175),
                    height_max_cm: None,
                    size: None,
                    sample_limit: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        let e = ListReviews
            .read(
                &testing::guest(),
                ListReviewsInput {
                    product_id: None,
                    rating: Some(9),
                    status: None,
                    sort: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_filter");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn overcoat_summary_matches_the_fit_check() {
        let cx = testing::shopper_db().await;
        let s = GetReviewSummary
            .read(
                &cx,
                GetReviewSummaryInput {
                    product_id: "prd_wool_overcoat".into(),
                    height_min_cm: Some(175),
                    height_max_cm: Some(182),
                    size: None,
                    sample_limit: Some(3),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s["review_count"], json!(41));
        assert_eq!(s["rating_value"].to_string(), "4.6");
        assert_eq!(s["fit_votes"][1]["label"], "A bit snug");
        assert_eq!(
            s["mentions"][0],
            json!({"text": "size up for a hoodie", "count": 9})
        );
        assert_eq!(s["sample"][0]["review_id"], "rev_8812");
        let list = ListReviews
            .read(
                &cx,
                ListReviewsInput {
                    product_id: Some("prd_wool_overcoat".into()),
                    rating: None,
                    status: None,
                    sort: None,
                    limit: Some(5),
                    cursor: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(list["total"], json!(41));
        assert_eq!(list["next_cursor"], "o5");
    }
}
