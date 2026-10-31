//! `get_facets` (docs/api/search.md#product): the filters that apply to a query and how many
//! products each value has.

use std::collections::BTreeMap;

use acom_catalog::service::for_search::{self, ProductQuery, ProductSort, Viewer};
use acom_catalog::{INVALID_FILTER, is_operator, rules};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{category_filter, shopper_availability, text_ranks};
use crate::INDEX_UNAVAILABLE;
use crate::model::{self, FacetField};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetFacetsInput {
    /// The query the facets are for, as given to `search_products`.
    #[schemars(length(max = 200), example = &"light jacket")]
    pub query: Option<String>,
    /// A category the products are in (narrower categories included).
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub category_id: Option<String>,
    /// Product tags; only products with all of them.
    #[schemars(example = ["packable"])]
    pub tags: Option<Vec<String>>,
    /// Which facets. Default all.
    #[schemars(example = ["price_range", "availability"])]
    pub facet_fields: Option<Vec<FacetField>>,
}

/// Filter dimensions and counts for a query.
#[derive(Debug)]
pub struct GetFacets;

impl Operation for GetFacets {
    const META: OpMeta = OpMeta {
        address: "search.product.get_facets",
        tool: "get_facets",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/search/facets",
        summary: "The filters that apply to a query (category, brand, price range, color, size, availability) and how many products each value has, for a filter panel or to suggest a narrower search.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[INVALID_FILTER, INDEX_UNAVAILABLE],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetFacetsInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetFacets {
    async fn read(&self, cx: &Cx, input: GetFacetsInput) -> Result<Value, ToolError> {
        let width = cx
            .policy
            .lookup("search.facets.price_range_width")
            .and_then(rules::money)
            .map(|m| m.value)
            .ok_or_else(|| {
                ToolError::new(
                    acom_contract::ErrorCode::INTERNAL,
                    "The store policy `search.facets.price_range_width` is missing.",
                )
            })?;
        let fields: Vec<FacetField> = match input.facet_fields {
            Some(f) if !f.is_empty() => f,
            _ => FacetField::ALL.to_vec(),
        };
        let operator = is_operator(&cx.principal);
        let viewer = Viewer {
            operator,
            tier: None,
            at: cx.clock.now(),
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let only = category_filter(
            &mut tx,
            input.category_id.as_deref(),
            INVALID_FILTER,
            "get_facets",
        )
        .await?;
        let ranked = text_ranks(cx, &mut tx, input.query.as_deref(), !operator).await?;
        let availability = if operator {
            Vec::new()
        } else {
            shopper_availability(cx)
        };
        let tags: Vec<String> = input
            .tags
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        // Every matching product counts, not one page.
        let (ids, _) = for_search::find_products(
            &mut tx,
            &ProductQuery {
                ranked: ranked.as_deref(),
                only: only.as_deref(),
                tags: &tags,
                availability: &availability,
                min_inventory: None,
                max_inventory: None,
                min_price: None,
                max_price: None,
                active_only: !operator,
                sort: ProductSort::Newest,
                limit: i64::from(i32::MAX),
                offset: 0,
            },
        )
        .await?;
        let cards = for_search::product_cards(&mut tx, &ids, &viewer).await?;
        let links = if fields.contains(&FacetField::Category) {
            for_search::categories(&mut tx, Some(&ids)).await?
        } else {
            Vec::new()
        };
        let brands = if fields.contains(&FacetField::Brand) {
            for_search::brand_names(&mut tx).await?
        } else {
            Vec::new()
        };
        tx.commit().await.map_err(ToolError::from)?;
        let mut labels: BTreeMap<String, String> = brands.into_iter().collect();
        labels.extend(
            links
                .iter()
                .map(|l| (l.category_id.clone(), l.name.clone())),
        );
        let items = model::facets(&fields, &cards, &links, &labels, width);
        let total = items.len();
        Ok(json!({"items": items, "total": total}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;
    use acom_core::enums::ActorKind;

    #[test]
    fn facet_words_follow_the_spec() {
        let schema = acom_contract::schema::closed_schema_for::<GetFacetsInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            schema["properties"]["facet_fields"]["items"]["enum"],
            json!([
                "category",
                "brand",
                "price_range",
                "color",
                "size",
                "availability"
            ])
        );
        let width = testing::policies()
            .lookup("search.facets.price_range_width")
            .and_then(rules::money)
            .map(|m| m.value);
        assert_eq!(width.map(|w| w.to_string()), Some("50.00".into()));
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn summer_outerwear_facets_count_products() {
        let cx = testing::db(ActorKind::Operator, "adm_sj").await;
        let out = GetFacets
            .read(
                &cx,
                GetFacetsInput {
                    query: None,
                    category_id: Some("cat_summer_outerwear".into()),
                    tags: None,
                    facet_fields: Some(vec![FacetField::Availability]),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let sum: i64 = out["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["count"].as_i64())
            .sum();
        assert_eq!(sum, 42);
    }
}
