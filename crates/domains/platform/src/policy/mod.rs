//! The `policy` resource: the rules in effect for this store (defaults merged with the store's
//! overrides). Shared here: the policy files, leaf walking, and how values read to people.

pub mod get_policy;
pub mod list_policies;

use acom_contract::ToolError;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A policy file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PolicyFile {
    /// When a person must approve.
    Approval,
    /// Products, variants, stock, prices, reviews.
    Catalog,
    /// Boards, posts, replies.
    Community,
    /// Members, tiers, consent, privacy, inquiries, remedies.
    Customer,
    /// Cancellations, returns, exchanges, refunds, delivery.
    Fulfillment,
    /// Outside channels, bulk imports, incoming events.
    Integration,
    /// Display, categories, banners, popups, curations.
    Merchandising,
    /// Customer messages.
    Messaging,
    /// Cart, checkout, orders, payments.
    Order,
    /// Sellers, contracts, grades.
    Partner,
    /// Watchers, roles, admins, API keys.
    Platform,
    /// Promotions, sale prices, coupons, loyalty, events.
    Promotion,
    /// Search, comparison, indexing.
    Search,
}

impl PolicyFile {
    pub(crate) const ALL: [PolicyFile; 13] = [
        Self::Approval,
        Self::Catalog,
        Self::Community,
        Self::Customer,
        Self::Fulfillment,
        Self::Integration,
        Self::Merchandising,
        Self::Messaging,
        Self::Order,
        Self::Partner,
        Self::Platform,
        Self::Promotion,
        Self::Search,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Approval => "approval",
            Self::Catalog => "catalog",
            Self::Community => "community",
            Self::Customer => "customer",
            Self::Fulfillment => "fulfillment",
            Self::Integration => "integration",
            Self::Merchandising => "merchandising",
            Self::Messaging => "messaging",
            Self::Order => "order",
            Self::Partner => "partner",
            Self::Platform => "platform",
            Self::Promotion => "promotion",
            Self::Search => "search",
        }
    }

    /// The API domain the file's rules belong to (docs/api/README.md §1).
    pub(crate) fn domain(self) -> &'static str {
        match self {
            Self::Approval => "platform",
            Self::Order => "sales",
            other => other.name(),
        }
    }

    pub(crate) fn summary(self) -> &'static str {
        match self {
            Self::Approval => "When a person must approve, undo windows, irreversible changes",
            Self::Catalog => "Price floors, stock thresholds, product approval, reviews",
            Self::Community => "Boards, posts, replies and moderation",
            Self::Customer => "Members, tiers, consent, privacy, inquiries, remedies",
            Self::Fulfillment => "Cancellation, return and exchange windows, refunds, delivery",
            Self::Integration => "Outside channels, bulk imports, incoming events",
            Self::Merchandising => "Display slots, categories, banners, popups, curations",
            Self::Messaging => "Customer message channels, quiet hours, send caps",
            Self::Order => "Cart, checkout, order and payment rules",
            Self::Partner => "Sellers, contracts, grades",
            Self::Platform => "Task watchers, roles, admin accounts, API keys",
            Self::Promotion => "Discount caps, minimum stock, overlap and coupon rules",
            Self::Search => "Search, comparison and indexing",
        }
    }
}

/// Every leaf under `prefix`, as (dotted key, value). Lists are leaves.
pub(crate) fn leaves(v: &Value, prefix: &str, out: &mut Vec<(String, Value)>) {
    match v {
        Value::Object(o) if !is_money(v) && !is_condition(v) => {
            for (k, x) in o {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                leaves(x, &key, out);
            }
        }
        other => out.push((prefix.to_owned(), other.clone())),
    }
}

fn is_money(v: &Value) -> bool {
    v.as_object()
        .is_some_and(|o| o.len() == 2 && o.contains_key("value") && o.contains_key("currency"))
}

fn is_condition(v: &Value) -> bool {
    v.as_object()
        .is_some_and(|o| o.contains_key("field") && o.contains_key("op"))
}

fn money(v: &Value) -> Option<String> {
    let amount = v.get("value")?.to_string();
    let currency = v.get("currency")?.as_str()?;
    let amount = amount
        .parse::<f64>()
        .map(|a| format!("{a:.2}"))
        .unwrap_or(amount);
    Some(match currency {
        "USD" => format!("${amount}"),
        "EUR" => format!("€{amount}"),
        other => format!("{amount} {other}"),
    })
}

/// How a value reads to people. Ratios become percentages when the key names a share.
pub(crate) fn display(key: &str, v: &Value) -> String {
    let ratio_key = ["discount", "rate", "ratio", "percent", "share", "fee"]
        .iter()
        .any(|w| key.contains(w));
    match v {
        Value::Bool(true) => "yes".into(),
        Value::Bool(false) => "no".into(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if ratio_key && (0.0..=1.0).contains(&f) && !n.is_i64() => {
                format!("{}%", (f * 100.0 * 100.0).round() / 100.0)
            }
            _ => n.to_string(),
        },
        Value::String(s) => s.replace('_', " "),
        Value::Array(a) => a
            .iter()
            .map(|x| display(key, x))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(_) if is_money(v) => money(v).unwrap_or_else(|| v.to_string()),
        Value::Object(o) if is_condition(v) => format!(
            "{} {} {}",
            o.get("field").and_then(Value::as_str).unwrap_or(""),
            o.get("op").and_then(Value::as_str).unwrap_or(""),
            o.get("value").map(|x| display(key, x)).unwrap_or_default()
        ),
        other => other.to_string(),
    }
}

/// The store's override partials, by file.
pub(crate) async fn overrides(cx: &Cx) -> Result<Map<String, Value>, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let rows: Vec<(String, Value)> =
        sqlx::query_as("SELECT file, partial FROM platform.policy_override WHERE tenant_id = $1")
            .bind(cx.tenant.as_str())
            .fetch_all(&mut *tx.tx)
            .await
            .map_err(|e| ToolError::from(acom_store::StoreError::from(e)))?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(rows.into_iter().collect())
}

pub(crate) fn has_path(v: &Value, key: &str) -> bool {
    let mut cur = v;
    for part in key.split('.') {
        match cur.get(part) {
            Some(next) => cur = next,
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn displays_values_people_can_read() {
        assert_eq!(display("max_discount.by_tier.vip", &json!(0.30)), "30%");
        assert_eq!(
            display(
                "thresholds.refund.when.value",
                &json!({"value": 200.00, "currency": "USD"})
            ),
            "$200.00"
        );
        assert_eq!(display("preview_ttl_minutes", &json!(30)), "30");
        assert_eq!(
            display("on_violation", &json!("use_everyone")),
            "use everyone"
        );
    }

    #[test]
    fn walks_leaves_but_keeps_money_whole() {
        let mut out = Vec::new();
        leaves(
            &json!({"a": {"b": 1, "m": {"value": 2, "currency": "USD"}}}),
            "",
            &mut out,
        );
        let keys: Vec<&str> = out.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["a.b", "a.m"]);
    }
}
