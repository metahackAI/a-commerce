//! The partner rules the operations apply, read from the store's policies (`policies/partner.yml`
//! plus overrides). Nothing here is a constant: a missing key is an internal error naming it, so
//! a broken policy file shows at the first call instead of as a silent default.

use std::collections::BTreeMap;
use std::str::FromStr;

use acom_contract::{ErrorCode, ToolError};
use acom_core::enums::GradeLevel;
use acom_policy::PolicySet;
use rust_decimal::Decimal;
use serde_json::Value;

fn missing(path: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INTERNAL,
        format!("The store policy `partner.{path}` is missing or malformed."),
    )
}

fn get<'a>(p: &'a PolicySet, path: &str) -> Result<&'a Value, ToolError> {
    p.lookup(&format!("partner.{path}"))
        .ok_or_else(|| missing(path))
}

/// An exact decimal from a policy number (`0.40`) or a numeric string.
pub fn decimal(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => {
            let s = n.to_string();
            Decimal::from_str(&s)
                .or_else(|_| Decimal::from_scientific(&s))
                .ok()
        }
        Value::String(s) => Decimal::from_str(s).ok(),
        _ => None,
    }
}

fn number(p: &PolicySet, path: &str) -> Result<Decimal, ToolError> {
    decimal(get(p, path)?).ok_or_else(|| missing(path))
}

fn whole(p: &PolicySet, path: &str) -> Result<i64, ToolError> {
    get(p, path)?.as_i64().ok_or_else(|| missing(path))
}

fn flag(p: &PolicySet, path: &str) -> Result<bool, ToolError> {
    get(p, path)?.as_bool().ok_or_else(|| missing(path))
}

fn words(p: &PolicySet, path: &str) -> Result<Vec<String>, ToolError> {
    get(p, path)?
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .ok_or_else(|| missing(path))
}

fn word_lists(p: &PolicySet, path: &str) -> Result<BTreeMap<String, Vec<String>>, ToolError> {
    let map = get(p, path)?.as_object().ok_or_else(|| missing(path))?;
    Ok(map
        .iter()
        .map(|(k, v)| {
            let list = v
                .as_array()
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            (k.clone(), list)
        })
        .collect())
}

/// What one grade level asks of the metrics (`grade.levels.<level>`); a bound that is not set
/// asks nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LevelBar {
    pub shipping_compliance_min: Option<Decimal>,
    pub order_fulfillment_min: Option<Decimal>,
    pub claim_rate_max: Option<Decimal>,
    pub service_score_min: Option<Decimal>,
}

/// Every partner rule, as the store's policy says it now.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    /// `tax_id_unique`.
    pub tax_id_unique: bool,
    /// `onboarding.required_documents`: `seller_document_type` words.
    pub required_documents: Vec<String>,
    /// `onboarding.prohibited_categories`: `cat_` ids.
    pub prohibited_categories: Vec<String>,
    /// `onboarding.tax_id_digits`: how many digits a tax id has, at least and at most.
    pub tax_id_digits: (usize, usize),
    /// `profile.editable_fields`.
    pub editable_fields: Vec<String>,
    /// `lifecycle.allowed_transitions`: status → statuses it may move to.
    pub allowed_transitions: BTreeMap<String, Vec<String>>,
    /// `suspend.hide_products`.
    pub hide_products: bool,
    /// `commission.min_rate`, `commission.max_rate` (0–1).
    pub commission_min: Decimal,
    pub commission_max: Decimal,
    /// `contract.no_period_overlap`.
    pub no_period_overlap: bool,
    /// `grade.computed_metrics`.
    pub computed_metrics: Vec<String>,
    /// `grade.override_requires_reason`.
    pub override_requires_reason: bool,
    /// `grade.benefits`: level → benefits (`{commission_discount_rate, exposure_weight}`).
    pub benefits: BTreeMap<String, Value>,
    /// `grade.levels`, best first; levels the policy leaves out are not graded with.
    pub levels: Vec<(GradeLevel, LevelBar)>,
    /// `grade.ship_within_days`.
    pub ship_within_days: i64,
    /// `grade.review_day_of_month`.
    pub review_day_of_month: u32,
    /// `settlement.payout_delay_days`.
    pub payout_delay_days: i64,
    /// `privacy.masked_fields`.
    pub masked_fields: Vec<String>,
    /// `privacy.visible_trailing_characters`.
    pub visible_trailing: usize,
    /// `dashboard.visible_metrics_by_role`: role → sections.
    pub visible_metrics: BTreeMap<String, Vec<String>>,
}

impl Rules {
    pub fn load(p: &PolicySet) -> Result<Self, ToolError> {
        let digits = get(p, "onboarding.tax_id_digits")?;
        let bound = |k: &str| {
            digits[k]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| missing("onboarding.tax_id_digits"))
        };
        let tax_id_digits = (bound("min")?, bound("max")?);
        let levels_map = get(p, "grade.levels")?
            .as_object()
            .ok_or_else(|| missing("grade.levels"))?;
        let mut levels = Vec::new();
        for level in GradeLevel::ALL {
            if *level == GradeLevel::Watch {
                continue;
            }
            let Some(bar) = levels_map.get(level.as_str()).filter(|b| b.is_object()) else {
                continue;
            };
            let bound = |k: &str| bar.get(k).and_then(decimal);
            levels.push((
                *level,
                LevelBar {
                    shipping_compliance_min: bound("shipping_compliance_min"),
                    order_fulfillment_min: bound("order_fulfillment_min"),
                    claim_rate_max: bound("claim_rate_max"),
                    service_score_min: bound("service_score_min"),
                },
            ));
        }
        let benefits = get(p, "grade.benefits")?
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        Ok(Self {
            tax_id_unique: flag(p, "tax_id_unique")?,
            required_documents: words(p, "onboarding.required_documents")?,
            prohibited_categories: words(p, "onboarding.prohibited_categories")?,
            tax_id_digits,
            editable_fields: words(p, "profile.editable_fields")?,
            allowed_transitions: word_lists(p, "lifecycle.allowed_transitions")?,
            hide_products: flag(p, "suspend.hide_products")?,
            commission_min: number(p, "commission.min_rate")?,
            commission_max: number(p, "commission.max_rate")?,
            no_period_overlap: flag(p, "contract.no_period_overlap")?,
            computed_metrics: words(p, "grade.computed_metrics")?,
            override_requires_reason: flag(p, "grade.override_requires_reason")?,
            benefits,
            levels,
            ship_within_days: whole(p, "grade.ship_within_days")?,
            review_day_of_month: u32::try_from(whole(p, "grade.review_day_of_month")?)
                .ok()
                .filter(|d| (1..=28).contains(d))
                .ok_or_else(|| missing("grade.review_day_of_month"))?,
            payout_delay_days: whole(p, "settlement.payout_delay_days")?,
            masked_fields: words(p, "privacy.masked_fields")?,
            visible_trailing: usize::try_from(whole(p, "privacy.visible_trailing_characters")?)
                .map_err(|_| missing("privacy.visible_trailing_characters"))?,
            visible_metrics: word_lists(p, "dashboard.visible_metrics_by_role")?,
        })
    }

    /// Whether a seller field is masked for store operators.
    pub fn masks(&self, field: &str) -> bool {
        self.masked_fields.iter().any(|f| f == field)
    }

    /// Whether `from` may move to `to`.
    pub fn allows(&self, from: &str, to: &str) -> bool {
        self.allowed_transitions
            .get(from)
            .is_some_and(|l| l.iter().any(|s| s == to))
    }

    /// Whether a computed metric feeds the grade.
    pub fn grades_with(&self, metric: &str) -> bool {
        self.computed_metrics.iter().any(|m| m == metric)
    }

    /// The benefits of a level, `{}` when the policy gives it none.
    pub fn benefits_of(&self, level: Option<&str>) -> Value {
        level
            .and_then(|l| self.benefits.get(l))
            .cloned()
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn policy() -> PolicySet {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
        let texts: Vec<(&str, String)> = acom_policy::FILES
            .iter()
            .map(|f| {
                (
                    *f,
                    std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
                )
            })
            .collect();
        PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str()))).unwrap()
    }

    pub(crate) fn rules() -> Rules {
        Rules::load(&policy()).unwrap()
    }

    #[test]
    fn reads_every_partner_rule_from_the_policy_file() {
        let r = rules();
        assert!(r.tax_id_unique);
        assert_eq!(
            r.required_documents,
            ["business_registration", "payout_account_proof"]
        );
        assert_eq!(r.tax_id_digits, (9, 13));
        assert!(r.allows("active", "suspended"));
        assert!(!r.allows("terminated", "active"));
        assert_eq!(r.commission_max, Decimal::new(40, 2));
        assert_eq!(r.levels.len(), 4);
        assert_eq!(r.levels[0].0, GradeLevel::Platinum);
        assert_eq!(
            r.levels[0].1.shipping_compliance_min,
            Some(Decimal::new(98, 2))
        );
        assert_eq!(r.review_day_of_month, 1);
        assert_eq!(r.payout_delay_days, 14);
        assert!(r.masks("tax_id") && r.masks("payout_account"));
        assert_eq!(r.visible_metrics["seller"].len(), 6);
    }
}
