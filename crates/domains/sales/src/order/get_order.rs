//! `get_order` (`sales.order.get`): one order with its lines, delivery, payment, totals,
//! coupons, gift and claims.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_promotion::service::for_sales as promotion;
use acom_store::Cx;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{item_facts, item_json, visible_order};
use crate::ORDER_NOT_FOUND;
use crate::gift_order as gift;
use crate::pricing;
use crate::repo;

/// Top-level fields of an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrderField {
    /// The order id.
    Id,
    /// The number people see.
    OrderNumber,
    /// Where the order is.
    OrderStatus,
    /// When it was placed.
    OrderDate,
    /// Who placed it.
    CustomerId,
    /// The lines.
    OrderedItem,
    /// The parcel.
    Delivery,
    /// The payment.
    Payment,
    /// Subtotal, discounts, shipping, total.
    Totals,
    /// Coupons the order recorded.
    CouponsRecorded,
    /// Cancel, return and exchange requests on the order.
    Claims,
    /// Who a gift is for, its card, and the recipient's acceptance; `null` for other orders.
    Gift,
}

impl OrderField {
    fn word(self) -> &'static str {
        match self {
            OrderField::Id => "id",
            OrderField::OrderNumber => "order_number",
            OrderField::OrderStatus => "order_status",
            OrderField::OrderDate => "order_date",
            OrderField::CustomerId => "customer_id",
            OrderField::Claims => "claims",
            OrderField::Gift => "gift",
            OrderField::OrderedItem => "ordered_item",
            OrderField::Delivery => "delivery",
            OrderField::Payment => "payment",
            OrderField::Totals => "totals",
            OrderField::CouponsRecorded => "coupons_recorded",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetOrderInput {
    /// The order (`get_my_orders`, `search_orders`).
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20402")]
    pub order_id: String,
    /// Only these top-level fields. All when left out.
    #[schemars(example = ["ordered_item", "payment"])]
    pub fields: Option<Vec<OrderField>>,
}

pub(crate) struct GetOrder;

impl Operation for GetOrder {
    const META: OpMeta = OpMeta {
        address: "sales.order.get",
        tool: "get_order",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/orders/{order_id}",
        summary: "One order: lines with their own status, delivery, payment (card last four only), totals and recorded coupons. Operators see every order, shoppers their own.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::many("order_item_id"), Slot::one("delivery_id")],
        checks: &[],
        errors: &[ORDER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetOrderInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetOrder {
    async fn read(&self, cx: &Cx, input: GetOrderInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let o = visible_order(cx, &mut tx, &input.order_id).await?;
        let ids = vec![o.id.clone()];
        let items = repo::items(&mut tx, &ids).await?;
        let facts = item_facts(&mut tx, &items).await?;
        let delivered = repo::delivered_at(&mut tx, &ids).await?;
        let record = repo::placed_record(&mut tx, &o.id)
            .await?
            .unwrap_or(Value::Null);
        let parcel = match crate::service::providers::deliveries() {
            Some(read) => read(&mut tx, &o.id).await?,
            None => None,
        };
        let claims = match crate::service::providers::claims() {
            Some(read) => read(&mut tx, &o.id).await?,
            None => Vec::new(),
        };
        let payment = repo::payment(&mut tx, &o.id).await?;
        let acceptance = match o.gift_options {
            Some(_) => repo::gift_order_of(&mut tx, &o.id).await?,
            None => None,
        };
        // Orders placed before discounts were recorded in full: one line per promotion.
        let discounts = match record.get("discounts") {
            Some(d) if d.is_array() => d.clone(),
            _ => {
                let mut by: Vec<(Option<String>, Decimal, String)> = Vec::new();
                for i in items.iter().filter(|i| i.discount_value > Decimal::ZERO) {
                    match by.iter_mut().find(|(p, _, _)| p == &i.promotion_id) {
                        Some((_, a, _)) => *a += i.discount_value,
                        None => by.push((
                            i.promotion_id.clone(),
                            i.discount_value,
                            i.discount_currency.clone(),
                        )),
                    }
                }
                let promotion_ids: Vec<String> =
                    by.iter().filter_map(|(p, _, _)| p.clone()).collect();
                let names = promotion::promotion_names(&mut tx, &promotion_ids).await?;
                let mut out = Vec::new();
                for (p, amount, currency) in by {
                    let label = p
                        .as_ref()
                        .and_then(|p| names.iter().find(|(id, _)| id == p).map(|(_, n)| n.clone()))
                        .or_else(|| o.discount_code.clone())
                        .unwrap_or_else(|| "Discount".to_owned());
                    out.push(
                        json!({"label": label, "amount": repo::money(amount, &currency)?,
                                    "promotion_id": p}),
                    );
                }
                json!(out)
            }
        };
        let coupons = match record.get("coupons_recorded") {
            Some(c) if c.is_array() => c.clone(),
            _ => {
                let ids: Vec<String> = o.coupon_id.iter().cloned().collect();
                let facts = promotion::coupon_facts(&mut tx, &ids).await?;
                json!(
                    facts
                        .iter()
                        .map(|c| json!({"coupon_id": c.coupon_id, "code": c.code,
                                    "promotion_id": c.promotion_id}))
                        .collect::<Vec<_>>()
                )
            }
        };
        tx.commit().await.map_err(ToolError::from)?;

        let delivered_at = (o.order_status == "delivered")
            .then(|| delivered.first().map(|(_, t)| cx.time.format(*t)))
            .flatten();
        let delivery = match parcel {
            Some(p) => json!({
                "id": p.id, "delivery_address": p.delivery_address,
                "expected_arrival_until": p.expected_arrival_until.map(|t| cx.time.format(t)),
                "delivery_state": p.delivery_state,
                "delivered_at": p.delivered_at.map(|t| cx.time.format(t)),
            }),
            // Without fulfillment's reader: what the order recorded at checkout.
            None => {
                let d = &record["delivery"];
                json!({
                    "id": null,
                    "delivery_address": d.get("delivery_address").cloned().unwrap_or(Value::Null),
                    "expected_arrival_until": null,
                    "delivery_state": null,
                    "delivered_at": delivered_at,
                })
            }
        };
        let payment = match payment {
            Some(p) => json!({"payment_method": p.payment_method, "card_last4": p.card_last4,
                              "payment_status": p.payment_status}),
            None => Value::Null,
        };
        let lines: Vec<Value> = items
            .iter()
            .map(|i| item_json(i, &facts, true))
            .collect::<Result<_, _>>()?;
        let out = json!({
            "id": o.id, "order_number": o.order_number, "order_status": o.order_status,
            "order_date": cx.time.format(o.order_date), "customer_id": o.customer_id,
            "ordered_item": lines,
            "delivery": delivery,
            "payment": payment,
            "totals": {
                "subtotal": repo::money(o.subtotal_value, &o.subtotal_currency)?,
                "discounts": discounts,
                "shipping": repo::money(o.shipping_value, &o.shipping_currency)?,
                "gift_wrap": match (o.gift_wrap_value, o.gift_wrap_currency.as_deref()) {
                    (Some(v), Some(c)) => json!(repo::money(v, c)?),
                    _ => Value::Null,
                },
                "total": repo::money(o.total_payment_due_value, &o.total_payment_due_currency)?,
            },
            "gift": o.gift_options.as_ref().map(|g| {
                gift::order_gift(g, acceptance.as_ref(), cx.clock.now(), &cx.time)
            }),
            "coupons_recorded": coupons,
            "claims": claims.iter().map(|c| json!({
                "id": c.id, "claim_type": c.claim_type, "claim_status": c.claim_status,
                "order_item_ids": c.order_item_ids,
                "requested_at": cx.time.format(c.requested_at),
            })).collect::<Vec<_>>(),
        });
        let fields: Vec<String> = input
            .fields
            .unwrap_or_default()
            .iter()
            .map(|f| f.word().to_owned())
            .collect();
        Ok(pricing::pick(out, &fields))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::get_my_orders::{GetMyOrders, GetMyOrdersInput};
    use crate::order::search_orders::{SearchOrders, SearchOrdersInput};
    use acom_contract::schema::{closed_schema_for, violations};
    use acom_core::enums::OrderStatus;

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<GetOrderInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
    }

    /// Integration: order reads for Alex's #20402 (`DATABASE_URL`, demo seed).
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reads_alexs_order() {
        let alex = crate::testdb::cx("cus_alex").await;
        let mine = GetMyOrders
            .read(
                &alex,
                GetMyOrdersInput {
                    order_status: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap();
        assert!(
            mine["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["id"] == "ord_20402")
        );
        let o = GetOrder
            .read(
                &alex,
                GetOrderInput {
                    order_id: "ord_20402".into(),
                    fields: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(o["totals"]["total"]["value"].to_string(), "312.00");
        assert_eq!(o["payment"]["card_last4"], "0937");
        let jamie = crate::testdb::cx("cus_jamie").await;
        let e = GetOrder
            .read(
                &jamie,
                GetOrderInput {
                    order_id: "ord_20402".into(),
                    fields: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let staff = crate::testdb::cx("adm_sj").await;
        let found = SearchOrders
            .read(
                &staff,
                SearchOrdersInput {
                    order_status: Some(vec![OrderStatus::Delivered]),
                    customer_id: Some("cus_alex".into()),
                    order_number: None,
                    customer: None,
                    order_date_from: None,
                    order_date_to: None,
                    seller_id: None,
                    sort: None,
                    limit: None,
                    cursor: None,
                    fields: None,
                },
            )
            .await
            .unwrap();
        assert!(found["total"].as_i64().unwrap() >= 1);
    }
}
