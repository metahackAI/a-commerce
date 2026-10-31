//! The `sales` resource (docs/api/analytics.md#sales): `explain_sales_change`,
//! `get_sales_trend` and `get_sales_overview`, one file each, and what they share.

use schemars::JsonSchema;
use serde::Deserialize;

use crate::repo::Split;

mod explain_sales_change;
mod get_sales_overview;
mod get_sales_trend;

pub use explain_sales_change::*;
pub use get_sales_overview::*;
pub use get_sales_trend::*;

/// What figures can be split by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Dimension {
    /// The product's primary standard category (line amounts, without shipping).
    Category,
    /// The tier the shopper is in now; guest orders have no tier.
    MemberTier,
}

impl Dimension {
    fn split(self) -> Split {
        match self {
            Self::Category => Split::Category,
            Self::MemberTier => Split::MemberTier,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Category => "category_id",
            Self::MemberTier => "member_tier",
        }
    }
}

/// Codes the reads may return, for the contract tests.
#[cfg(test)]
pub(crate) const CODES: &[acom_contract::ErrorCode] = &[
    crate::INVALID_PERIOD,
    crate::NO_DATA_FOR_PERIOD,
    crate::INVALID_GRANULARITY,
    crate::DIMENSION_NOT_SUPPORTED,
];
