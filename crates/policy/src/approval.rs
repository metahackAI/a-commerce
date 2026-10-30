//! When a write needs a person's approval (`policies/approval.yml`).
//!
//! A preview computes facts about the change (`amount`, `changed_offers`, `reaches_customers`,
//! …) and asks [`PolicySet::approval_for`]. The thresholds decide; the operation never
//! hard-codes a limit. Kind W$ operations need approval whatever this says.

use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::Value;

use crate::PolicySet;

/// What the approval policy says about one write.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Approval {
    pub needed: bool,
    /// A sentence for the change-set card, for example "Refund over $200.00".
    pub reason: Option<String>,
    /// The role that may approve (`merchandiser`, `operator`, …).
    pub approver_role: Option<String>,
    /// The threshold key that matched, for example `refund`.
    pub threshold: Option<String>,
}

/// A number or an amount (`{value, currency}`) as an exact decimal; money is never compared as
/// a float.
fn number(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => {
            let text = n.to_string();
            Decimal::from_str(&text)
                .or_else(|_| Decimal::from_scientific(&text))
                .ok()
        }
        Value::String(s) => Decimal::from_str(s).ok(),
        Value::Object(o) => o.get("value").and_then(number),
        _ => None,
    }
}

fn money_text(v: &Value) -> Option<String> {
    let o = v.as_object()?;
    let amount = number(o.get("value")?)?.round_dp(2);
    Some(match o.get("currency")?.as_str()? {
        "USD" => format!("${amount:.2}"),
        "EUR" => format!("€{amount:.2}"),
        other => format!("{amount:.2} {other}"),
    })
}

fn text(v: &Value) -> String {
    money_text(v).unwrap_or_else(|| match v {
        Value::String(s) => s.replace('_', " "),
        other => other.to_string(),
    })
}

impl PolicySet {
    /// A value by dotted path, the first part naming the file (`approval.thresholds.refund`).
    pub fn lookup(&self, path: &str) -> Option<&Value> {
        let mut parts = path.split('.');
        let mut v = self.raw(parts.next()?)?;
        for p in parts {
            v = v.get(p)?;
        }
        Some(v)
    }

    /// Whether `tool`, with the facts its preview computed, needs a person's approval.
    /// Tools in `no_approval` never do. The first threshold that applies and matches wins.
    pub fn approval_for(&self, tool: &str, facts: &Value) -> Approval {
        let Some(policy) = self.raw("approval") else {
            return Approval::default();
        };
        let exempt = policy["no_approval"]
            .as_array()
            .is_some_and(|l| l.iter().any(|t| t.as_str() == Some(tool)));
        if exempt {
            return Approval::default();
        }
        let Some(thresholds) = policy["thresholds"].as_object() else {
            return Approval::default();
        };
        for (key, t) in thresholds {
            let applies = t["applies_to"]
                .as_array()
                .is_some_and(|l| l.iter().any(|x| x.as_str() == Some(tool)));
            if !applies {
                continue;
            }
            if let Some(why) = self.holds(&t["when"], facts) {
                return Approval {
                    needed: true,
                    reason: Some(why),
                    approver_role: t["approver_role"].as_str().map(str::to_owned),
                    threshold: Some(key.clone()),
                };
            }
        }
        Approval::default()
    }

    /// `Some(reason)` when the condition holds for the facts.
    fn holds(&self, when: &Value, facts: &Value) -> Option<String> {
        match when {
            Value::String(s) if s == "always" => Some("Always needs approval".into()),
            Value::String(s) if s == "reaches_customers" => (facts["reaches_customers"]
                == Value::Bool(true))
            .then(|| "Reaches customers".into()),
            Value::Object(o) if o.contains_key("all") => {
                let parts: Option<Vec<String>> = o["all"]
                    .as_array()?
                    .iter()
                    .map(|c| self.holds(c, facts))
                    .collect();
                parts.map(|p| p.join(" and "))
            }
            Value::Object(o) if o.contains_key("any") => o["any"]
                .as_array()?
                .iter()
                .find_map(|c| self.holds(c, facts)),
            Value::Object(o) => {
                let field = o.get("field")?.as_str()?;
                let op = o.get("op")?.as_str()?;
                let expected = match o.get("value_from").and_then(Value::as_str) {
                    Some(path) => self.lookup(path)?.clone(),
                    None => o.get("value")?.clone(),
                };
                let actual = facts.get(field)?;
                let ok = match op {
                    "eq" => {
                        actual == &expected
                            || number(actual)
                                .zip(number(&expected))
                                .is_some_and(|(a, b)| a == b)
                    }
                    "ne" => actual != &expected,
                    "in" => expected.as_array().is_some_and(|l| l.contains(actual)),
                    _ => {
                        let (a, b) = (number(actual)?, number(&expected)?);
                        match op {
                            "gt" => a > b,
                            "gte" => a >= b,
                            "lt" => a < b,
                            "lte" => a <= b,
                            _ => return None,
                        }
                    }
                };
                let words = match op {
                    "gt" => "over",
                    "gte" => "at least",
                    "lt" => "under",
                    "lte" => "at most",
                    "ne" => "not",
                    "in" => "one of",
                    _ => "is",
                };
                ok.then(|| {
                    let name = field.replace('_', " ");
                    let mut c = name.chars();
                    let name: String = c
                        .next()
                        .map(|f| f.to_uppercase().chain(c).collect())
                        .unwrap_or_default();
                    format!("{name} {words} {}", text(&expected))
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::tests::defaults;

    #[test]
    fn thresholds_decide_from_the_facts() {
        let p = defaults();
        let small = p.approval_for(
            "refund_return",
            &json!({"amount": {"value": 120.0, "currency": "USD"}}),
        );
        assert!(!small.needed);
        let big = p.approval_for(
            "refund_return",
            &json!({"amount": {"value": 240.0, "currency": "USD"}}),
        );
        assert!(big.needed);
        assert_eq!(big.reason.as_deref(), Some("Amount over $200.00"));
        assert_eq!(big.approver_role.as_deref(), Some("operator"));

        assert!(p.approval_for("set_sale_prices", &json!({})).needed);
        assert!(
            !p.approval_for("add_to_cart", &json!({"amount": 1e9}))
                .needed
        );
        assert!(
            !p.approval_for("set_price", &json!({"changed_offers": 3}))
                .needed
        );
        assert!(
            p.approval_for("set_price", &json!({"changed_offers": 12}))
                .needed
        );
        let banner = p.approval_for("schedule_banner", &json!({"reaches_customers": true}));
        assert_eq!(banner.approver_role.as_deref(), Some("merchandiser"));
        let product = p.approval_for("update_product", &json!({"change_type": "structural"}));
        assert!(product.needed);
    }

    #[test]
    fn looks_up_values_by_path() {
        let p = defaults();
        assert_eq!(
            p.lookup("approval.preview_ttl_minutes")
                .and_then(|v| v.as_i64()),
            Some(30)
        );
        assert!(p.lookup("approval.nothing.here").is_none());
    }
}
