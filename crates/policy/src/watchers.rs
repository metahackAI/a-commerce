//! The watchers of `policies/platform.yml` (`watchers.*`): which run, how often, what each one
//! looks for, and whether its fix may run without a person. Watchers are scheduled jobs; their
//! thresholds come from here (and from the approval thresholds they point at), never from code.
//! Spec: `docs/api/platform.md#task`.

use std::str::FromStr;

use acom_core::enums::TaskOrigin;
use acom_core::{Currency, Money};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;

use crate::{PolicyError, PolicySet};

/// What a watcher does with the fix it drafts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunFix {
    /// Draft the change set and submit it; a person approves it.
    Ask,
    /// The store approved this fix in advance (a policy override, itself approved): the change
    /// set runs as soon as it is drafted, and can be undone until its deadline.
    StandingApproval,
}

/// `watchers.returns_over_refund_threshold`: return requests waiting for a person because their
/// amount is over the refund approval threshold.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnsOverRefundThreshold {
    pub enabled: bool,
    pub every_minutes: u32,
    /// The approval threshold whose amount a request must be over (`approval.thresholds.refund`).
    pub amount_over_from: String,
    /// The claim status that counts as waiting (`requested`).
    pub claim_status: String,
    /// A task appears at this many requests or more.
    pub min_count: u32,
    /// Only requests sent at least this long ago.
    pub min_waiting_hours: u32,
    pub origin: TaskOrigin,
}

/// `watchers.stock_out_forecast`: products that, at the recent sales rate, run out soon.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockOutForecast {
    pub enabled: bool,
    pub every_minutes: u32,
    /// Whole store days before today over which the sales rate is measured.
    pub lookback_days: u32,
    /// Stock ÷ daily sales at or under this many days trips the watcher.
    #[serde(deserialize_with = "decimal")]
    pub days_until_stockout_lte: Decimal,
    /// Offers in these availabilities are left out (`discontinued`, `pre_order`).
    pub exclude_availability: Vec<String>,
    pub origin: TaskOrigin,
}

/// `watchers.price_below_cost`: products whose list price is below their cost.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceBelowCost {
    pub enabled: bool,
    pub every_minutes: u32,
    /// What is compared: `list_price_below_cost`.
    pub compare: String,
    /// The fix drafted: `raise_to_price_floor` (catalog.yml `price_floor`).
    pub fix: String,
    pub run_fix: RunFix,
    pub origin: TaskOrigin,
}

/// Every watcher's settings.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Watchers {
    pub returns_over_refund_threshold: ReturnsOverRefundThreshold,
    pub stock_out_forecast: StockOutForecast,
    pub price_below_cost: PriceBelowCost,
}

/// What the scheduler needs of any watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    pub enabled: bool,
    pub every_minutes: u32,
    pub origin: TaskOrigin,
    /// `Ask` for watchers that draft no fix.
    pub run_fix: RunFix,
}

impl Watchers {
    /// The schedule of the watcher named `key`, if there is one.
    pub fn schedule(&self, key: &str) -> Option<Schedule> {
        let (enabled, every_minutes, origin, run_fix) = match key {
            "returns_over_refund_threshold" => {
                let w = &self.returns_over_refund_threshold;
                (w.enabled, w.every_minutes, w.origin, RunFix::Ask)
            }
            "stock_out_forecast" => {
                let w = &self.stock_out_forecast;
                (w.enabled, w.every_minutes, w.origin, RunFix::Ask)
            }
            "price_below_cost" => {
                let w = &self.price_below_cost;
                (w.enabled, w.every_minutes, w.origin, w.run_fix)
            }
            _ => return None,
        };
        Some(Schedule {
            enabled,
            every_minutes,
            origin,
            run_fix,
        })
    }
}

/// An amount an approval threshold compares with (`approval.thresholds.refund`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmountThreshold {
    /// The threshold's key without the file name (`thresholds.refund`), as tasks cite it.
    pub policy_key: String,
    /// The amount, written to the currency's minor unit.
    pub amount: Money,
    /// `gt` (over) or `gte` (at least).
    pub op: String,
}

impl AmountThreshold {
    /// Whether `value` (in the threshold's currency) crosses it.
    pub fn crossed_by(&self, value: Decimal) -> bool {
        match self.op.as_str() {
            "gte" => value >= self.amount.value,
            _ => value > self.amount.value,
        }
    }
}

impl PolicySet {
    /// `platform.yml watchers`, typed.
    pub fn watchers(&self) -> Result<Watchers, PolicyError> {
        let schema = |message: String| PolicyError::Schema {
            file: "platform".to_owned(),
            message,
        };
        let v = self
            .raw("platform")
            .and_then(|p| p.get("watchers"))
            .ok_or_else(|| schema("`watchers` is missing".to_owned()))?;
        serde_json::from_value(v.clone()).map_err(|e| schema(e.to_string()))
    }

    /// The amount condition of an approval threshold given by dotted path
    /// (`approval.thresholds.refund`): `when: {field, op, value | value_from}`.
    pub fn amount_threshold(&self, path: &str) -> Option<AmountThreshold> {
        let when = self.lookup(path)?.get("when")?;
        let op = when.get("op")?.as_str()?.to_owned();
        if !matches!(op.as_str(), "gt" | "gte") {
            return None;
        }
        let value = match when.get("value_from").and_then(Value::as_str) {
            Some(from) => self.lookup(from)?,
            None => when.get("value")?,
        };
        let currency = Currency::parse(value.get("currency")?.as_str()?).ok()?;
        let mut amount = number(value.get("value")?)?;
        amount.rescale(currency.minor_units());
        let policy_key = path.split_once('.').map_or(path, |(_, rest)| rest);
        Some(AmountThreshold {
            policy_key: policy_key.to_owned(),
            amount: Money::new(amount, currency),
            op,
        })
    }
}

fn number(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => {
            let text = n.to_string();
            Decimal::from_str(&text)
                .or_else(|_| Decimal::from_scientific(&text))
                .ok()
        }
        Value::String(s) => Decimal::from_str(s).ok(),
        _ => None,
    }
}

fn decimal<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Decimal, D::Error> {
    let v = Value::deserialize(d)?;
    number(&v).ok_or_else(|| serde::de::Error::custom(format!("`{v}` is not a number")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::defaults;

    #[test]
    fn reads_every_watcher_from_the_policy_file() {
        let w = defaults().watchers().unwrap();
        assert_eq!(
            w.returns_over_refund_threshold.amount_over_from,
            "approval.thresholds.refund"
        );
        assert_eq!(w.stock_out_forecast.lookback_days, 7);
        assert_eq!(
            w.stock_out_forecast.days_until_stockout_lte,
            Decimal::from(5)
        );
        assert_eq!(w.price_below_cost.run_fix, RunFix::Ask);
        let s = w.schedule("returns_over_refund_threshold").unwrap();
        assert_eq!((s.enabled, s.every_minutes), (true, 30));
        assert_eq!(s.origin, TaskOrigin::AgentNoticed);
        assert!(w.schedule("nothing").is_none());
    }

    #[test]
    fn a_store_can_give_the_fix_a_standing_approval() {
        let demo = defaults()
            .with_overrides([(
                "platform",
                "watchers:\n  price_below_cost:\n    run_fix: standing_approval\n",
            )])
            .unwrap();
        let s = demo
            .watchers()
            .unwrap()
            .schedule("price_below_cost")
            .unwrap();
        assert_eq!(s.run_fix, RunFix::StandingApproval);
        assert_eq!(s.origin, TaskOrigin::Automation);
        let typo = defaults()
            .with_overrides([("platform", "watchers:\n  price_below_cost:\n    run: x\n")])
            .unwrap();
        assert!(typo.watchers().is_err());
    }

    #[test]
    fn reads_the_refund_threshold_amount() {
        let t = defaults()
            .amount_threshold("approval.thresholds.refund")
            .unwrap();
        assert_eq!(t.policy_key, "thresholds.refund");
        assert_eq!(
            serde_json::to_string(&t.amount).unwrap(),
            r#"{"value":200.00,"currency":"USD"}"#
        );
        assert!(t.crossed_by(Decimal::new(20001, 2)));
        assert!(!t.crossed_by(Decimal::new(20000, 2)));
        assert!(defaults().amount_threshold("approval.nothing").is_none());
    }
}
