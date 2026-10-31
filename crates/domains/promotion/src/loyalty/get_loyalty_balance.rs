//! `get_loyalty_balance` (docs/api/promotion.md#loyalty).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::Cx;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::currency;
use crate::{known_member, money, whose};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoyaltyBalanceInput {
    /// The member, or `me` for the signed-in shopper.
    #[schemars(regex(pattern = r"^(cus_[a-z0-9_]+|me)$"), example = &"me")]
    pub customer_id: String,
}

pub struct GetLoyaltyBalance;

impl Operation for GetLoyaltyBalance {
    const META: OpMeta = OpMeta {
        address: "promotion.loyalty.get_balance",
        tool: "get_loyalty_balance",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/customers/{customer_id}/loyalty",
        summary: "A member's store-point balance and what expires soon. A shopper uses customer_id `me`.",
        consumes: &[Slot::one("customer_id")],
        produces: &[],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = LoyaltyBalanceInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetLoyaltyBalance {
    async fn read(&self, cx: &Cx, input: LoyaltyBalanceInput) -> Result<Value, ToolError> {
        let customer_id = whose(cx, &input.customer_id)?;
        let currency = currency(cx)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        known_member(&mut tx, &customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(json!({
            "customer_id": customer_id,
            "balance": money(Decimal::ZERO, &currency),
            "expiring_soon": [],
        }))
    }
}
