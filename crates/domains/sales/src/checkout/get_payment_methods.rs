//! `get_payment_methods` (`sales.checkout.list_payment_methods`): the methods that can pay
//! every line of the caller's own cart.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::pricing::{self, DeliveryChoice};
use crate::rules;
use crate::{CART_EMPTY, cart_empty, owner};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPaymentMethodsInput {}

pub(crate) struct GetPaymentMethods;

impl Operation for GetPaymentMethods {
    const META: OpMeta = OpMeta {
        address: "sales.checkout.list_payment_methods",
        tool: "get_payment_methods",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/checkout/payment-methods",
        summary: "The payment methods that work for every line of the caller's own cart, with installments and whether loyalty points can be added. The next tool when a payment check fails.",
        consumes: &[Slot::one("cart_id")],
        produces: &[],
        checks: &[
            "payment.methods",
            "payment.primary_methods_per_order",
            "payment.installment",
        ],
        errors: &[CART_EMPTY],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPaymentMethodsInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetPaymentMethods {
    async fn read(&self, cx: &Cx, _: GetPaymentMethodsInput) -> Result<Value, ToolError> {
        let owners = owner::owners(&cx.principal)?;
        let limits = rules::cart_limits(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let carts = pricing::load_carts(&mut tx, &owners, limits).await?;
        let view = pricing::view(&mut tx, &carts, &DeliveryChoice::CartDefault, None)
            .await?
            .map_err(|_| cart_empty())?;
        tx.commit().await.map_err(ToolError::from)?;
        if view.lines.is_empty() {
            return Err(cart_empty());
        }
        let total = view.options.len();
        Ok(json!({"items": view.options, "total": total}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        assert!(violations(&closed_schema_for::<GetPaymentMethodsInput>()).is_empty());
    }

    /// Integration: Jamie's payment methods from the database (`DATABASE_URL`).
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn lists_jamies_methods_from_the_database() {
        let cx = crate::testdb::cx("cus_jamie").await;
        match GetPaymentMethods.read(&cx, GetPaymentMethodsInput {}).await {
            Ok(v) => assert!(v["total"].as_u64().unwrap() >= 1),
            Err(e) => assert_eq!(e.code, "cart_empty"),
        }
    }
}
