//! `autocomplete` (docs/api/search.md#product): suggestions while the person types.

use std::collections::BTreeMap;

use acom_catalog::is_operator;
use acom_catalog::model::num;
use acom_catalog::service::for_search;
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ToolError,
};
use acom_store::Cx;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{self, SuggestType};
use crate::{INDEX_UNAVAILABLE, INVALID_PREFIX};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AutocompleteInput {
    /// What the person has typed so far, at least one letter or digit.
    #[schemars(length(min = 1, max = 100), example = &"wind")]
    pub prefix: String,
    /// What to suggest. Default keyword.
    #[schemars(example = &"keyword")]
    pub suggest_type: Option<SuggestType>,
    /// How many suggestions. Default and maximum from the store's search policy (10 and 20).
    #[schemars(range(min = 1, max = 20), example = 10)]
    pub limit: Option<u32>,
}

/// Suggestions for a prefix.
#[derive(Debug)]
pub struct Autocomplete;

impl Operation for Autocomplete {
    const META: OpMeta = OpMeta {
        address: "search.product.autocomplete",
        tool: "autocomplete",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/search/autocomplete",
        summary: "Suggestions for what the person is typing: search words, product names, brands or categories that start with the prefix.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[INVALID_PREFIX, INDEX_UNAVAILABLE],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = AutocompleteInput;
    type Output = Value;
}

/// The limit asked for, within `search.yml autocomplete`.
fn limit(cx: &Cx, asked: Option<u32>) -> Result<usize, ToolError> {
    let read = |key: &str| {
        cx.policy
            .lookup(&format!("search.autocomplete.{key}"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let (default, max) = (read("default_limit"), read("max_limit"));
    let n = asked.map_or(default, u64::from);
    if n == 0 || n > max {
        return Err(
            ToolError::invalid_input(format!("`limit` must be 1–{max}."))
                .detail(json!({"field": "limit", "reason": "range"})),
        );
    }
    Ok(n as usize)
}

fn item(text: &str, kind: SuggestType, score: Decimal, id: Option<&str>) -> Value {
    let mut v = json!({"text": text, "suggest_type": kind.word(), "score": num(score)});
    if let Some(id) = id {
        v["id"] = json!(id);
    }
    v
}

impl ReadOp<Cx> for Autocomplete {
    async fn read(&self, cx: &Cx, input: AutocompleteInput) -> Result<Value, ToolError> {
        let prefix = input.prefix.trim().to_lowercase();
        if !prefix.chars().any(char::is_alphanumeric) {
            return Err(ToolError::new(
                INVALID_PREFIX,
                "Type at least one letter or digit to get suggestions.",
            )
            .detail(json!({"field": "prefix", "reason": "format"}))
            .next("autocomplete"));
        }
        let limit = limit(cx, input.limit)?;
        let kind = input.suggest_type.unwrap_or(SuggestType::Keyword);
        let active_only = !is_operator(&cx.principal);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let all: Vec<Value> = match kind {
            SuggestType::Keyword => {
                let texts = for_search::product_texts(&mut tx, active_only).await?;
                let lines: Vec<String> = texts
                    .iter()
                    .map(|t| {
                        format!(
                            "{} {} {}",
                            t.name,
                            t.material.as_deref().unwrap_or_default(),
                            t.keywords.join(" ")
                        )
                    })
                    .collect();
                model::keyword_suggestions(&prefix, &lines)
                    .into_iter()
                    .map(|(w, s)| item(&w, kind, s, None))
                    .collect()
            }
            SuggestType::Product => {
                let texts = for_search::product_texts(&mut tx, active_only).await?;
                let names: Vec<(String, String)> =
                    texts.into_iter().map(|t| (t.id, t.name)).collect();
                model::name_suggestions(&prefix, &names)
                    .into_iter()
                    .map(|(id, n, s)| item(&n, kind, s, Some(&id)))
                    .collect()
            }
            SuggestType::Brand => {
                let names = for_search::brand_names(&mut tx).await?;
                model::name_suggestions(&prefix, &names)
                    .into_iter()
                    .map(|(id, n, s)| item(&n, kind, s, Some(&id)))
                    .collect()
            }
            SuggestType::Category => {
                let names: BTreeMap<String, String> = for_search::categories(&mut tx, None)
                    .await?
                    .into_iter()
                    .map(|l| (l.category_id, l.name))
                    .collect();
                let names: Vec<(String, String)> = names.into_iter().collect();
                model::name_suggestions(&prefix, &names)
                    .into_iter()
                    .map(|(id, n, s)| item(&n, kind, s, Some(&id)))
                    .collect()
            }
        };
        tx.commit().await.map_err(ToolError::from)?;
        let total = all.len();
        let items: Vec<Value> = all.into_iter().take(limit).collect();
        Ok(json!({"items": items, "total": total}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;
    use acom_core::enums::ActorKind;

    #[tokio::test]
    async fn a_prefix_needs_a_letter_or_digit() {
        let e = Autocomplete
            .read(
                &testing::jamie(),
                AutocompleteInput {
                    prefix: " -- ".into(),
                    suggest_type: None,
                    limit: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_prefix");
        assert_eq!(e.next_tool.as_deref(), Some("autocomplete"));
    }

    #[tokio::test]
    async fn limits_come_from_the_policy() {
        let cx = testing::jamie();
        assert_eq!(limit(&cx, None).ok(), Some(10));
        assert_eq!(limit(&cx, Some(20)).ok(), Some(20));
        assert!(limit(&cx, Some(21)).is_err());
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn wind_suggests_the_windbreaker() {
        let cx = testing::db(ActorKind::Shopper, "cus_jamie").await;
        let out = Autocomplete
            .read(
                &cx,
                AutocompleteInput {
                    prefix: "wind".into(),
                    suggest_type: None,
                    limit: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["items"][0]["text"], "windbreaker");
    }
}
