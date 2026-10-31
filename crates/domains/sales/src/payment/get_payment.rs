//! `get_payment` (`sales.payment.get`): how an order is paid.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::PAYMENT_NOT_FOUND;
use crate::checkout::payment_label;
use crate::order::visible_order;
use crate::{ORDER_NOT_FOUND, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPaymentInput {
    /// The order whose payment to show.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20518")]
    pub order_id: String,
}

pub(crate) struct GetPayment;

impl Operation for GetPayment {
    const META: OpMeta = OpMeta {
        address: "sales.payment.get",
        tool: "get_payment",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/orders/{order_id}/payment",
        summary: "An order's payment: state, method (card last four only), amount and installments.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("payment_id")],
        checks: &[],
        errors: &[ORDER_NOT_FOUND, PAYMENT_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPaymentInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetPayment {
    async fn read(&self, cx: &Cx, input: GetPaymentInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let order = visible_order(cx, &mut tx, &input.order_id).await?;
        let p = repo::payment(&mut tx, &order.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let p = p.ok_or_else(|| {
            ToolError::new(
                PAYMENT_NOT_FOUND,
                format!("Order {} has no payment yet.", order.id),
            )
            .detail(json!({"entity": "payment", "order_id": order.id}))
            .next("get_order")
        })?;
        Ok(json!({
            "id": p.id,
            "order_id": p.order_id,
            "payment_status": p.payment_status,
            "payment_method": p.payment_method,
            "label": payment_label(&p.payment_method, p.card_last4.as_deref()),
            "amount": repo::money(p.amount_value, &p.amount_currency)?,
            "installment_months": p.installment_months,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<GetPaymentInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["order_id"]));
    }

    /// Integration: Alex's card payment for #20402 (`DATABASE_URL`, demo seed).
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reads_alexs_card_payment() {
        let cx = crate::testdb::cx("cus_alex").await;
        let p = GetPayment
            .read(
                &cx,
                GetPaymentInput {
                    order_id: "ord_20402".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(p["label"], "Card ending 0937");
        assert_eq!(p["payment_status"], "payment_complete");
    }
}
