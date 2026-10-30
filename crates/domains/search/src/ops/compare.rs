//! `compare_products` (docs/api/search.md#product): products side by side on the same
//! attributes, priced for the caller.

use acom_catalog::service::for_search::{self, Viewer};
use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{self, Attribute};
use crate::{TOO_MANY_PRODUCTS, UNKNOWN_ATTRIBUTE};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompareProductsInput {
    /// The products to compare, 2–6, no repeats (ids from `search_products`).
    #[schemars(inner(regex(pattern = r"^prd_[a-z0-9_]+$")))]
    #[schemars(example = ["prd_windbreaker", "prd_field_jacket", "prd_seersucker_overshirt"])]
    pub product_ids: Vec<String>,
    /// The attributes to line up, returned in this order. Default: every one the store allows.
    #[schemars(example = ["packed_size", "weight", "rain", "price"])]
    pub attributes: Option<Vec<Attribute>>,
    /// One of `attributes`: order the products by it, smallest first (products without it go
    /// last). Default: the order given.
    #[schemars(example = &"packed_size")]
    pub sort_by: Option<Attribute>,
}

/// Compare products on the same attributes.
#[derive(Debug)]
pub struct CompareProducts;

impl Operation for CompareProducts {
    const META: OpMeta = OpMeta {
        address: "search.product.compare",
        tool: "compare_products",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/search/products/compare",
        summary: "Put 2–6 products side by side on the same attributes (weight, packed size, packing, warmth, rain, material, the caller's price), optionally sorted by one, to answer which one packs smallest or weighs least.",
        consumes: &[Slot::many("product_id")],
        produces: &[],
        checks: &[],
        errors: &[
            TOO_MANY_PRODUCTS,
            ErrorCode::INVALID_INPUT,
            ErrorCode::NOT_FOUND,
            UNKNOWN_ATTRIBUTE,
        ],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = CompareProductsInput;
    type Output = Value;
}

/// The checked request: how many products, which attributes, which to sort by.
pub(crate) fn plan(
    policy: &acom_policy::PolicySet,
    input: &CompareProductsInput,
) -> Result<(Vec<Attribute>, Option<Attribute>), ToolError> {
    let bound = |key: &str| {
        policy
            .lookup(&format!("search.compare.{key}"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize
    };
    let (min, max) = (bound("min_products"), bound("max_products"));
    let n = input.product_ids.len();
    if n > max {
        return Err(ToolError::new(
            TOO_MANY_PRODUCTS,
            format!("Compare at most {max} products at a time."),
        )
        .detail(json!({"limit": max, "actual": n, "policy_key": "compare.max_products"}))
        .next("compare_products"));
    }
    if n < min {
        return Err(
            ToolError::invalid_input(format!("Give at least {min} products to compare."))
                .detail(json!({"field": "product_ids", "reason": "range"}))
                .next("search_products"),
        );
    }
    for (i, id) in input.product_ids.iter().enumerate() {
        if input.product_ids[..i].contains(id) {
            return Err(ToolError::invalid_input(format!("{id} is listed twice."))
                .detail(json!({"field": "product_ids", "reason": "format", "id": id}))
                .next("search_products"));
        }
    }
    let allowed = model::allowed_attributes(policy);
    let allowed_words: Vec<&str> = allowed.iter().map(|a| a.word()).collect();
    let unknown = |a: Attribute| {
        ToolError::new(
            UNKNOWN_ATTRIBUTE,
            format!("This store does not compare by `{}`.", a.word()),
        )
        .detail(json!({"field": "attributes", "reason": "enum", "allowed": allowed_words}))
        .next("compare_products")
    };
    let attributes = match &input.attributes {
        Some(list) if !list.is_empty() => {
            if let Some(a) = list.iter().find(|a| !allowed.contains(a)) {
                return Err(unknown(*a));
            }
            let mut out: Vec<Attribute> = Vec::new();
            for a in list {
                if !out.contains(a) {
                    out.push(*a);
                }
            }
            out
        }
        _ => allowed.clone(),
    };
    if let Some(s) = input.sort_by
        && !attributes.contains(&s)
    {
        return Err(unknown(s));
    }
    Ok((attributes, input.sort_by))
}

impl ReadOp<Cx> for CompareProducts {
    async fn read(&self, cx: &Cx, input: CompareProductsInput) -> Result<Value, ToolError> {
        let (attributes, sort_by) = plan(&cx.policy, &input)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        // The caller's own price: their tier for shoppers, everyone for operators.
        let viewer = Viewer::resolve(cx, &mut tx, None, None).await?;
        let cards = for_search::product_cards(&mut tx, &input.product_ids, &viewer).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let missing: Vec<&String> = input
            .product_ids
            .iter()
            .filter(|id| !cards.iter().any(|c| &c.id == *id))
            .collect();
        if let Some(first) = missing.first() {
            return Err(ToolError::new(
                ErrorCode::NOT_FOUND,
                format!("There is no product {first} in this store."),
            )
            .detail(json!({"entity": "product", "id": first, "ids": missing}))
            .next("search_products"));
        }
        let mut items: Vec<Value> = cards
            .iter()
            .map(|c| model::compare_item(c, &attributes))
            .collect();
        if let Some(a) = sort_by {
            model::sort_items(&mut items, a);
        }
        let total = items.len();
        Ok(json!({"items": items, "total": total}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;
    use acom_core::enums::ActorKind;

    fn input(ids: &[&str]) -> CompareProductsInput {
        CompareProductsInput {
            product_ids: ids.iter().map(|s| (*s).to_owned()).collect(),
            attributes: None,
            sort_by: None,
        }
    }

    #[test]
    fn product_count_comes_from_the_policy() {
        let p = testing::policies();
        let seven = [
            "prd_a", "prd_b", "prd_c", "prd_d", "prd_e", "prd_f", "prd_g",
        ];
        let e = plan(&p, &input(&seven)).unwrap_err();
        assert_eq!(e.code, "too_many_products");
        assert_eq!(e.detail.map(|d| d["limit"].clone()), Some(json!(6)));
        assert_eq!(
            plan(&p, &input(&["prd_a"])).unwrap_err().code,
            "invalid_input"
        );
        assert_eq!(
            plan(&p, &input(&["prd_a", "prd_a"])).unwrap_err().code,
            "invalid_input"
        );
        let (attrs, _) = plan(&p, &input(&["prd_a", "prd_b"])).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(attrs.len(), 7);
    }

    #[test]
    fn sorting_needs_a_requested_attribute() {
        let p = testing::policies();
        let mut i = input(&["prd_a", "prd_b"]);
        i.attributes = Some(vec![Attribute::Weight]);
        i.sort_by = Some(Attribute::PackedSize);
        assert_eq!(plan(&p, &i).unwrap_err().code, "unknown_attribute");
        let narrowed = p
            .with_overrides([("search", "compare:\n  attributes: [weight, price]\n")])
            .unwrap_or_else(|e| panic!("{e}"));
        i.sort_by = None;
        i.attributes = Some(vec![Attribute::Rain]);
        let e = plan(&narrowed, &i).unwrap_err();
        assert_eq!(e.code, "unknown_attribute");
        assert_eq!(
            e.detail.map(|d| d["allowed"].clone()),
            Some(json!(["weight", "price"]))
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_compares_three_jackets_by_packed_size() {
        let cx = testing::db(ActorKind::Shopper, "cus_jamie").await;
        let mut i = input(&[
            "prd_windbreaker",
            "prd_field_jacket",
            "prd_seersucker_overshirt",
        ]);
        i.attributes = Some(vec![
            Attribute::PackedSize,
            Attribute::Weight,
            Attribute::Rain,
            Attribute::Price,
        ]);
        i.sort_by = Some(Attribute::PackedSize);
        let out = CompareProducts
            .read(&cx, i)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["total"], json!(3));
        assert_eq!(out["items"][0]["product_id"], "prd_windbreaker");
        assert_eq!(out["items"][0]["color"], "Slate");
    }
}
