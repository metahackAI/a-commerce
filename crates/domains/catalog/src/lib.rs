//! The `catalog` domain: products, variants, offers, inventory, brands, reviews, questions.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/catalog.md`.
//! Layout: `model` (pure business logic: availability, prices, floors, stock rules, review
//! summaries), `rules` (the catalog policy values the model needs), `ops` (one type per
//! operation implementing `ReadOp`/`WriteOp`), `repo` (SQL). Other domains are reached only
//! through their service functions, never through their tables; the ones above catalog in the
//! crate graph come in through [`neighbors`].

pub mod import;
pub mod model;
pub mod neighbors;
pub mod rules;
pub mod service;

mod cards;
mod ops;
mod repo;
mod viewer;
mod watch;
mod write;

use acom_auth::Principal;
use acom_contract::{ErrorClass, ErrorCode, RegistryBuilder, ToolError};
use acom_core::enums::ActorKind;
use acom_store::{Cx, StoreError};
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
    b.read(ops::product_group::SearchProductGroups)
        .read(ops::product_group::GetProductGroup)
        .read(ops::product::GetProduct)
        .read(ops::product::GetProductMeasurements)
        .read(ops::product::GetReturnPolicy)
        .read(ops::product::GetDeliveryPolicy)
        .write(ops::product::DiscontinueProduct)
        .read(ops::offer::GetPriceBreakdown)
        .write(ops::offer::SetPrice)
        .read(ops::inventory::GetInventory)
        .write(ops::inventory::SetVariantStock)
        .read(ops::history::GetChangeHistory)
        .read(ops::brand::ListBrands)
        .read(ops::brand::GetBrand)
        .read(ops::review::GetReviewSummary)
        .read(ops::review::ListReviews)
        .write(ops::review::CreateReview)
        .write(ops::review::VoteReviewHelpful)
        .write(ops::import::BatchImport)
        .write(ops::group_write::CreateProductGroup)
        .write(ops::group_write::UpdateProductGroup)
        .write(ops::variant::CreateProduct)
        .write(ops::variant::UpdateProduct)
        .write(ops::variant::SetVariants)
        .write(ops::approval::SubmitForReview)
        .write(ops::approval::ApproveProduct)
        .write(ops::approval::RejectProduct)
        .write(ops::brand_write::CreateBrand)
        .write(ops::brand_write::UpdateBrand)
        .write(ops::safety::SetSafetyStock)
        .write(ops::schedule::SchedulePriceChange)
        .write(ops::moderate::ModerateReview);
    // Scheduled work (platform.job): the watcher over prices under cost.
    b.job(watch::PriceBelowCost);
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

/// Keeps only the named top-level fields of an object (all of them when `fields` is empty).
pub fn project(v: Value, fields: &[String]) -> Value {
    match v {
        Value::Object(map) if !fields.is_empty() => Value::Object(
            map.into_iter()
                .filter(|(k, _)| fields.iter().any(|f| f == k))
                .collect(),
        ),
        other => other,
    }
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
    use acom_policy::{FILES, PolicySet};

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
    fn keeps_only_the_named_fields() {
        let v = json!({"id": "prd_windbreaker", "name": "Packable windbreaker", "price": 89.00});
        let kept = project(v.clone(), &["id".into(), "price".into()]);
        assert_eq!(kept, json!({"id": "prd_windbreaker", "price": 89.00}));
        assert_eq!(project(v.clone(), &[]), v);
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
