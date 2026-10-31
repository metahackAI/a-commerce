//! `search_sellers`: marketplace sellers by name or tax id part, status, type and grade. The
//! ids it returns are the `seller_id` every other tool takes.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::{GradeLevel, SellerStatus, SellerType};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::seller_item;
use crate::repo::{self, SellerFilter, SellerOrder};
use crate::rules::Rules;
use crate::viewer::{self, Viewer};
use crate::{INVALID_FILTER, model, store};

/// How sellers are ordered.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SellerSort {
    /// Latest application first (the default).
    Newest,
    /// By legal name, A to Z.
    Name,
    /// Best grade first; sellers without a grade last.
    Grade,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchSellersInput {
    /// Part of the legal name, or at least four digits of the tax id.
    #[schemars(length(min = 1, max = 200), example = &"Harbor")]
    pub query: Option<String>,
    /// Only sellers in this status: applied, under_review, approved, active, suspended,
    /// terminated.
    #[serde(rename = "acom:seller_status", alias = "seller_status")]
    #[schemars(example = &"active")]
    pub seller_status: Option<SellerStatus>,
    /// Only this kind of seller: marketplace (sells itself, the store takes a commission),
    /// consignment (the store sells its stock), direct_supply (the store buys and resells).
    #[serde(rename = "acom:seller_type", alias = "seller_type")]
    #[schemars(example = &"marketplace")]
    pub seller_type: Option<SellerType>,
    /// Only sellers with this grade now: platinum, gold, silver, bronze, watch.
    #[serde(rename = "acom:grade_level", alias = "grade_level")]
    #[schemars(example = &"gold")]
    pub grade_level: Option<GradeLevel>,
    /// newest: latest application first (default); name: A to Z; grade: best grade first.
    #[schemars(example = &"name")]
    pub sort: Option<SellerSort>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchSellers;

impl Operation for SearchSellers {
    const META: OpMeta = OpMeta {
        address: "partner.seller.search_sellers",
        tool: "search_sellers",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Post,
        path: "/v1/partner/sellers/search",
        summary: "Find marketplace sellers by part of the legal name or tax id, status, type or grade. The ids are the seller_id that products, orders and the other partner tools take.",
        consumes: &[],
        produces: &[Slot::many("seller_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchSellersInput;
    type Output = Value;
}

/// What a query looks for: a name part, and the tax id digits when there are enough.
fn query_parts(q: &str) -> Result<(String, Option<String>), ToolError> {
    let q = q.trim();
    if q.is_empty() {
        return Err(ToolError::new(
            INVALID_FILTER,
            "`query` is empty; give part of a name or a tax id, or leave it out.",
        )
        .detail(json!({"field": "query", "reason": "format"}))
        .next("search_sellers"));
    }
    let digits = model::normalized_tax_id(q);
    let tax = (q.chars().filter(char::is_ascii_digit).count() >= 4).then_some(digits);
    Ok((q.to_owned(), tax))
}

impl ReadOp<Cx> for SearchSellers {
    async fn read(&self, cx: &Cx, input: SearchSellersInput) -> Result<Value, ToolError> {
        viewer::staff(cx)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let (name_part, tax_id_part) = match input.query.as_deref() {
            Some(q) => {
                let (n, t) = query_parts(q)?;
                (Some(n), t)
            }
            None => (None, None),
        };
        let rules = Rules::load(&cx.policy)?;
        let mut tx = cx.read().await.map_err(store)?;
        let viewer = viewer::viewer(cx, &mut tx).await?;
        let filter = SellerFilter {
            name_part,
            tax_id_part,
            status: input.seller_status.map(|s| s.as_str().to_owned()),
            seller_type: input.seller_type.map(|s| s.as_str().to_owned()),
            grade_level: input.grade_level.map(|g| g.as_str().to_owned()),
            only: match &viewer {
                Viewer::Seller(id) => Some(id.clone()),
                Viewer::Store => None,
            },
        };
        let order = match input.sort.unwrap_or(SellerSort::Newest) {
            SellerSort::Newest => SellerOrder::Newest,
            SellerSort::Name => SellerOrder::Name,
            SellerSort::Grade => SellerOrder::Grade,
        };
        let now = cx.clock.now();
        let (total, rows) =
            repo::search_sellers(&mut tx, &filter, order, now, page.limit, page.offset).await?;
        tx.commit().await.map_err(store)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|f| seller_item(&f.seller, f.grade_level.as_deref(), &viewer, &rules))
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(next) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(next);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    #[tokio::test]
    async fn a_blank_query_is_an_invalid_filter() {
        let e = SearchSellers
            .read(&offline("adm_sj"), input(json!({"query": "  "})))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_filter");
        assert_eq!(e.next_tool.as_deref(), Some("search_sellers"));
    }

    #[tokio::test]
    async fn shoppers_cannot_search_sellers() {
        let e = SearchSellers
            .read(&offline("cus_jamie"), input(json!({})))
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    #[test]
    fn a_query_with_digits_also_looks_at_tax_ids() {
        assert_eq!(
            query_parts("4410").unwrap(),
            ("4410".to_owned(), Some("4410".to_owned()))
        );
        assert_eq!(query_parts("Harbor").unwrap(), ("Harbor".to_owned(), None));
    }
}
