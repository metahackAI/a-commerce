//! The `catalog` domain: products, variants, offers, inventory, brands, reviews, questions.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/catalog.md`.
//! Layout: the folders follow the API hierarchy `catalog.{resource}.{operation}`: one folder per
//! resource (`product`, `offer`, `review`, …) and one file per operation, named after its tool
//! (`review/create_review.rs`); what a resource's operations share is in its `mod.rs`.
//! Domain-wide code: `model` (pure business logic: availability, prices, floors, stock rules,
//! review summaries), `rules` (the catalog policy values the model needs), `repo` and `write`
//! (SQL). Other domains are reached only through their service functions, never through their
//! tables; the ones above catalog in the crate graph come in through [`neighbors`].

pub mod import;
pub mod model;
pub mod neighbors;
pub mod rules;
pub mod service;

mod cards;
mod repo;
mod viewer;
mod watch;
mod write;

mod approval;
mod brand;
mod bundle;
mod inventory;
mod offer;
mod product;
mod product_group;
mod question;
mod reorder;
mod restock_alert;
mod review;

use acom_auth::Principal;
use acom_contract::{ErrorClass, ErrorCode, RegistryBuilder, ToolError};
use acom_core::enums::ActorKind;
use acom_store::{Cx, StoreError, TxCx};
use serde_json::{Value, json};

pub use viewer::Viewer;

/// No product (or variant) with that id in this store.
pub const PRODUCT_NOT_FOUND: ErrorCode = ErrorCode::new("product_not_found", ErrorClass::NotFound);
/// No brand with that id.
pub const BRAND_NOT_FOUND: ErrorCode = ErrorCode::new("brand_not_found", ErrorClass::NotFound);
/// No review with that id.
pub const REVIEW_NOT_FOUND: ErrorCode = ErrorCode::new("review_not_found", ErrorClass::NotFound);
/// No member tier with that id.
pub const TIER_NOT_FOUND: ErrorCode = ErrorCode::new("tier_not_found", ErrorClass::NotFound);
/// A size the product's measurement chart does not have.
pub const SIZE_NOT_FOUND: ErrorCode = ErrorCode::new("size_not_found", ErrorClass::NotFound);
/// A filter value that cannot be used.
pub const INVALID_FILTER: ErrorCode = ErrorCode::new("invalid_filter", ErrorClass::InvalidInput);
/// A price under the floor (cost × multiplier).
pub const BELOW_PRICE_FLOOR: ErrorCode =
    ErrorCode::new("below_price_floor", ErrorClass::PolicyViolation);
/// A stock level under zero.
pub const NEGATIVE_STOCK: ErrorCode = ErrorCode::new("negative_stock", ErrorClass::InvalidInput);
/// A variant with running subscriptions cannot be discontinued.
pub const HAS_OPEN_SUBSCRIPTIONS: ErrorCode =
    ErrorCode::new("has_open_subscriptions", ErrorClass::Domain);
/// No review matches the summary's conditions.
pub const NO_REVIEWS: ErrorCode = ErrorCode::new("no_reviews", ErrorClass::Domain);
/// The shopper has no delivered purchase of the product in that order.
pub const NOT_PURCHASED: ErrorCode = ErrorCode::new("not_purchased", ErrorClass::PolicyViolation);
/// The shopper already reviewed the product for that order.
pub const ALREADY_REVIEWED: ErrorCode =
    ErrorCode::new("already_reviewed", ErrorClass::AlreadyExists);
/// The review text has a word the store blocks.
pub const PROFANITY_BLOCKED: ErrorCode =
    ErrorCode::new("profanity_blocked", ErrorClass::PolicyViolation);
/// A supplier sheet that cannot be read: no rows, too many, or a required column missing.
pub const INVALID_FILE_FORMAT: ErrorCode =
    ErrorCode::new("invalid_file_format", ErrorClass::InvalidInput);
/// A SKU already used in the store.
pub const DUPLICATE_SKU: ErrorCode = ErrorCode::new("duplicate_sku", ErrorClass::AlreadyExists);
/// Only the shopper who wrote a review or a question may change or delete it.
pub const NOT_AUTHOR: ErrorCode = ErrorCode::new("not_author", ErrorClass::Forbidden);
/// The shopper already reported this review.
pub const ALREADY_REPORTED: ErrorCode =
    ErrorCode::new("already_reported", ErrorClass::AlreadyExists);
/// The question has its answer already (one per question; the asker can no longer edit it).
pub const ALREADY_ANSWERED: ErrorCode =
    ErrorCode::new("already_answered", ErrorClass::InvalidState);
/// The shopper already waits for this variant on this channel.
pub const ALREADY_SUBSCRIBED: ErrorCode =
    ErrorCode::new("already_subscribed", ErrorClass::AlreadyExists);
/// The variant can be bought now; there is nothing to wait for.
pub const ALREADY_IN_STOCK: ErrorCode =
    ErrorCode::new("already_in_stock", ErrorClass::InvalidState);
/// A bundle component that is not on sale (not active, discontinued, or without a price).
pub const COMPONENT_INACTIVE: ErrorCode =
    ErrorCode::new("component_inactive", ErrorClass::InvalidState);
/// Bundle components sold by different sellers.
pub const MIXED_SELLER: ErrorCode = ErrorCode::new("mixed_seller", ErrorClass::Domain);
/// Bundle components with different minimum ages.
pub const MIXED_AGE_RESTRICTION: ErrorCode =
    ErrorCode::new("mixed_age_restriction", ErrorClass::Domain);
/// The content generator could not make the draft.
pub const GENERATION_FAILED: ErrorCode =
    ErrorCode::new("generation_failed", ErrorClass::UpstreamUnavailable);
/// A generation prompt the store's brand guideline does not allow.
pub const BRAND_GUIDELINE_VIOLATION: ErrorCode =
    ErrorCode::new("brand_guideline_violation", ErrorClass::Domain);

/// Lowercase words joined by `_`, for ids made from names and codes:
/// `FW26-OT-014-CHR` → `fw26_ot_014_chr`, `Wool blend overcoat` → `wool_blend_overcoat`.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() { "x".into() } else { out }
}

/// Adds every operation of this domain to the registry.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    // product_group
    b.read(product_group::search_product_groups::SearchProductGroups)
        .read(product_group::get_product_group::GetProductGroup)
        .write(product_group::create_product_group::CreateProductGroup)
        .write(product_group::update_product_group::UpdateProductGroup);
    // product
    b.read(product::get_product::GetProduct)
        .read(product::get_product_measurements::GetProductMeasurements)
        .write(product::create_product::CreateProduct)
        .write(product::update_product::UpdateProduct)
        .write(product::set_variants::SetVariants)
        .write(product::discontinue_product::DiscontinueProduct)
        .read(product::get_return_policy::GetReturnPolicy)
        .read(product::get_delivery_policy::GetDeliveryPolicy)
        .write(product::batch_import::BatchImport)
        .write(product::generate_image::GenerateImage)
        .write(product::generate_copy::GenerateCopy);
    // offer
    b.read(offer::get_price_breakdown::GetPriceBreakdown)
        .write(offer::set_price::SetPrice)
        .write(offer::schedule_price_change::SchedulePriceChange);
    // inventory
    b.read(inventory::get_inventory::GetInventory)
        .write(inventory::set_variant_stock::SetVariantStock)
        .write(inventory::set_safety_stock::SetSafetyStock);
    // reorder
    b.read(reorder::suggest_reorder::SuggestReorder)
        .write(reorder::create_reorder::CreateReorder)
        .write(reorder::receive_reorder::ReceiveReorder);
    // approval
    b.write(approval::submit_for_review::SubmitForReview)
        .write(approval::approve_product::ApproveProduct)
        .write(approval::reject_product::RejectProduct)
        .read(approval::get_change_history::GetChangeHistory);
    // brand
    b.read(brand::list_brands::ListBrands)
        .read(brand::get_brand::GetBrand)
        .write(brand::create_brand::CreateBrand)
        .write(brand::update_brand::UpdateBrand);
    // review
    b.read(review::get_review_summary::GetReviewSummary)
        .read(review::list_reviews::ListReviews)
        .write(review::moderate_review::ModerateReview)
        .write(review::create_review::CreateReview)
        .write(review::update_review::UpdateReview)
        .write(review::delete_review::DeleteReview)
        .write(review::vote_review_helpful::VoteReviewHelpful)
        .write(review::report_review::ReportReview);
    // question
    b.read(question::list_questions::ListQuestions)
        .write(question::answer_question::AnswerQuestion)
        .write(question::moderate_question::ModerateQuestion)
        .write(question::ask_question::AskQuestion)
        .write(question::update_question::UpdateQuestion)
        .write(question::delete_question::DeleteQuestion);
    // bundle
    b.write(bundle::create_bundle::CreateBundle);
    // restock_alert
    b.write(restock_alert::subscribe_restock::SubscribeRestock)
        .read(restock_alert::list_restock_subscriptions::ListRestockSubscriptions);
    // Scheduled work (platform.job): the watcher over prices under cost.
    b.job(watch::PriceBelowCost);
}

/// `product_not_found` for a product (`prd_`) or a variant (`var_`).
pub(crate) fn product_not_found(entity: &str, id: &str, next: &'static str) -> ToolError {
    let what = if entity == "variant" {
        "variant"
    } else {
        "product"
    };
    ToolError::new(
        PRODUCT_NOT_FOUND,
        format!("There is no {what} {id} in this store."),
    )
    .detail(json!({"entity": entity, "id": id}))
    .next(next)
}

/// The spec's plain `not_found` with the entity named in `detail`.
pub(crate) fn not_found(entity: &str, id: &str, next: &'static str) -> ToolError {
    ToolError::new(
        ErrorCode::NOT_FOUND,
        format!(
            "There is no {} {id} in this store.",
            entity.replace('_', " ")
        ),
    )
    .detail(json!({"entity": entity, "id": id}))
    .next(next)
}

/// A `*.profanity_filter` check: passed without a blocked word; with one, failed when the store
/// blocks such text (`block`) and passed with a note when it holds it for review.
pub(crate) fn blocked_words_check(
    rule: &str,
    blocks: bool,
    found: Option<&str>,
    fix_tool: &str,
) -> acom_contract::Check {
    let refused = found.is_some() && blocks;
    acom_contract::Check {
        rule: rule.into(),
        passed: !refused,
        detail: Some(match found {
            None => "No blocked words".into(),
            Some(w) if blocks => format!("Uses a blocked word ({w})"),
            Some(_) => "Uses a blocked word; it will be held for review".into(),
        }),
        fix_tool: refused.then(|| fix_tool.into()),
    }
}

/// `profanity_blocked`: the text uses a word the store does not allow.
pub(crate) fn profanity_blocked(rule: &str, word: &str, next: &'static str) -> ToolError {
    ToolError::new(
        PROFANITY_BLOCKED,
        format!("The text uses a word this store does not allow ({word})."),
    )
    .detail(json!({"rule": rule, "word": word}))
    .next(next)
}

/// The signed-in shopper making a shopper-only write; guests must sign in first.
pub(crate) fn signed_in_shopper(cx: &Cx, what: &str) -> Result<String, ToolError> {
    cx.principal
        .customer_id()
        .map(str::to_owned)
        .ok_or_else(|| {
            ToolError::new(ErrorCode::FORBIDDEN, format!("Sign in to {what}."))
                .detail(json!({"reason": "owner"}))
        })
}

/// The name shown with what a shopper writes, masked as `author_masking` says.
pub(crate) fn shown_author(cx: &Cx) -> Result<String, ToolError> {
    let masking = rules::AuthorMasking::from_policy(&cx.policy)?;
    let name = cx.principal.name.as_deref().unwrap_or("").trim();
    Ok(if name.is_empty() {
        "A shopper".into()
    } else {
        masking.shown(name)
    })
}

/// A write only the store may run (moderating questions, generating content): refused for shoppers.
pub(crate) fn operators_only(cx: &Cx, what: &str) -> Result<(), ToolError> {
    if is_operator(&cx.principal) {
        Ok(())
    } else {
        Err(
            ToolError::new(ErrorCode::FORBIDDEN, format!("Only the store can {what}."))
                .detail(json!({"reason": "role"})),
        )
    }
}

/// `not_author`: someone other than the writer tries to change or delete it.
pub(crate) fn not_author(entity: &str, id: &str, next: &'static str) -> ToolError {
    ToolError::new(
        NOT_AUTHOR,
        format!(
            "Only the shopper who wrote {id} can change or delete this {}.",
            entity
        ),
    )
    .detail(json!({"reason": "owner", "entity": entity, "id": id}))
    .next(next)
}

/// A product the caller may see: shoppers only see active ones.
pub(crate) async fn visible_group(
    cx: &Cx,
    tx: &mut TxCx,
    product_id: &str,
) -> Result<Option<repo::Group>, ToolError> {
    Ok(repo::group(tx, product_id)
        .await?
        .filter(|g| is_operator(&cx.principal) || g.approval_status == "active"))
}

pub(crate) fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// Whether the caller works for the store: an operator, our agent acting for one, or the
/// system. Only they see costs and may ask for another tier's prices
/// (`catalog.yml pricing.cost_visibility: operators_only`).
pub fn is_operator(p: &Principal) -> bool {
    match p.kind {
        ActorKind::Operator | ActorKind::System => true,
        ActorKind::AgentOnBehalf => p.subject.starts_with("adm_"),
        ActorKind::Shopper | ActorKind::ExternalAgent => false,
    }
}

/// A time input in store time.
pub(crate) fn parse_time(
    cx: &Cx,
    field: &str,
    s: Option<&str>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, ToolError> {
    s.map(|s| {
        cx.time.parse(s).map_err(|e| {
            ToolError::invalid_input(e.to_string())
                .detail(json!({"field": field, "reason": "format"}))
        })
    })
    .transpose()
}

/// Whether `approval.yml` asks for a person on this write (`thresholds.*` whose `applies_to`
/// names the tool and whose `when` holds for `facts`). Writes with no threshold run without one.
pub(crate) fn policy_approval(cx: &Cx, tool: &str, facts: &Value) -> acom_contract::ApprovalNeed {
    let a = cx.policy.approval_for(tool, facts);
    match a.reason {
        Some(reason) if a.needed => acom_contract::ApprovalNeed::Needed { reason },
        _ if a.needed => acom_contract::ApprovalNeed::Needed {
            reason: format!("The store asks a person to approve {tool}"),
        },
        _ => acom_contract::ApprovalNeed::NotNeeded,
    }
}

/// An operator-only input sent by someone else.
pub(crate) fn operator_only(field: &str) -> ToolError {
    ToolError::new(
        ErrorCode::FORBIDDEN,
        format!("Only store operators can set `{field}`."),
    )
    .detail(json!({"reason": "scope", "field": field}))
}

/// A list envelope `{items, total, next_cursor?}`.
pub(crate) fn list(items: Vec<Value>, total: i64, page: &acom_store::Page) -> Value {
    let returned = items.len();
    let mut out = json!({"items": items, "total": total});
    if let Some(c) = page.next_cursor(returned, total) {
        out["next_cursor"] = json!(c);
    }
    out
}

#[cfg(test)]
pub(crate) mod testing {
    use acom_auth::{Principal, Surface, scopes};
    use acom_core::enums::ActorKind;
    use acom_core::{FixedClock, StoreTime, TenantId};
    use acom_policy::{FILES, PolicySet};
    use acom_store::{Cx, Db};
    use std::sync::Arc;
    /// The default policies in `policies/`.
    pub fn policies() -> PolicySet {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
        let texts: Vec<(&str, String)> = FILES
            .iter()
            .map(|f| {
                (
                    *f,
                    std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap_or_default(),
                )
            })
            .collect();
        PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str())))
            .unwrap_or_else(|e| panic!("policies: {e}"))
    }

    /// A context whose database is never reached: for checks that fail before any query.
    pub fn cx(kind: ActorKind, subject: &str) -> Cx {
        let tenant = TenantId::parse("store_northwind").unwrap_or_else(|e| panic!("{e}"));
        let now = StoreTime::new(chrono_tz::America::New_York)
            .parse("2026-11-07T10:00")
            .unwrap_or_else(|e| panic!("{e}"));
        Cx {
            principal: Arc::new(Principal {
                subject: subject.into(),
                kind,
                tenant: tenant.clone(),
                surface: Surface::Fo,
                scopes: scopes::shopper(),
                name: Some("Jamie".into()),
                session_id: None,
            }),
            tenant,
            request_id: "req_test".into(),
            clock: Arc::new(FixedClock::at(now)),
            time: StoreTime::new(chrono_tz::America::New_York),
            currency: acom_core::Currency::parse("USD").unwrap_or_else(|e| panic!("{e}")),
            policy: Arc::new(crate::testing::policies()),
            db: Db::connect_lazy("postgres://nobody@127.0.0.1:1/none", 1)
                .unwrap_or_else(|e| panic!("{e}")),
        }
    }

    /// Jamie, a signed-in VIP shopper.
    pub fn shopper() -> Cx {
        cx(ActorKind::Shopper, "cus_jamie")
    }

    /// A guest without an account.
    pub fn guest() -> Cx {
        cx(ActorKind::Shopper, "ses_guest")
    }

    /// A context on a real database (`DATABASE_URL`) as the demo store's operator SJ, for the
    /// integration tests (`cargo test -- --ignored`).
    pub async fn operator_db() -> Cx {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| panic!("DATABASE_URL"));
        let mut cx = cx(ActorKind::Operator, "adm_sj");
        cx.db = Db::connect(&url, 2).await.unwrap_or_else(|e| panic!("{e}"));
        let mut p = (*cx.principal).clone();
        p.surface = Surface::Bo;
        p.scopes = scopes::admin();
        cx.principal = Arc::new(p);
        let now = cx
            .time
            .parse("2026-11-06T11:00")
            .unwrap_or_else(|e| panic!("{e}"));
        cx.clock = Arc::new(FixedClock::at(now));
        cx
    }

    /// The same, as Jamie on Saturday morning.
    pub async fn shopper_db() -> Cx {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| panic!("DATABASE_URL"));
        let mut cx = shopper();
        cx.db = Db::connect(&url, 2).await.unwrap_or_else(|e| panic!("{e}"));
        cx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_make_id_suffixes() {
        assert_eq!(slug("FW26-OT-014-CHR"), "fw26_ot_014_chr");
        assert_eq!(
            slug("  Wool blend overcoat, charcoal "),
            "wool_blend_overcoat_charcoal"
        );
        assert_eq!(slug("!!"), "x");
    }

    #[test]
    fn operators_and_their_agents_see_costs_but_shoppers_do_not() {
        let tenant =
            acom_core::TenantId::parse("store_northwind").unwrap_or_else(|e| panic!("{e}"));
        let mut p = Principal::system(tenant, "sys_runner");
        assert!(is_operator(&p));
        p.kind = ActorKind::AgentOnBehalf;
        p.subject = "adm_sj".into();
        assert!(is_operator(&p));
        p.subject = "cus_jamie".into();
        assert!(!is_operator(&p));
        p.kind = ActorKind::Shopper;
        assert!(!is_operator(&p));
    }
}
