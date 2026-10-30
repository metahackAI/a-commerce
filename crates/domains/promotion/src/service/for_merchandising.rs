//! Between promotion and merchandising.
//!
//! Merchandising uses: [`promotion`], the period and status of a promotion a banner announces.
//!
//! Promotion needs one thing merchandising owns: which products sit in a category (a sale on
//! "summer outerwear", a promotion scoped to knitwear). Merchandising depends on promotion, not
//! the other way, so merchandising hands that read over once, when it registers its operations
//! ([`provide_category_products`]); promotion calls it through [`category_products`].

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use acom_contract::{ErrorCode, ToolError};
use acom_core::enums::PromotionStatus;
use acom_store::TxCx;
use chrono::{DateTime, Utc};

use crate::{model, repo};

/// The products placed in one category or any category under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryProducts {
    /// `cat_…`.
    pub category_id: String,
    /// Name people read (`Summer outerwear`).
    pub name: String,
    /// Product ids (`prd_…`) in display order, without repeats.
    pub product_ids: Vec<String>,
}

/// Reads [`CategoryProducts`] for the categories among the ids that exist.
pub type CategoryLookup = for<'a> fn(
    &'a mut TxCx,
    &'a [String],
) -> Pin<
    Box<dyn Future<Output = Result<Vec<CategoryProducts>, ToolError>> + Send + 'a>,
>;

static CATEGORY_LOOKUP: OnceLock<CategoryLookup> = OnceLock::new();

/// Called by merchandising when it registers; later calls are ignored.
pub fn provide_category_products(lookup: CategoryLookup) {
    let _ = CATEGORY_LOOKUP.set(lookup);
}

/// The products of each category in `category_ids` (missing categories are left out).
pub(crate) async fn category_products(
    tx: &mut TxCx,
    category_ids: &[String],
) -> Result<Vec<CategoryProducts>, ToolError> {
    if category_ids.is_empty() {
        return Ok(Vec::new());
    }
    let lookup = CATEGORY_LOOKUP.get().ok_or_else(|| {
        ToolError::new(
            ErrorCode::INTERNAL,
            "Category placement is not available: the merchandising domain is not registered.",
        )
    })?;
    lookup(tx, category_ids).await
}

/// What a banner needs to know about the promotion it announces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromotionPeriod {
    pub id: String,
    pub name: String,
    /// Status now (period and clock, unless a person paused or ended it).
    pub status: PromotionStatus,
    /// `None` for everyone.
    pub valid_for_member_tier: Option<String>,
    pub valid_from: DateTime<Utc>,
    /// `None` for a standing promotion.
    pub valid_through: Option<DateTime<Utc>>,
}

/// The promotion, or `None` when the store has no promotion with that id.
pub async fn promotion(
    tx: &mut TxCx,
    promotion_id: &str,
) -> Result<Option<PromotionPeriod>, ToolError> {
    let now = tx.clock.now();
    Ok(repo::promotion(tx, promotion_id)
        .await?
        .map(|p| PromotionPeriod {
            status: model::promotion_status(
                &p.promotion_status,
                p.valid_from,
                p.valid_through,
                now,
            ),
            id: p.id,
            name: p.name,
            valid_for_member_tier: p.valid_for_member_tier,
            valid_from: p.valid_from,
            valid_through: p.valid_through,
        }))
}
