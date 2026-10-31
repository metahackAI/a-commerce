//! The `schema` resource: the database's data dictionary. The source is the migrations'
//! comments, as `cargo xtask check-schema --write` exports them to
//! `docs/api/generated/schema-map.json`; that file is compiled in, so the answer always matches
//! the migrations in this build.

pub mod describe_data;

use std::sync::LazyLock;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use describe_data::DescribeDataInput;

static MAP: LazyLock<Vec<Value>> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../../docs/api/generated/schema-map.json"
    ))
    .unwrap_or_default()
});

/// A database schema (one per domain).
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataSchema {
    /// Stores, roles, change sets, tasks, the action log.
    Platform,
    /// Products, variants, offers, stock, reviews.
    Catalog,
    /// Categories, display slots, banners.
    Merchandising,
    /// Search documents and lexicon.
    Search,
    /// Promotions, sale prices, coupons, loyalty.
    Promotion,
    /// Members, tiers, addresses, preferences.
    Customer,
    /// Carts, orders, payments.
    Sales,
    /// Deliveries, returns, refunds.
    Fulfillment,
    /// Customer messages.
    Messaging,
    /// Daily sales and other derived views.
    Analytics,
    /// Conversations and turns of the agent service.
    Agent,
}

impl DataSchema {
    fn name(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Catalog => "catalog",
            Self::Merchandising => "merchandising",
            Self::Search => "search",
            Self::Promotion => "promotion",
            Self::Customer => "customer",
            Self::Sales => "sales",
            Self::Fulfillment => "fulfillment",
            Self::Messaging => "messaging",
            Self::Analytics => "analytics",
            Self::Agent => "agent",
        }
    }
}

/// `orderStatus`, `schema:orderStatus`, and `order_status` all name the same term.
fn term_matches(have: &str, want: &str) -> bool {
    let norm = |s: &str| {
        s.rsplit(':')
            .next()
            .unwrap_or(s)
            .replace('_', "")
            .to_lowercase()
    };
    let w = norm(want);
    !w.is_empty() && norm(have) == w
}

pub(crate) fn describe(input: &DescribeDataInput) -> Vec<Value> {
    let mut out = Vec::new();
    for t in MAP.iter() {
        let schema = t["schema"].as_str().unwrap_or("");
        let table = t["table"].as_str().unwrap_or("");
        let full = format!("{schema}.{table}");
        if input.schema.is_some_and(|s| s.name() != schema)
            || input.table.as_deref().is_some_and(|x| x != full)
        {
            continue;
        }
        let columns: Vec<Value> = t["columns"].as_array().cloned().unwrap_or_default();
        let table_term = t["term"].as_str().unwrap_or("");
        let keep: Vec<Value> = columns
            .into_iter()
            .filter(|c| {
                let by_term = input.term.as_deref().is_none_or(|w| {
                    term_matches(table_term, w)
                        || c["term"].as_str().is_some_and(|ct| term_matches(ct, w))
                        || c["name"].as_str().is_some_and(|n| term_matches(n, w))
                });
                let by_api = input
                    .api_name
                    .as_deref()
                    .is_none_or(|a| c["name"].as_str() == Some(a));
                by_term && by_api
            })
            .collect();
        if keep.is_empty() && (input.term.is_some() || input.api_name.is_some()) {
            continue;
        }
        out.push(json!({
            "table": full, "kind": t["kind"], "term": t["term"], "id_prefix": t["id_prefix"],
            "description": t["description"], "columns": keep,
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dictionary_is_compiled_in_and_finds_terms() {
        assert!(!MAP.is_empty());
        let found = describe(&DescribeDataInput {
            term: Some("schema:Order".into()),
            schema: None,
            table: None,
            api_name: None,
        });
        assert!(
            found.iter().any(|t| t["table"] == "sales.sales_order"),
            "{found:?}"
        );
        assert!(term_matches("schema:orderStatus", "order_status"));
    }
}
