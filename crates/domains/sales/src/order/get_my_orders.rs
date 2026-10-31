//! `get_my_orders` (`sales.order.list_mine`): the caller's own orders, newest first.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::OrderStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{item_facts, item_json};
use crate::owner::{self, Viewer};
use crate::repo;
use crate::{NOT_SIGNED_IN, not_signed_in};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetMyOrdersInput {
    /// Only orders in these states. All when left out.
    #[schemars(example = ["delivered"])]
    pub order_status: Option<Vec<OrderStatus>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub(crate) struct GetMyOrders;

impl Operation for GetMyOrders {
    const META: OpMeta = OpMeta {
        address: "sales.order.list_mine",
        tool: "get_my_orders",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/orders/mine",
        summary: "The caller's own orders, newest first, with their lines and when they were delivered. Where returns and exchanges start.",
        consumes: &[],
        produces: &[Slot::many("order_id"), Slot::many("order_item_id")],
        checks: &[],
        errors: &[NOT_SIGNED_IN],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetMyOrdersInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetMyOrders {
    async fn read(&self, cx: &Cx, input: GetMyOrdersInput) -> Result<Value, ToolError> {
        let Viewer::Shopper(customer) = owner::viewer(&cx.principal) else {
            return Err(not_signed_in());
        };
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let filter = repo::OrderFilter {
            customer_id: Some(customer),
            statuses: input
                .order_status
                .unwrap_or_default()
                .iter()
                .map(|s| s.as_str().to_owned())
                .collect(),
            order_by: "order_date DESC, id DESC",
            limit: page.limit,
            offset: page.offset,
            ..repo::OrderFilter::default()
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (orders, total) = repo::orders(&mut tx, &filter).await?;
        let ids: Vec<String> = orders.iter().map(|o| o.id.clone()).collect();
        let items = repo::items(&mut tx, &ids).await?;
        let delivered = repo::delivered_at(&mut tx, &ids).await?;
        let facts = item_facts(&mut tx, &items).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut out = Vec::with_capacity(orders.len());
        for o in &orders {
            let lines: Vec<Value> = items
                .iter()
                .filter(|i| i.order_id == o.id)
                .map(|i| item_json(i, &facts, false))
                .collect::<Result<_, _>>()?;
            let delivered_at = (o.order_status == "delivered")
                .then(|| delivered.iter().find(|(id, _)| id == &o.id))
                .flatten()
                .map(|(_, t)| cx.time.format(*t));
            out.push(json!({
                "id": o.id, "order_number": o.order_number, "order_status": o.order_status,
                "order_date": cx.time.format(o.order_date), "ordered_item": lines,
                "delivered_at": delivered_at,
                "total": repo::money(o.total_payment_due_value, &o.total_payment_due_currency)?,
            }));
        }
        let mut v = json!({"items": out, "total": total});
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
    fn input_schema_is_closed() {
        let s = closed_schema_for::<GetMyOrdersInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
    }
}
