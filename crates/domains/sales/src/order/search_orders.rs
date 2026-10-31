//! `search_orders` (`sales.order.search`): operators find orders the way people name them.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError, fields,
};
use acom_core::enums::OrderStatus;
use acom_customer::service::for_sales as customer;
use acom_store::{Cx, Page};
use chrono::Duration;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::owner::{self, Viewer};
use crate::repo;
use crate::{INVALID_FILTER, parse_time};

/// How to order the search results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum OrderSort {
    /// Oldest first.
    #[serde(rename = "order_date")]
    OrderDate,
    /// Newest first (the default).
    #[serde(rename = "-order_date")]
    OrderDateDescending,
    /// Smallest total first.
    #[serde(rename = "total")]
    Total,
    /// Largest total first.
    #[serde(rename = "-total")]
    TotalDescending,
}

/// Fields of a search result row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrderRowField {
    /// The order id.
    Id,
    /// The number people see.
    OrderNumber,
    /// Where the order is.
    OrderStatus,
    /// Who placed it.
    CustomerId,
    /// When it was placed.
    OrderDate,
    /// What the shopper pays.
    Total,
}

impl OrderRowField {
    fn word(self) -> &'static str {
        match self {
            OrderRowField::Id => "id",
            OrderRowField::OrderNumber => "order_number",
            OrderRowField::OrderStatus => "order_status",
            OrderRowField::CustomerId => "customer_id",
            OrderRowField::OrderDate => "order_date",
            OrderRowField::Total => "total",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchOrdersInput {
    /// Only orders in these states.
    #[schemars(example = ["processing", "payment_due"])]
    pub order_status: Option<Vec<OrderStatus>>,
    /// Only this customer's orders.
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_alex")]
    pub customer_id: Option<String>,
    /// The order number people see, with or without `#`.
    #[schemars(regex(pattern = r"^#?[0-9]{1,12}$"), example = &"20402")]
    pub order_number: Option<String>,
    /// The customer as people say it: an email, a name, part of either, or a phone number.
    #[schemars(length(min = 2, max = 100), example = &"alex@example.com")]
    pub customer: Option<String>,
    /// Placed on or after this day (store time).
    #[schemars(example = &"2026-10-01")]
    pub order_date_from: Option<String>,
    /// Placed on or before this day (store time).
    #[schemars(example = &"2026-11-06")]
    pub order_date_to: Option<String>,
    /// Only orders of this seller. Sellers are not recorded on orders yet.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_northwind")]
    pub seller_id: Option<String>,
    /// Order of the results. Default newest first.
    #[schemars(example = &"-order_date")]
    pub sort: Option<OrderSort>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
    /// Only these fields of each row. All when left out.
    #[schemars(example = ["id", "order_number", "total"])]
    pub fields: Option<Vec<OrderRowField>>,
}

pub(crate) struct SearchOrders;

/// How many people a `customer` filter may name before it is too broad to be what was meant.
const MATCHED_PEOPLE: i64 = 500;

impl Operation for SearchOrders {
    const META: OpMeta = OpMeta {
        address: "sales.order.search",
        tool: "search_orders",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/orders",
        summary: "Operators' order search by order number, customer (email or name), state and date, sorted by date or total.",
        consumes: &[],
        produces: &[Slot::many("order_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchOrdersInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchOrders {
    async fn read(&self, cx: &Cx, input: SearchOrdersInput) -> Result<Value, ToolError> {
        if owner::viewer(&cx.principal) != Viewer::Staff {
            return Err(
                ToolError::new(ErrorCode::FORBIDDEN, "Order search is for operators.")
                    .detail(json!({"reason": "role"}))
                    .next("get_my_orders"),
            );
        }
        if input.seller_id.is_some() {
            return Err(ToolError::new(
                INVALID_FILTER,
                "Orders do not record a seller yet, so they cannot be filtered by one.",
            )
            .detail(json!({"field": "seller_id", "reason": "unsupported"}))
            .next("list_enums"));
        }
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let from = parse_time(cx, "order_date_from", input.order_date_from.as_deref())?;
        let until = parse_time(cx, "order_date_to", input.order_date_to.as_deref())?
            .map(|t| t + Duration::days(1));
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let customer_ids = match input.customer.as_deref().map(str::trim) {
            Some(text) if !text.is_empty() => {
                let (ids, more) = customer::people_matching(&mut tx, text, MATCHED_PEOPLE).await?;
                if more {
                    return Err(ToolError::new(
                        INVALID_FILTER,
                        format!(
                            "“{text}” matches more than {MATCHED_PEOPLE} people; give an email or a fuller name."
                        ),
                    )
                    .detail(json!({"field": "customer", "reason": "too_broad"}))
                    .next("search_members"));
                }
                Some(ids)
            }
            _ => None,
        };
        let filter = repo::OrderFilter {
            customer_id: input.customer_id,
            order_number: input
                .order_number
                .map(|n| n.trim_start_matches('#').to_owned()),
            customer_ids,
            statuses: input
                .order_status
                .unwrap_or_default()
                .iter()
                .map(|s| s.as_str().to_owned())
                .collect(),
            from,
            until,
            order_by: match input.sort {
                Some(OrderSort::OrderDate) => "order_date, id",
                Some(OrderSort::Total) => "total_payment_due_value, id",
                Some(OrderSort::TotalDescending) => "total_payment_due_value DESC, id",
                _ => "order_date DESC, id DESC",
            },
            limit: page.limit,
            offset: page.offset,
        };
        let (orders, total) = repo::orders(&mut tx, &filter).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let fields: Vec<String> = input
            .fields
            .unwrap_or_default()
            .iter()
            .map(|f| f.word().to_owned())
            .collect();
        let items: Vec<Value> = orders
            .iter()
            .map(|o| {
                Ok(fields::pick(
                    json!({"id": o.id, "order_number": o.order_number,
                           "order_status": o.order_status, "customer_id": o.customer_id,
                           "order_date": cx.time.format(o.order_date),
                           "total": repo::money(o.total_payment_due_value, &o.total_payment_due_currency)?}),
                    &fields,
                ))
            })
            .collect::<Result<_, ToolError>>()?;
        let mut v = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(orders.len(), total) {
            v["next_cursor"] = json!(c);
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed_with_sorts_as_words() {
        let s = closed_schema_for::<SearchOrdersInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["properties"]["sort"]["enum"],
            json!(["order_date", "-order_date", "total", "-total"])
        );
    }

    /// Integration: an operator finds orders the way people name them: by number with or
    /// without `#`, and by the customer's email or name (`DATABASE_URL`, demo seed).
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn operators_find_orders_by_number_and_customer() {
        let sj = crate::testdb::cx("adm_sj").await;
        let search = |order_number: Option<&str>, customer: Option<&str>| SearchOrdersInput {
            order_status: None,
            customer_id: None,
            order_number: order_number.map(str::to_owned),
            customer: customer.map(str::to_owned),
            order_date_from: None,
            order_date_to: None,
            seller_id: None,
            sort: None,
            limit: None,
            cursor: None,
            fields: None,
        };
        let ids = |v: &Value| -> Vec<String> {
            v["items"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|o| o["id"].as_str().map(str::to_owned))
                .collect()
        };
        let by_number = SearchOrders
            .read(&sj, search(Some("#20402"), None))
            .await
            .unwrap();
        assert_eq!(ids(&by_number), ["ord_20402"]);
        let by_email = SearchOrders
            .read(&sj, search(None, Some("ALEX.KIM@example.com")))
            .await
            .unwrap();
        assert!(
            ids(&by_email).contains(&"ord_20402".to_owned()),
            "{by_email}"
        );
        let both = SearchOrders
            .read(&sj, search(Some("20402"), Some("Morgan Reyes")))
            .await
            .unwrap();
        assert_eq!(both["total"], 0, "the number is Alex's, not Morgan's");
        let broad = SearchOrders
            .read(&sj, search(None, Some("example")))
            .await
            .unwrap_err();
        assert_eq!(
            broad.detail.as_deref().map(|d| d["reason"].clone()),
            Some(json!("too_broad"))
        );
    }
}
