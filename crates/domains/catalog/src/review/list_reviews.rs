//! `list_reviews` (docs/api/catalog.md#review).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::ReviewStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::plain;
use crate::repo::{self, Review, ReviewFilter};
use crate::{INVALID_FILTER, is_operator, list};

/// How `list_reviews` orders reviews.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSort {
    /// Latest first (the default).
    Newest,
    /// Highest rating first, then latest.
    Rating,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListReviewsInput {
    /// Only reviews of this product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_wool_overcoat")]
    pub product_id: Option<String>,
    /// Only reviews with this many stars, 1–5.
    #[schemars(range(min = 1, max = 5), example = 5)]
    pub rating: Option<u8>,
    /// Only reviews in this moderation state. Shoppers never see hidden reviews.
    #[schemars(example = &"visible")]
    pub status: Option<ReviewStatus>,
    /// Order of the results.
    #[schemars(example = &"newest")]
    pub sort: Option<ReviewSort>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// Reviews with rating, author and text.
#[derive(Debug)]
pub struct ListReviews;

impl Operation for ListReviews {
    const META: OpMeta = OpMeta {
        address: "catalog.review.list",
        tool: "list_reviews",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/reviews",
        summary: "Reviews with their rating, author, text, fit vote, size bought and helpful votes; by product, rating and moderation state.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("review_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListReviewsInput;
    type Output = Value;
}

fn review_json(r: &Review) -> Value {
    json!({
        "id": r.id, "@type": "Review", "product_id": r.product_group_id, "variant_id": r.product_id,
        "review_rating": r.rating_value, "author": {"@type": "Person", "name": r.author_name},
        "review_body": r.review_body, "fit": r.fit_vote,
        "height_cm": r.author_height_value.map(plain), "size_bought": r.size,
        "image": r.image, "mentions": r.keywords, "status": r.creative_work_status,
        "helpful_count": r.helpful_count, "date_published": r.date_published.to_string(),
    })
}

impl ReadOp<Cx> for ListReviews {
    async fn read(&self, cx: &Cx, input: ListReviewsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        if input.rating.is_some_and(|r| !(1..=5).contains(&r)) {
            return Err(ToolError::new(INVALID_FILTER, "`rating` is 1–5.")
                .detail(json!({"field": "rating", "reason": "range"}))
                .next("list_reviews"));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (rows, total) = repo::reviews(
            &mut tx,
            &ReviewFilter {
                product_id: input.product_id.as_deref(),
                rating: input.rating.map(i16::from),
                status: input.status.map(|s| s.as_str()),
                hide_hidden: !is_operator(&cx.principal),
                by_rating: input.sort == Some(ReviewSort::Rating),
                limit: page.limit,
                offset: page.offset,
            },
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(list(rows.iter().map(review_json).collect(), total, &page))
    }
}
