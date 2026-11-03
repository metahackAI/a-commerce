//! The `sales` resource (docs/api/analytics.md#sales): `explain_sales_change`,
//! `get_sales_trend` and `get_sales_overview`, one file each, and what they share.

use schemars::JsonSchema;
use serde::Deserialize;

use serde_json::{Value, json};

use crate::repo::{Bucket, Split};

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
    /// The promotion that priced the line (line amounts, without shipping); the lines no
    /// promotion priced are one more row, "Without a promotion", which is not a promotion.
    Promotion,
}

impl Dimension {
    fn split(self) -> Split {
        match self {
            Self::Category => Split::Category,
            Self::MemberTier => Split::MemberTier,
            Self::Promotion => Split::Promotion,
        }
    }

    /// Whether the figures come from order lines (no cancellations or returns there).
    fn by_line(self) -> bool {
        matches!(self, Self::Category | Self::Promotion)
    }

    /// The bucket's value of this dimension, with its name where it has one.
    fn label(self, b: &Bucket) -> Value {
        let mut v = json!({self.key(): b.dimension});
        if self == Self::Promotion {
            // The lines no promotion priced are one row too; named so it is not read as the total,
            // nor, in a sentence, as "no promotion did …" ("No promotion generated $77,987").
            v["name"] = json!(
                b.dimension_name
                    .as_deref()
                    .or(b.dimension.is_none().then_some("Without a promotion"))
            );
        }
        v
    }

    fn key(self) -> &'static str {
        match self {
            Self::Category => "category_id",
            Self::MemberTier => "member_tier",
            Self::Promotion => "promotion_id",
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
