//! Policy reads the loyalty, event and segment operations share (`policies/promotion.yml`,
//! `approval.yml`). Every business number comes from here; a value that is missing is a broken
//! policy file (`internal`), never a default in code.

use std::collections::BTreeMap;

use acom_contract::{ErrorCode, ToolError};
use acom_core::{Currency, Money};
use acom_policy::PolicySet;
use rust_decimal::Decimal;
use serde_json::Value;

use crate::model::decimal;

/// A policy value that should be there and is not.
pub(crate) fn missing(key: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INTERNAL,
        format!("The policy value `{key}` is missing or not readable."),
    )
}

pub(crate) fn value<'a>(p: &'a PolicySet, key: &str) -> Result<&'a Value, ToolError> {
    p.lookup(key).ok_or_else(|| missing(key))
}

pub(crate) fn number(p: &PolicySet, key: &str) -> Result<Decimal, ToolError> {
    p.lookup(key).and_then(decimal).ok_or_else(|| missing(key))
}

pub(crate) fn integer(p: &PolicySet, key: &str) -> Result<i64, ToolError> {
    number(p, key)?.trunc().try_into().map_err(|_| missing(key))
}

pub(crate) fn word(p: &PolicySet, key: &str) -> Result<String, ToolError> {
    p.lookup(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| missing(key))
}

pub(crate) fn flag(p: &PolicySet, key: &str) -> Result<bool, ToolError> {
    p.lookup(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| missing(key))
}

/// A word list; a missing key is an empty list only when `optional`.
pub(crate) fn words(p: &PolicySet, key: &str, optional: bool) -> Result<Vec<String>, ToolError> {
    match p.lookup(key) {
        Some(Value::Array(l)) => Ok(l
            .iter()
            .filter_map(|w| w.as_str().map(str::to_owned))
            .collect()),
        None | Some(Value::Null) if optional => Ok(Vec::new()),
        _ => Err(missing(key)),
    }
}

/// `{value, currency}` from the policy, as exact money.
pub(crate) fn money(p: &PolicySet, key: &str) -> Result<Money, ToolError> {
    let v = value(p, key)?;
    let amount = v.get("value").and_then(decimal);
    let currency = v
        .get("currency")
        .and_then(Value::as_str)
        .and_then(|c| Currency::parse(c).ok());
    match (amount, currency) {
        (Some(mut a), Some(c)) => {
            a.rescale(c.minor_units());
            Ok(Money::new(a, c))
        }
        _ => Err(missing(key)),
    }
}

/// `promotion.loyalty`: store points.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LoyaltyRules {
    /// `loyalty.expiry.window_days`: points expire this many days after they are added.
    pub window_days: i64,
    /// The largest of `loyalty.expiry.notify_days_before`: what "expiring soon" means.
    pub soon_days: i64,
    /// `loyalty.spend.min_order`.
    pub spend_min_order: Money,
    /// `loyalty.spend.partial_allowed`.
    pub partial_allowed: bool,
    /// `loyalty.grant.max_amount`.
    pub grant_max: Money,
}

impl LoyaltyRules {
    pub(crate) fn read(p: &PolicySet) -> Result<Self, ToolError> {
        let notify = value(p, "promotion.loyalty.expiry.notify_days_before")?
            .as_array()
            .ok_or_else(|| missing("promotion.loyalty.expiry.notify_days_before"))?
            .iter()
            .filter_map(Value::as_i64)
            .max()
            .unwrap_or(0);
        Ok(Self {
            window_days: integer(p, "promotion.loyalty.expiry.window_days")?,
            soon_days: notify,
            spend_min_order: money(p, "promotion.loyalty.spend.min_order")?,
            partial_allowed: flag(p, "promotion.loyalty.spend.partial_allowed")?,
            grant_max: money(p, "promotion.loyalty.grant.max_amount")?,
        })
    }
}

/// One item of an event registry (entry mechanic, selection method or reward type).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RegistryItem {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub builtin: bool,
    /// Entry mechanics only: what an entry sends.
    pub entry_payload_schema: Option<Value>,
    /// Reward types only: `ship`, `issue`, `grant`, `email` or `manual`.
    pub fulfillment: Option<String>,
    /// Reward types only: whether the reward has a money value.
    pub has_value: Option<bool>,
}

/// `purchase_triggered` → `Purchase triggered`.
pub(crate) fn label(id: &str) -> String {
    let words = id.replace('_', " ");
    let mut c = words.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// A registry under `promotion.event.<key>`: the built-in items (words, or for reward types a
/// map of settings), minus `disabled`, plus `store_defined`.
pub(crate) fn registry(p: &PolicySet, key: &str) -> Result<Vec<RegistryItem>, ToolError> {
    let base = format!("promotion.event.{key}");
    let disabled = words(p, &format!("{base}.disabled"), true)?;
    let builtin = value(p, &format!("{base}.builtin"))?;
    let mut out = Vec::new();
    let mut push = |id: String, settings: Option<&Value>, builtin: bool| {
        let s = settings.cloned().unwrap_or(Value::Null);
        out.push(RegistryItem {
            name: s
                .get("name")
                .and_then(Value::as_str)
                .map_or_else(|| label(&id), str::to_owned),
            enabled: !disabled.contains(&id)
                && s.get("enabled").and_then(Value::as_bool).unwrap_or(true),
            builtin,
            entry_payload_schema: s.get("entry_payload_schema").cloned(),
            fulfillment: s
                .get("fulfillment")
                .and_then(Value::as_str)
                .map(str::to_owned),
            has_value: s.get("has_value").and_then(Value::as_bool),
            id,
        });
    };
    match builtin {
        Value::Array(items) => {
            for i in items {
                match i {
                    Value::String(id) => push(id.clone(), None, true),
                    Value::Object(o) => {
                        if let Some(id) = o.get("id").and_then(Value::as_str) {
                            push(id.to_owned(), Some(i), true);
                        }
                    }
                    _ => {}
                }
            }
        }
        Value::Object(m) => {
            for (id, s) in m {
                push(id.clone(), Some(s), true);
            }
        }
        _ => return Err(missing(&format!("{base}.builtin"))),
    }
    if let Some(Value::Array(items)) = p.lookup(&format!("{base}.store_defined")) {
        for i in items {
            if let Some(id) = i.get("id").and_then(Value::as_str) {
                push(id.to_owned(), Some(i), false);
            }
        }
    }
    Ok(out)
}

/// `event.selection_methods.allowed_combinations`: `any`, or entry mechanic → methods.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Combinations {
    Any,
    Only(BTreeMap<String, Vec<String>>),
}

impl Combinations {
    pub(crate) fn read(p: &PolicySet) -> Result<Self, ToolError> {
        let key = "promotion.event.selection_methods.allowed_combinations";
        match value(p, key)? {
            Value::String(s) if s == "any" => Ok(Self::Any),
            Value::Object(m) => Ok(Self::Only(
                m.iter()
                    .map(|(k, v)| {
                        let methods = v
                            .as_array()
                            .map(|l| {
                                l.iter()
                                    .filter_map(|w| w.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default();
                        (k.clone(), methods)
                    })
                    .collect(),
            )),
            _ => Err(missing(key)),
        }
    }

    pub(crate) fn allows(&self, mechanic: &str, method: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Only(m) => m
                .get(mechanic)
                .is_some_and(|l| l.iter().any(|x| x == method)),
        }
    }

    /// As `list_event_types` shows it.
    pub(crate) fn json(&self) -> Value {
        match self {
            Self::Any => Value::from("any"),
            Self::Only(m) => serde_json::to_value(m).unwrap_or(Value::Null),
        }
    }
}

/// One store-defined compliance rule (`event.compliance.rules[]`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ComplianceRule {
    pub jurisdiction: String,
    /// The reward value at or above which the rule applies (`applies_when` with `gte` or `gt`).
    pub threshold: Option<Money>,
    /// `gt` (over) or `gte` (at least).
    pub op: String,
    pub actions: Vec<String>,
    pub winner_info_required: Vec<String>,
}

impl ComplianceRule {
    /// Whether a reward of this value triggers the rule (a rule without a threshold always does;
    /// a reward without a value never triggers a threshold rule).
    pub(crate) fn applies(&self, reward_value: Option<Money>) -> bool {
        match (self.threshold, reward_value) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(t), Some(v)) => {
                v.currency == t.currency
                    && if self.op == "gt" {
                        v.value > t.value
                    } else {
                        v.value >= t.value
                    }
            }
        }
    }
}

pub(crate) fn compliance_rules(p: &PolicySet) -> Result<Vec<ComplianceRule>, ToolError> {
    let key = "promotion.event.compliance.rules";
    let list = value(p, key)?.as_array().ok_or_else(|| missing(key))?;
    Ok(list
        .iter()
        .map(|r| {
            let when = r.get("applies_when");
            let threshold = when.and_then(|w| w.get("value")).and_then(|v| {
                Some(Money::new(
                    v.get("value").and_then(decimal)?,
                    Currency::parse(v.get("currency")?.as_str()?).ok()?,
                ))
            });
            let list = |k: &str| {
                r.get(k)
                    .and_then(Value::as_array)
                    .map(|l| {
                        l.iter()
                            .filter_map(|w| w.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            ComplianceRule {
                jurisdiction: r
                    .get("jurisdiction")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                threshold,
                op: when
                    .and_then(|w| w.get("op"))
                    .and_then(Value::as_str)
                    .unwrap_or("gte")
                    .to_owned(),
                actions: list("actions"),
                winner_info_required: list("winner_info_required"),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::policies;

    #[test]
    fn reads_loyalty_rules() {
        let r = LoyaltyRules::read(&policies(false)).unwrap();
        assert_eq!(r.window_days, 365);
        assert_eq!(r.soon_days, 10);
        assert!(r.partial_allowed);
        assert_eq!(r.grant_max.value.to_string(), "500.00");
    }

    #[test]
    fn registries_list_builtins_and_honour_disabled() {
        let p = policies(false);
        let mechanics = registry(&p, "entry_mechanics").unwrap();
        assert_eq!(mechanics.len(), 7);
        assert!(mechanics.iter().all(|m| m.enabled && m.builtin));
        let rewards = registry(&p, "reward_types").unwrap();
        let product = rewards.iter().find(|r| r.id == "product").unwrap();
        assert_eq!(product.fulfillment.as_deref(), Some("ship"));
        assert_eq!(product.has_value, Some(true));
        let off = p
            .with_overrides([(
                "promotion",
                "event:\n  entry_mechanics:\n    disabled: [survey]\n    store_defined:\n      - {id: photo_styling_contest, name: Styling photo, entry_payload_schema: {image: ImageObject}}\n",
            )])
            .unwrap();
        let mechanics = registry(&off, "entry_mechanics").unwrap();
        assert!(!mechanics.iter().find(|m| m.id == "survey").unwrap().enabled);
        let photo = mechanics
            .iter()
            .find(|m| m.id == "photo_styling_contest")
            .unwrap();
        assert!(!photo.builtin && photo.enabled && photo.entry_payload_schema.is_some());
        assert_eq!(photo.name, "Styling photo");
        assert_eq!(label("purchase_triggered"), "Purchase triggered");
        assert_eq!(Combinations::read(&p).unwrap(), Combinations::Any);
    }

    #[test]
    fn compliance_rules_apply_from_their_threshold() {
        let p = policies(false)
            .with_overrides([(
                "promotion",
                "event:\n  compliance:\n    rules:\n      - {jurisdiction: us_ny, applies_when: {field: reward_value, op: gte, value: {value: 600.00, currency: USD}}, actions: [collect_winner_info], winner_info_required: [legal_name, tax_id]}\n",
            )])
            .unwrap();
        let rules = compliance_rules(&p).unwrap();
        assert_eq!(rules.len(), 1);
        let usd = |v: &str| Some(Money::new(v.parse().unwrap(), Currency::USD));
        assert!(rules[0].applies(usd("600.00")));
        assert!(!rules[0].applies(usd("289.00")));
        assert!(!rules[0].applies(None));
        assert!(compliance_rules(&policies(false)).unwrap().is_empty());
    }
}
