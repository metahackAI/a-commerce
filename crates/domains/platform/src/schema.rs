//! `describe_data`: the database's data dictionary. The source is the migrations' comments, as
//! `cargo xtask check-schema --write` exports them to `docs/api/generated/schema-map.json`;
//! that file is compiled in, so the answer always matches the migrations in this build.

use std::sync::LazyLock;

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, ReadResult,
    Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

static MAP: LazyLock<Vec<Value>> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../docs/api/generated/schema-map.json"
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeDataInput {
    /// A schema.org or acom term; only tables and columns that use it (`orderStatus`,
    /// `schema:Order`, `acom:percentOff`).
    #[schemars(example = &"orderStatus")]
    pub term: Option<String>,
    /// Only this schema.
    #[schemars(example = &"sales")]
    pub schema: Option<DataSchema>,
    /// One table, as `schema.table`.
    #[schemars(regex(pattern = r"^[a-z_]+\.[a-z_]+$"), example = &"sales.sales_order")]
    pub table: Option<String>,
    /// A name the API uses, for example `variant_id`.
    #[schemars(example = &"variant_id")]
    pub api_name: Option<String>,
}

pub struct DescribeData;

impl Operation for DescribeData {
    const META: OpMeta = OpMeta {
        address: "platform.schema.describe",
        tool: "describe_data",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/schema",
        summary: "The data dictionary: every table and column with its schema.org term, a one-sentence meaning, type, allowed values and references. Use it to learn what a field means or where a number lives.",
        consumes: &[],
        produces: &[Slot::many("table_name")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = DescribeDataInput;
    type Output = ReadResult<Value>;
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

impl ReadOp<Cx> for DescribeData {
    async fn read(&self, _: &Cx, input: DescribeDataInput) -> Result<ReadResult<Value>, ToolError> {
        let items = describe(&input);
        if items.is_empty() {
            let (entity, id) = match (&input.table, &input.term) {
                (Some(t), _) => ("table", t.clone()),
                (_, Some(t)) => ("term", t.clone()),
                _ => ("table", String::new()),
            };
            return Err(ToolError::new(
                ErrorCode::NOT_FOUND,
                format!("Nothing in the data dictionary matches `{id}`."),
            )
            .detail(json!({"entity": entity, "id": id}))
            .next("describe_data"));
        }
        Ok(ReadResult::all(items))
    }
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
