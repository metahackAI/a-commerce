//! The `conditional_gift` resource (docs/api/promotion.md#conditional_gift): a gift added to an
//! order whose subtotal before member benefits reaches a threshold (`conditional_gift.trigger`),
//! one gift per threshold (`conditional_gift.tiers_by_amount`). What its operations share: the
//! threshold shape, the status from the period and the clock, and SQL.

pub(crate) mod create_conditional_gift;
pub(crate) mod list_conditional_gifts;

use acom_contract::ToolError;
use acom_core::Money;
use acom_store::TxCx;
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::repo::db;

/// One threshold and the gift it adds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GiftTier {
    /// The subtotal before member benefits at or above which the gift is added.
    #[schemars(example = json!({"value": 120.00, "currency": "USD"}))]
    pub trigger_threshold: Money,
    /// The product given (`search_products`).
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_merino_tee")]
    pub gift_product_id: String,
}

/// A gift's status now: from its period and the store clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GiftStatus {
    /// Its period has not started.
    Scheduled,
    /// Orders get it now.
    Running,
    /// Its period is over.
    Ended,
}

impl GiftStatus {
    pub(crate) fn at(from: DateTime<Utc>, through: DateTime<Utc>, now: DateTime<Utc>) -> Self {
        if now < from {
            Self::Scheduled
        } else if now >= through + Duration::minutes(1) {
            Self::Ended
        } else {
            Self::Running
        }
    }

    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Running => "running",
            Self::Ended => "ended",
        }
    }
}

/// A stored conditional gift.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct GiftRow {
    pub id: String,
    pub name: String,
    pub gift_tiers: Value,
    pub valid_for_member_tier: Option<String>,
    pub sales_channels: Option<Vec<String>>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
}

const GIFT_SELECT: &str = "SELECT id, name, gift_tiers, valid_for_member_tier, sales_channels,
        valid_from, valid_through FROM promotion.conditional_gift WHERE tenant_id = $1";

pub(crate) async fn gift(tx: &mut TxCx, id: &str) -> Result<Option<GiftRow>, ToolError> {
    sqlx::query_as::<_, GiftRow>(sqlx::AssertSqlSafe(format!("{GIFT_SELECT} AND id = $2")))
        .bind(tx.tenant.as_str())
        .bind(id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)
}

/// Every gift, newest start first (a store has few).
pub(crate) async fn gifts(tx: &mut TxCx) -> Result<Vec<GiftRow>, ToolError> {
    sqlx::query_as::<_, GiftRow>(sqlx::AssertSqlSafe(format!(
        "{GIFT_SELECT} ORDER BY valid_from DESC, id"
    )))
    .bind(tx.tenant.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_follows_the_period_to_the_minute() {
        let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let (from, through) = (
            t("2026-11-20T00:00:00-05:00"),
            t("2026-11-30T23:59:00-05:00"),
        );
        assert_eq!(
            GiftStatus::at(from, through, t("2026-11-06T11:00:00-05:00")),
            GiftStatus::Scheduled
        );
        assert_eq!(
            GiftStatus::at(from, through, t("2026-11-30T23:59:30-05:00")),
            GiftStatus::Running
        );
        assert_eq!(
            GiftStatus::at(from, through, t("2026-12-01T00:00:00-05:00")),
            GiftStatus::Ended
        );
    }
}
