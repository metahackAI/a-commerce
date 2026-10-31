//! The `review` resource (docs/api/catalog.md#review): what its operations share.

pub mod create_review;
pub mod get_review_summary;
pub mod list_reviews;
pub mod moderate_review;
pub mod vote_review_helpful;

use acom_contract::{ErrorCode, ToolError};
use acom_store::Cx;
use serde_json::json;

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
