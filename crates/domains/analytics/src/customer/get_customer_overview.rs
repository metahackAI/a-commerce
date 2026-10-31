//! `get_customer_overview` (docs/api/analytics.md#customer): member counts, never members.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ToolError,
};
use acom_sales::service::for_analytics;
use acom_store::Cx;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{INVALID_PERIOD, rate, repo, span};

/// A member figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CustomerMetric {
    /// Accounts created in the period.
    NewMembers,
    /// Members who placed a paid, not cancelled order in the period.
    ActiveMembers,
    /// Members per tier now, one item per tier (the period does not change it).
    TierDistribution,
    /// Active members who ordered twice or more, out of active members, 0–1.
    RepurchaseRate,
}

impl CustomerMetric {
    const ALL: [Self; 4] = [
        Self::NewMembers,
        Self::ActiveMembers,
        Self::TierDistribution,
        Self::RepurchaseRate,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::NewMembers => "new_members",
            Self::ActiveMembers => "active_members",
            Self::TierDistribution => "tier_distribution",
            Self::RepurchaseRate => "repurchase_rate",
        }
    }
}

/// What member figures can be split by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CustomerDimension {
    /// The tier the members are in now. Splits new_members; tier_distribution is always by tier.
    MemberTier,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCustomerOverviewInput {
    /// First store day of the period.
    #[schemars(example = &"2026-10-01")]
    pub valid_from: NaiveDate,
    /// Last store day, included; today at the latest.
    #[schemars(example = &"2026-10-31")]
    pub valid_through: NaiveDate,
    /// Figures to give. Default all four.
    #[serde(rename = "acom:metrics", alias = "metrics")]
    #[schemars(example = ["new_members", "tier_distribution"])]
    pub metrics: Option<Vec<CustomerMetric>>,
    /// `member_tier` splits new_members by tier. active_members and repurchase_rate are counted
    /// over all members.
    #[serde(rename = "acom:dimensions", alias = "dimensions")]
    #[schemars(example = ["member_tier"])]
    pub dimensions: Option<Vec<CustomerDimension>>,
}

/// Member counts of a period: new, active, per tier, and how many bought again.
#[derive(Debug)]
pub struct GetCustomerOverview;

impl Operation for GetCustomerOverview {
    const META: OpMeta = OpMeta {
        address: "analytics.customer.get_customer_overview",
        tool: "get_customer_overview",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Post,
        path: "/v1/analytics/customers/overview",
        summary: "Member counts for a period: new members, active members, members per tier and the repurchase rate. Counts only, never who.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[INVALID_PERIOD],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCustomerOverviewInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCustomerOverview {
    async fn read(&self, cx: &Cx, input: GetCustomerOverviewInput) -> Result<Value, ToolError> {
        let period = span(
            cx,
            input.valid_from,
            input.valid_through,
            "get_customer_overview",
        )?;
        let mut metrics = input
            .metrics
            .unwrap_or_else(|| CustomerMetric::ALL.to_vec());
        metrics.dedup();
        let by_tier = input
            .dimensions
            .is_some_and(|d| d.contains(&CustomerDimension::MemberTier));
        let (start, end) = period.instants(cx.time.zone);

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mut items = Vec::new();
        for m in metrics {
            match m {
                CustomerMetric::NewMembers => {
                    let rows = repo::new_members(&mut tx, period.from, period.through).await?;
                    if by_tier {
                        for (tier, n) in rows {
                            items.push(json!({"acom:metric": m.as_str(), "value": n,
                                              "acom:dimension": {"member_tier": tier}}));
                        }
                    } else {
                        let n: i64 = rows.iter().map(|(_, n)| n).sum();
                        items.push(json!({"acom:metric": m.as_str(), "value": n,
                                          "acom:dimension": null}));
                    }
                }
                CustomerMetric::TierDistribution => {
                    for (tier, n) in repo::tier_counts(&mut tx).await? {
                        items.push(json!({"acom:metric": m.as_str(), "value": n,
                                          "acom:dimension": {"member_tier": tier}}));
                    }
                }
                CustomerMetric::ActiveMembers | CustomerMetric::RepurchaseRate => {
                    let a = for_analytics::member_activity(&mut tx, start, end).await?;
                    let value = if m == CustomerMetric::ActiveMembers {
                        json!(a.active)
                    } else {
                        rate(Decimal::from(a.repeat), Decimal::from(a.active))
                    };
                    items.push(json!({"acom:metric": m.as_str(), "value": value,
                                      "acom:dimension": null}));
                }
            }
        }
        tx.commit().await.map_err(ToolError::from)?;
        let total = items.len();
        Ok(json!({
            "items": items,
            "total": total,
            "valid_from": period.from,
            "valid_through": period.through,
        }))
    }
}
