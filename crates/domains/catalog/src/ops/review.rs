//! `get_review_summary`, `list_reviews`, `create_review`, `vote_review_helpful`
//! (docs/api/catalog.md#review).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, ReadOp, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_core::enums::{FitVote, ReviewStatus};
use acom_store::{Cx, Page, TxCx};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::product::visible_group;
use super::{not_found, product_not_found};
use crate::model::{self, plain};
use crate::neighbors::{self, PurchasedItem};
use crate::repo::{self, NewReview, Review, ReviewFilter};
use crate::rules::ReviewRules;
use crate::{
    ALREADY_REVIEWED, INVALID_FILTER, NO_REVIEWS, NOT_PURCHASED, PRODUCT_NOT_FOUND,
    PROFANITY_BLOCKED, REVIEW_NOT_FOUND, is_operator, list,
};

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

/// The signed-in shopper writing or voting; guests must sign in first.
fn shopper_id(cx: &Cx, what: &str) -> Result<String, ToolError> {
    cx.principal
        .customer_id()
        .map(str::to_owned)
        .ok_or_else(|| {
            ToolError::new(ErrorCode::FORBIDDEN, format!("Sign in to {what}."))
                .detail(json!({"reason": "owner"}))
        })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateReviewInput {
    /// The product bought.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_windbreaker")]
    pub product_id: String,
    /// The order it was bought in (`get_my_orders`).
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20268")]
    pub order_id: String,
    /// Stars, 1 (worst) to 5 (best).
    #[schemars(range(min = 1, max = 5), example = 5)]
    pub review_rating: u8,
    /// The review text.
    #[schemars(length(min = 1, max = 4000), example = &"Packs into its pocket and kept the drizzle off.")]
    pub review_body: String,
    /// Photo URLs.
    #[schemars(example = ["https://cdn.northwind.example/r/wb_jamie.webp"])]
    pub image: Option<Vec<String>>,
    /// How the size fit.
    #[schemars(example = &"true_to_size")]
    pub fit: Option<FitVote>,
    /// The reviewer's height in cm, 100–230.
    #[schemars(range(min = 100, max = 230), example = 178)]
    pub height_cm: Option<u32>,
}

/// What `create_review` runs, with what its checks found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateReviewPlan {
    pub review_id: String,
    pub product_id: String,
    pub variant_id: Option<String>,
    pub order_id: String,
    pub author_id: String,
    pub author_name: String,
    pub rating: i16,
    pub body: String,
    pub fit: Option<String>,
    pub height_cm: Option<u32>,
    pub image: Vec<String>,
    pub keywords: Vec<String>,
    pub status: String,
    pub date_published: NaiveDate,
    /// A delivered purchase was found, or the store does not require one.
    pub purchase_ok: bool,
    /// The blocked word found, when the store blocks such reviews.
    pub blocked_word: Option<String>,
}

/// A shopper reviews a product they bought.
#[derive(Debug)]
pub struct CreateReview;

impl Operation for CreateReview {
    const META: OpMeta = OpMeta {
        address: "catalog.review.create",
        tool: "create_review",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/reviews",
        summary: "The signed-in shopper reviews a product they received: stars, text, photos, fit and height. Shown right away and moderated afterwards.",
        consumes: &[Slot::one("product_id"), Slot::one("order_id")],
        produces: &[Slot::one("review_id")],
        checks: &[
            "review.verified_purchase_only",
            "review.one_per_order_item",
            "review.profanity_filter",
        ],
        errors: &[
            NOT_PURCHASED,
            ALREADY_REVIEWED,
            PROFANITY_BLOCKED,
            PRODUCT_NOT_FOUND,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateReviewInput;
    type Output = Value;
}

/// The review checks, from what the store knows about the shopper's order.
pub(crate) fn review_checks(
    rules: &ReviewRules,
    bought: &[&PurchasedItem],
    existing: Option<&str>,
    blocked: Option<&str>,
) -> Vec<Check> {
    let delivered = bought.iter().any(|i| i.order_item_status == "delivered");
    let purchase = Check {
        rule: "review.verified_purchase_only".into(),
        passed: delivered || !rules.verified_purchase_only,
        detail: Some(match (delivered, bought.is_empty()) {
            (true, _) => "Bought and delivered in this order".into(),
            (false, true) => "This order has no such product for you".into(),
            (false, false) => "Not delivered yet; review once it arrives".into(),
        }),
        fix_tool: (!delivered && rules.verified_purchase_only).then(|| "get_my_orders".into()),
    };
    let duplicate = existing.is_some() && rules.one_per_order_item;
    let once = Check {
        rule: "review.one_per_order_item".into(),
        passed: !duplicate,
        detail: Some(match existing {
            Some(id) => format!("Already reviewed in {id}"),
            None => "First review of this product from this order".into(),
        }),
        fix_tool: duplicate.then(|| "update_review".into()),
    };
    let blocks = blocked.is_some() && rules.profanity_filter == "block";
    let words = Check {
        rule: "review.profanity_filter".into(),
        passed: !blocks,
        detail: Some(match blocked {
            None => "No blocked words".into(),
            Some(w) if blocks => format!("Uses a blocked word ({w})"),
            Some(_) => "Uses a blocked word; it will be held for review".into(),
        }),
        fix_tool: blocks.then(|| "create_review".into()),
    };
    vec![purchase, once, words]
}

impl WriteOp<Cx> for CreateReview {
    type Plan = CreateReviewPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateReviewInput,
    ) -> Result<Planned<CreateReviewPlan>, ToolError> {
        let author = shopper_id(cx, "write a review")?;
        let body = input.review_body.trim().to_owned();
        if body.is_empty() {
            return Err(ToolError::invalid_input("The review text is empty.")
                .detail(json!({"field": "review_body", "reason": "missing"})));
        }
        if !(1..=5).contains(&input.review_rating) {
            return Err(ToolError::invalid_input("`review_rating` is 1–5.")
                .detail(json!({"field": "review_rating", "reason": "range"})));
        }
        if input.height_cm.is_some_and(|h| !(100..=230).contains(&h)) {
            return Err(ToolError::invalid_input("`height_cm` is 100–230.")
                .detail(json!({"field": "height_cm", "reason": "range"})));
        }
        let image: Vec<String> = input
            .image
            .unwrap_or_default()
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
        let rules = ReviewRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let Some(group) = visible_group(cx, &mut tx, &input.product_id).await? else {
            return Err(product_not_found(
                "product",
                &input.product_id,
                "search_products",
            ));
        };
        let variants = repo::variants(&mut tx, std::slice::from_ref(&group.id)).await?;
        let items = neighbors::purchased_items(&mut tx, &author, &input.order_id).await?;
        let existing = repo::review_by(&mut tx, &author, &input.order_id, &group.id).await?;
        let known = repo::known_mentions(&mut tx, &group.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut bought: Vec<&PurchasedItem> = items
            .iter()
            .filter(|i| variants.iter().any(|v| v.id == i.variant_id))
            .collect();
        bought.sort_by_key(|i| i.order_item_status != "delivered");
        let blocked = model::blocked_word(&body, &rules.blocked_words);
        let checks = review_checks(&rules, &bought, existing.as_deref(), blocked.as_deref());
        let held = blocked.is_some() && rules.profanity_filter != "block";
        let plan = CreateReviewPlan {
            review_id: format!("rev_{}", acom_core::id::new_suffix()),
            product_id: group.id.clone(),
            variant_id: bought.first().map(|i| i.variant_id.clone()),
            order_id: input.order_id,
            author_id: author,
            author_name: cx
                .principal
                .name
                .clone()
                .unwrap_or_else(|| "A shopper".into()),
            rating: i16::from(input.review_rating),
            keywords: model::mentions_in(&body, &known),
            body,
            fit: input.fit.map(|f| f.as_str().to_owned()),
            height_cm: input.height_cm,
            image,
            status: if held { "reported" } else { "visible" }.into(),
            date_published: cx.clock.now().with_timezone(&cx.time.zone).date_naive(),
            purchase_ok: checks[0].passed,
            blocked_word: blocked.filter(|_| !held),
        };
        let size = bought
            .first()
            .and_then(|i| variants.iter().find(|v| v.id == i.variant_id))
            .and_then(|v| v.size.clone());
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "review".into(),
                id: plan.review_id.clone(),
                from: Value::Null,
                to: json!({"product_id": plan.product_id, "order_id": plan.order_id,
                           "review_rating": plan.rating, "review_body": plan.body,
                           "fit": plan.fit, "height_cm": plan.height_cm, "size_bought": size,
                           "image": plan.image, "status": plan.status}),
                note: None,
            }],
            checks,
            basis: vec![Basis {
                entity: "catalog.product_group".into(),
                id: group.id,
                version: i64::from(group.version),
            }],
            approval: match approval.reason {
                Some(reason) if approval.needed => ApprovalNeed::Needed { reason },
                _ => ApprovalNeed::NotNeeded,
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &CreateReviewPlan) -> Result<Committed, ToolError> {
        if let Some(w) = &plan.blocked_word {
            return Err(ToolError::new(
                PROFANITY_BLOCKED,
                format!("The review uses a word this store does not allow ({w})."),
            )
            .detail(json!({"rule": "review.profanity_filter"}))
            .next("create_review"));
        }
        if !plan.purchase_ok {
            return Err(ToolError::new(
                NOT_PURCHASED,
                "Only shoppers who received the product in this order can review it.",
            )
            .detail(json!({"rule": "review.verified_purchase_only", "order_id": plan.order_id}))
            .next("get_my_orders"));
        }
        let rules = ReviewRules::from_policy(&tx.policy)?;
        if rules.one_per_order_item
            && let Some(id) =
                repo::review_by(tx, &plan.author_id, &plan.order_id, &plan.product_id).await?
            && id != plan.review_id
        {
            return Err(ToolError::new(
                ALREADY_REVIEWED,
                format!("You already reviewed this product for this order ({id})."),
            )
            .detail(json!({"entity": "review", "existing_id": id}))
            .next("update_review"));
        }
        repo::insert_review(
            tx,
            &NewReview {
                id: &plan.review_id,
                group_id: &plan.product_id,
                variant_id: plan.variant_id.as_deref(),
                author_id: &plan.author_id,
                author_name: &plan.author_name,
                order_id: &plan.order_id,
                rating: plan.rating,
                body: &plan.body,
                fit: plan.fit.as_deref(),
                height_cm: plan.height_cm.map(Decimal::from),
                keywords: &plan.keywords,
                image: &plan.image,
                status: &plan.status,
                date_published: plan.date_published,
            },
        )
        .await?;
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
        plan: &CreateReviewPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::review_written(&mut tx, &plan.review_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = row.is_some_and(|(rating, body, product)| {
            rating == plan.rating && body == plan.body && product == plan.product_id
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} does not read back as written", plan.review_id)]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &CreateReviewPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} is the shopper's own review; they remove it with delete_review.",
                plan.review_id
            ),
        ))
    }
}

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
    use crate::ops::testing;

    fn item(variant: &str, status: &str) -> PurchasedItem {
        PurchasedItem {
            order_item_id: "oln_20268_1".into(),
            variant_id: variant.into(),
            order_item_status: status.into(),
        }
    }

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

    #[test]
    fn reviews_need_a_delivered_purchase_and_one_per_order() {
        let rules =
            ReviewRules::from_policy(&crate::testing::policies()).unwrap_or_else(|e| panic!("{e}"));
        let delivered = item("var_windbreaker_m_black", "delivered");
        let checks = review_checks(&rules, &[&delivered], None, None);
        assert!(checks.iter().all(|c| c.passed), "{checks:?}");
        let shipping = item("var_windbreaker_m_black", "in_transit");
        let checks = review_checks(&rules, &[&shipping], Some("rev_1"), Some("junk"));
        assert_eq!(
            checks.iter().map(|c| c.passed).collect::<Vec<_>>(),
            [false, false, false]
        );
        assert_eq!(checks[0].fix_tool.as_deref(), Some("get_my_orders"));
        assert_eq!(checks[1].fix_tool.as_deref(), Some("update_review"));
    }

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
