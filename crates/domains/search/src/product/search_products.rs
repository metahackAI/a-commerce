//! `search_products` (docs/api/search.md#product): the first tool of most requests, shared by
//! operators and shoppers.

use acom_catalog::service::for_search::{self, ProductQuery, ProductSort, Viewer};
use acom_catalog::{TIER_NOT_FOUND, is_operator};
use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError, fields,
};
use acom_core::Money;
use acom_core::enums::Availability;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{category_filter, list, shopper_availability, text_ranks};
use crate::INDEX_UNAVAILABLE;

/// How results are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum SearchSort {
    /// Best match first (the default with a query).
    #[serde(rename = "relevance")]
    Relevance,
    /// Lowest list price first.
    #[serde(rename = "price")]
    Price,
    /// Highest list price first.
    #[serde(rename = "-price")]
    PriceDesc,
    /// Latest release first (the default without a query).
    #[serde(rename = "newest")]
    Newest,
    /// Least stock first.
    #[serde(rename = "inventory_level")]
    Inventory,
    /// Most stock first.
    #[serde(rename = "-inventory_level")]
    InventoryDesc,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchProductsInput {
    /// What the person is looking for, in their words. Empty: filters only.
    #[schemars(length(max = 200), example = &"light jacket for spring trips")]
    pub query: Option<String>,
    /// A standard or display category (`list_categories`); narrower categories are included.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub category_id: Option<String>,
    /// Product tags; only products with all of them.
    #[schemars(example = ["packable"])]
    pub tags: Option<Vec<String>>,
    /// Only products with one of these availabilities (derived from the variants). Default:
    /// every one for operators; in stock, pre-order and back-order for shoppers.
    #[schemars(example = ["in_stock"])]
    pub availability: Option<Vec<Availability>>,
    /// Operators only. Products whose variants hold at least this many units in all.
    #[schemars(range(min = 0), example = 10)]
    pub min_inventory_level: Option<i64>,
    /// Operators only. Products whose variants hold at most this many units in all (low stock).
    #[schemars(range(min = 0), example = 9)]
    pub max_inventory_level: Option<i64>,
    /// Lowest list price.
    pub min_price: Option<Money>,
    /// Highest list price.
    pub max_price: Option<Money>,
    /// Operators only. The member tier whose prices to show (`list_tiers`), or `everyone`.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub valid_for_member_tier: Option<String>,
    /// Operators only. The moment to price at, in store time. Default now.
    #[schemars(example = &"2026-11-07T00:00:00-05:00")]
    pub as_of: Option<String>,
    /// Order of the results.
    #[schemars(example = &"-inventory_level")]
    pub sort: Option<SearchSort>,
    /// Only these fields of each product, for example without `has_variant`.
    #[schemars(example = ["id", "name", "price", "inventory_level"])]
    pub fields: Option<Vec<String>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 50)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// Find products by words, category, stock and price.
#[derive(Debug)]
pub struct SearchProducts;

impl Operation for SearchProducts {
    const META: OpMeta = OpMeta {
        address: "search.product.search",
        tool: "search_products",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/search/products",
        summary: "Find products by words, category, tags, stock and price. Each product comes with its list price, the member price that applies to the caller, total stock and its variants. Operators can price for a tier and time and also see cost.",
        consumes: &[Slot::one("category_id")],
        produces: &[Slot::many("product_id")],
        checks: &[],
        errors: &[
            ErrorCode::INVALID_INPUT,
            TIER_NOT_FOUND,
            ErrorCode::NOT_FOUND,
            ErrorCode::FORBIDDEN,
            INDEX_UNAVAILABLE,
        ],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchProductsInput;
    type Output = Value;
}

fn bad(field: &str, message: impl Into<String>) -> ToolError {
    ToolError::invalid_input(message)
        .detail(json!({"field": field, "reason": "range"}))
        .next("get_facets")
}

/// Input checks that need no database: operator-only fields, ranges, currencies.
fn check(cx: &Cx, i: &SearchProductsInput) -> Result<(), ToolError> {
    let operator = is_operator(&cx.principal);
    for (field, given) in [
        ("min_inventory_level", i.min_inventory_level.is_some()),
        ("max_inventory_level", i.max_inventory_level.is_some()),
    ] {
        if given && !operator {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                format!("Only store operators can filter by `{field}`."),
            )
            .detail(json!({"reason": "scope", "field": field})));
        }
    }
    Viewer::check_inputs(cx, i.valid_for_member_tier.as_deref(), i.as_of.as_deref())?;
    if i.min_inventory_level.is_some_and(|n| n < 0) || i.max_inventory_level.is_some_and(|n| n < 0)
    {
        return Err(bad("min_inventory_level", "Stock levels are 0 or more."));
    }
    if let (Some(lo), Some(hi)) = (i.min_inventory_level, i.max_inventory_level)
        && lo > hi
    {
        return Err(bad(
            "min_inventory_level",
            "`min_inventory_level` is above `max_inventory_level`.",
        ));
    }
    if let (Some(lo), Some(hi)) = (&i.min_price, &i.max_price) {
        if lo.currency != hi.currency {
            return Err(bad(
                "max_price",
                "`min_price` and `max_price` must use one currency.",
            ));
        }
        if lo.value > hi.value {
            return Err(bad("min_price", "`min_price` is above `max_price`."));
        }
    }
    if [&i.min_price, &i.max_price]
        .into_iter()
        .flatten()
        .any(|m| m.value.is_sign_negative())
    {
        return Err(bad("min_price", "Prices are 0 or more."));
    }
    Ok(())
}

impl ReadOp<Cx> for SearchProducts {
    async fn read(&self, cx: &Cx, input: SearchProductsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        check(cx, &input)?;
        let operator = is_operator(&cx.principal);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let viewer = Viewer::resolve(
            cx,
            &mut tx,
            input.valid_for_member_tier.as_deref(),
            input.as_of.as_deref(),
        )
        .await?;
        let only = category_filter(
            &mut tx,
            input.category_id.as_deref(),
            ErrorCode::NOT_FOUND,
            "list_categories",
        )
        .await?;
        let ranked = text_ranks(cx, &mut tx, input.query.as_deref(), !operator).await?;
        let availability: Vec<String> = match &input.availability {
            Some(list) => list.iter().map(|a| a.as_str().to_owned()).collect(),
            None if operator => Vec::new(),
            None => shopper_availability(cx),
        };
        let sort = match (input.sort, ranked.is_some()) {
            (Some(SearchSort::Relevance) | None, true) => ProductSort::Relevance,
            (Some(SearchSort::Relevance) | None, false) => ProductSort::Newest,
            (Some(SearchSort::Price), _) => ProductSort::Price,
            (Some(SearchSort::PriceDesc), _) => ProductSort::PriceDesc,
            (Some(SearchSort::Newest), _) => ProductSort::Newest,
            (Some(SearchSort::Inventory), _) => ProductSort::Inventory,
            (Some(SearchSort::InventoryDesc), _) => ProductSort::InventoryDesc,
        };
        let tags: Vec<String> = input
            .tags
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        let (ids, total) = for_search::find_products(
            &mut tx,
            &ProductQuery {
                ranked: ranked.as_deref(),
                only: only.as_deref(),
                tags: &tags,
                availability: &availability,
                min_inventory: input.min_inventory_level,
                max_inventory: input.max_inventory_level,
                min_price: input.min_price,
                max_price: input.max_price,
                active_only: !operator,
                sort,
                limit: page.limit,
                offset: page.offset,
            },
        )
        .await?;
        let cards = for_search::product_cards(&mut tx, &ids, &viewer).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let fields = input.fields.unwrap_or_default();
        let items = cards
            .iter()
            .map(|c| c.item_json(&viewer, &cx.time))
            .collect();
        Ok(list(fields::keep_each(items, &fields)?, total, &page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;
    use acom_core::Currency;
    use acom_core::enums::ActorKind;
    use rust_decimal::Decimal;

    fn input() -> SearchProductsInput {
        SearchProductsInput {
            query: None,
            category_id: None,
            tags: None,
            availability: None,
            min_inventory_level: None,
            max_inventory_level: None,
            min_price: None,
            max_price: None,
            valid_for_member_tier: None,
            as_of: None,
            sort: None,
            fields: None,
            limit: None,
            cursor: None,
        }
    }

    #[tokio::test]
    async fn shoppers_cannot_use_operator_filters() {
        let mut i = input();
        i.max_inventory_level = Some(9);
        let e = SearchProducts.read(&testing::jamie(), i).await.unwrap_err();
        assert_eq!(e.code, "forbidden");
        let mut i = input();
        i.valid_for_member_tier = Some("vip".into());
        let e = SearchProducts.read(&testing::jamie(), i).await.unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    #[tokio::test]
    async fn price_ranges_must_make_sense() {
        let cx = testing::cx(ActorKind::Operator, "adm_sj");
        let mut i = input();
        i.min_price = Some(Money::new(Decimal::from(150), Currency::USD));
        i.max_price = Some(Money::new(Decimal::from(50), Currency::USD));
        let e = check(&cx, &i).unwrap_err();
        assert_eq!(e.code, "invalid_input");
        assert_eq!(e.next_tool.as_deref(), Some("get_facets"));
        i.min_price = None;
        assert!(check(&cx, &i).is_ok());
    }

    #[test]
    fn sort_words_follow_the_spec() {
        let schema = acom_contract::schema::closed_schema_for::<SearchProductsInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            schema["properties"]["sort"]["enum"],
            json!([
                "relevance",
                "price",
                "-price",
                "newest",
                "inventory_level",
                "-inventory_level"
            ])
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn summer_outerwear_for_the_operator_on_friday() {
        let mut cx = testing::db(ActorKind::Operator, "adm_sj").await;
        let now = cx
            .time
            .parse("2026-11-06T11:02")
            .unwrap_or_else(|e| panic!("{e}"));
        cx.clock = std::sync::Arc::new(acom_core::FixedClock::at(now));
        let mut i = input();
        i.category_id = Some("cat_summer_outerwear".into());
        i.valid_for_member_tier = Some("vip".into());
        i.limit = Some(50);
        let out = SearchProducts
            .read(&cx, i)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["total"], json!(42));
        let wb = out["items"]
            .as_array()
            .and_then(|l| l.iter().find(|p| p["id"] == "prd_windbreaker").cloned())
            .unwrap_or_default();
        assert_eq!(wb["inventory_level"], json!(214));
        assert_eq!(wb["cost"].to_string(), "36.00");
        assert_eq!(wb["member_price"], Value::Null);
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn spring_trip_query_finds_the_windbreaker_for_jamie() {
        let cx = testing::db(ActorKind::Shopper, "cus_jamie").await;
        let mut i = input();
        i.query = Some("light jacket for spring trips".into());
        let out = SearchProducts
            .read(&cx, i)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let ids: Vec<&str> = out["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["id"].as_str())
            .collect();
        assert!(
            ids.contains(&"prd_windbreaker") && ids.contains(&"prd_field_jacket"),
            "{ids:?}"
        );
        assert!(out["items"][0].get("cost").is_none());
    }
}
