//! The `segment` resource (docs/api/promotion.md#segment): groups of customers defined by
//! conditions (`promotion.segment`), used by `issue_coupon`, messaging and event eligibility.
//! What its operations share: the condition shape, how a segment is evaluated now (tiers from
//! customer, purchases through sales' order reader, categories through merchandising), and SQL.
//!
//! Every given condition must hold. `purchased_category_ids`, `last_order_within_days` and
//! `min_spend` look at the member's orders that were not cancelled (`min_spend` over all of them,
//! net of returns); `exclude_segment_ids` takes out the members of other segments as they are now.

pub(crate) mod define_segment;
pub(crate) mod list_segments;
pub(crate) mod preview_segment;

use std::collections::{BTreeMap, BTreeSet};

use acom_catalog::service::for_promotion::products_with_offers;
use acom_contract::ToolError;
use acom_core::Money;
use acom_customer::service::for_promotion::{self as customers, Member};
use acom_store::TxCx;
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::repo::db;
use crate::service::for_merchandising::category_products;
use crate::service::for_sales::purchases;
use crate::{TIER_NOT_FOUND, check_id, not_found};

/// Who is in a segment. Give at least one condition; every given one must hold.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConditionRules {
    /// Only members of these tiers (`list_tiers`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(inner(regex(pattern = r"^[a-z][a-z0-9_]*$")), example = ["member", "gold"])]
    pub member_tiers: Option<Vec<String>>,
    /// Only members who bought a product of one of these categories (subcategories included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(inner(regex(pattern = r"^cat_[a-z0-9_]+$")), example = ["cat_knitwear"])]
    pub purchased_category_ids: Option<Vec<String>>,
    /// Only members whose last order was placed within this many days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 3650), example = 90)]
    pub last_order_within_days: Option<u32>,
    /// Only members who paid at least this much over all their orders, net of returns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = json!({"value": 200.00, "currency": "USD"}))]
    pub min_spend: Option<Money>,
    /// Leave out the members of these segments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(inner(regex(pattern = r"^seg_[a-z0-9_]+$")), example = ["seg_recent_buyers"])]
    pub exclude_segment_ids: Option<Vec<String>>,
}

impl ConditionRules {
    pub(crate) fn is_empty(&self) -> bool {
        self.member_tiers.as_ref().is_none_or(Vec::is_empty)
            && self
                .purchased_category_ids
                .as_ref()
                .is_none_or(Vec::is_empty)
            && self.last_order_within_days.is_none()
            && self.min_spend.is_none()
            && self.exclude_segment_ids.as_ref().is_none_or(Vec::is_empty)
    }

    fn needs_orders(&self) -> bool {
        self.purchased_category_ids
            .as_ref()
            .is_some_and(|l| !l.is_empty())
            || self.last_order_within_days.is_some()
            || self.min_spend.is_some()
    }
}

/// Checks the ids and tiers the conditions name: `invalid_input` for a bad id or no condition,
/// `tier_not_found`, `not_found` for a segment to exclude.
pub(crate) async fn validate(
    tx: &mut TxCx,
    rules: &ConditionRules,
    next: &'static str,
) -> Result<(), ToolError> {
    if rules.is_empty() {
        return Err(ToolError::invalid_input(
            "Give at least one condition: member_tiers, purchased_category_ids, last_order_within_days, min_spend or exclude_segment_ids.",
        )
        .detail(json!({"field": "condition_rules", "reason": "missing"}))
        .next(next));
    }
    for t in rules.member_tiers.iter().flatten() {
        if customers::tier(tx, t).await?.is_none() {
            return Err(
                ToolError::new(TIER_NOT_FOUND, format!("The store has no tier `{t}`."))
                    .detail(json!({"entity": "tier", "id": t}))
                    .next("list_tiers"),
            );
        }
    }
    for c in rules.purchased_category_ids.iter().flatten() {
        check_id("purchased_category_ids", "cat", c, false)?;
    }
    for s in rules.exclude_segment_ids.iter().flatten() {
        check_id("exclude_segment_ids", "seg", s, false)?;
        if segment(tx, s).await?.is_none() {
            return Err(not_found("segment", s, "list_segments"));
        }
    }
    Ok(())
}

/// A stored segment.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct SegmentRow {
    pub id: String,
    pub name: String,
    pub condition_rules: Value,
    pub estimated_size: i32,
    pub estimated_at: DateTime<Utc>,
    pub version: i32,
}

impl SegmentRow {
    pub(crate) fn rules(&self) -> ConditionRules {
        serde_json::from_value(self.condition_rules.clone()).unwrap_or_default()
    }
}

const SEGMENT_SELECT: &str = "SELECT id, name, condition_rules, estimated_size, estimated_at,
        version FROM promotion.segment WHERE tenant_id = $1";

pub(crate) async fn segment(tx: &mut TxCx, id: &str) -> Result<Option<SegmentRow>, ToolError> {
    sqlx::query_as::<_, SegmentRow>(sqlx::AssertSqlSafe(format!("{SEGMENT_SELECT} AND id = $2")))
        .bind(tx.tenant.as_str())
        .bind(id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)
}

/// The segment with this name (any letter case), if any.
pub(crate) async fn segment_named(
    tx: &mut TxCx,
    name: &str,
) -> Result<Option<SegmentRow>, ToolError> {
    sqlx::query_as::<_, SegmentRow>(sqlx::AssertSqlSafe(format!(
        "{SEGMENT_SELECT} AND lower(name) = lower($2) ORDER BY id LIMIT 1"
    )))
    .bind(tx.tenant.as_str())
    .bind(name)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One page of segments, newest first, and how many there are.
pub(crate) async fn segments(
    tx: &mut TxCx,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<SegmentRow>), ToolError> {
    let total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM promotion.segment WHERE tenant_id = $1")
            .bind(tx.tenant.as_str())
            .fetch_one(&mut *tx.tx)
            .await
            .map_err(db)?;
    let rows = sqlx::query_as::<_, SegmentRow>(sqlx::AssertSqlSafe(format!(
        "{SEGMENT_SELECT} ORDER BY created_at DESC, id LIMIT $2 OFFSET $3"
    )))
    .bind(tx.tenant.as_str())
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

/// The active members who match the conditions now, by id. `seen` holds the segments being
/// evaluated, so a segment that excludes itself through others cannot loop.
pub(crate) async fn members(
    tx: &mut TxCx,
    rules: &ConditionRules,
    now: DateTime<Utc>,
    seen: &mut BTreeSet<String>,
) -> Result<Vec<Member>, ToolError> {
    let tiers = rules.member_tiers.clone().unwrap_or_default();
    let mut found = customers::active_members(tx, &tiers).await?;
    if rules.needs_orders() {
        let bought: BTreeMap<String, crate::service::for_sales::MemberPurchases> = purchases(tx)
            .await?
            .into_iter()
            .map(|p| (p.customer_id.clone(), p))
            .collect();
        let wanted_variants: Option<BTreeSet<String>> = match &rules.purchased_category_ids {
            Some(cats) if !cats.is_empty() => {
                let products: Vec<String> = category_products(tx, cats)
                    .await?
                    .into_iter()
                    .flat_map(|c| c.product_ids)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                Some(
                    products_with_offers(tx, &products)
                        .await?
                        .into_iter()
                        .flat_map(|p| p.offers.into_iter().map(|o| o.variant_id))
                        .collect(),
                )
            }
            _ => None,
        };
        found.retain(|m| {
            let Some(p) = bought.get(&m.customer_id) else {
                return false;
            };
            rules
                .last_order_within_days
                .is_none_or(|d| p.last_order_at >= now - Duration::days(i64::from(d)))
                && rules.min_spend.is_none_or(|min| {
                    p.spend.currency == min.currency && p.spend.value >= min.value
                })
                && wanted_variants
                    .as_ref()
                    .is_none_or(|w| p.variant_ids.iter().any(|v| w.contains(v)))
        });
    }
    for id in rules.exclude_segment_ids.iter().flatten() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(other) = segment(tx, id).await? {
            let out: BTreeSet<String> = Box::pin(members(tx, &other.rules(), now, seen))
                .await?
                .into_iter()
                .map(|m| m.customer_id)
                .collect();
            found.retain(|m| !out.contains(&m.customer_id));
        }
    }
    Ok(found)
}

/// The members of a stored segment now, or `not_found`.
pub(crate) async fn members_of(
    tx: &mut TxCx,
    segment_id: &str,
    now: DateTime<Utc>,
    next: &'static str,
) -> Result<(SegmentRow, Vec<Member>), ToolError> {
    let s = segment(tx, segment_id)
        .await?
        .ok_or_else(|| not_found("segment", segment_id, next))?;
    let mut seen = BTreeSet::from([s.id.clone()]);
    let m = members(tx, &s.rules(), now, &mut seen).await?;
    Ok((s, m))
}

/// How many members each tier has among `members` (`none` for people in no tier).
pub(crate) fn by_tier(members: &[Member]) -> BTreeMap<String, i64> {
    let mut out = BTreeMap::new();
    for m in members {
        *out.entry(m.tier_id.clone().unwrap_or_else(|| "none".into()))
            .or_insert(0) += 1;
    }
    out
}

/// `Lapsed knit buyers` → `lapsed_knit_buyers`.
pub(crate) fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_owned();
    out.chars()
        .take(40)
        .collect::<String>()
        .trim_end_matches('_')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn conditions_are_a_closed_object() {
        let s = closed_schema_for::<ConditionRules>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        let r: ConditionRules = serde_json::from_value(json!({"member_tiers": ["member"],
            "purchased_category_ids": ["cat_knitwear"], "last_order_within_days": 90,
            "min_spend": {"value": 200.00, "currency": "USD"}}))
        .unwrap();
        assert!(!r.is_empty() && r.needs_orders());
        assert!(ConditionRules::default().is_empty());
        assert!(serde_json::from_value::<ConditionRules>(json!({"tiers": ["vip"]})).is_err());
        assert_eq!(slug("Lapsed knit buyers!"), "lapsed_knit_buyers");
    }
}
