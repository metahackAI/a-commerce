//! `list_wishlist`.

use crate::access::customer_for;
use crate::person::customer_not_found;
use crate::service::providers;
use crate::{CUSTOMER_NOT_FOUND, INVALID_FILTER, repo};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListWishlistInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
    /// Only products in this category. Categories are not available here yet; the call is
    /// refused.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub category_id: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListWishlist;

impl Operation for ListWishlist {
    const META: OpMeta = OpMeta {
        address: "customer.wishlist.list",
        tool: "list_wishlist",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/{customer_id}/wishlist",
        summary: "The products a member keeps for later, newest first, with today's price and availability.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::many("product_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND, INVALID_FILTER],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListWishlistInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListWishlist {
    async fn read(&self, cx: &Cx, input: ListWishlistInput) -> Result<Value, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        if input.category_id.is_some() {
            return Err(ToolError::new(
                INVALID_FILTER,
                "The wishlist cannot be filtered by category yet; list it without `category_id`.",
            )
            .detail(json!({"field": "category_id", "reason": "unsupported"}))
            .next("list_wishlist"));
        }
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if repo::person(&mut tx, &customer_id).await?.is_none() {
            return Err(customer_not_found(&customer_id));
        }
        let (total, rows) = repo::wishlist(&mut tx, &customer_id, page.limit, page.offset).await?;
        let ids: Vec<String> = rows.iter().map(|r| r.product_group_id.clone()).collect();
        let cards = match providers::product_cards() {
            Some(read) => Some(read(&mut tx, &ids).await?),
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|r| {
                let mut item = json!({"product_id": r.product_group_id,
                                      "added_at": cx.time.format(r.created_at)});
                let card = cards
                    .as_ref()
                    .and_then(|c| c.iter().find(|c| c.product_id == r.product_group_id));
                if let Some(c) = card {
                    item["name"] = json!(c.name);
                    item["price"] = json!(c.price)["value"].clone();
                    item["price_currency"] = json!(c.price.currency);
                    item["availability"] = json!(c.availability);
                }
                item
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<ListWishlistInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }
}
