//! `create_review` (docs/api/catalog.md#review).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::FitVote;
use acom_store::{Cx, TxCx};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::shopper_id;
use crate::model::{self};
use crate::neighbors::{self, PurchasedItem};
use crate::repo::{self, NewReview};
use crate::rules::ReviewRules;
use crate::{
    ALREADY_REVIEWED, NOT_PURCHASED, PRODUCT_NOT_FOUND, PROFANITY_BLOCKED, product_not_found,
    visible_group,
};

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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(variant: &str, status: &str) -> PurchasedItem {
        PurchasedItem {
            order_item_id: "oln_20268_1".into(),
            variant_id: variant.into(),
            order_item_status: status.into(),
        }
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
}
