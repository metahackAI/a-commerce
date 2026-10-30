//! `get_loyalty_balance`, `list_loyalty_transactions` (docs/api/promotion.md#loyalty).
//!
//! Store points are a ledger of transactions and the balance is their sum. The schema has no
//! ledger table yet (`migrations/0005_promotion.sql`), so every member's ledger is empty: the
//! balance is zero in the currency of the loyalty policy and there are no transactions. The
//! reads still check who may look and that the member exists, and keep the spec's shapes, so
//! callers do not change when the ledger arrives.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::{Cx, Page};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{known_member, money, whose};

/// The ledger's currency: the one the loyalty policy states its amounts in.
fn currency(cx: &Cx) -> Result<String, ToolError> {
    cx.policy
        .lookup("promotion.loyalty.spend.min_order.currency")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            ToolError::new(
                ErrorCode::INTERNAL,
                "The policy value `promotion.loyalty.spend.min_order` is missing.",
            )
        })
}

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

/// Kinds of store-point transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LoyaltyTransactionType {
    /// Earned on a delivered order.
    Accrual,
    /// Used on an order.
    Spend,
    /// Ran out unused.
    Expire,
    /// Given by the store.
    Grant,
    /// Given back after a cancellation or return.
    Restore,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoyaltyTransactionsInput {
    /// The member, or `me` for the signed-in shopper.
    #[schemars(regex(pattern = r"^(cus_[a-z0-9_]+|me)$"), example = &"me")]
    pub customer_id: String,
    /// Only these kinds of transactions.
    #[schemars(example = ["accrual", "spend"])]
    pub r#type: Option<Vec<LoyaltyTransactionType>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListLoyaltyTransactions;

impl Operation for ListLoyaltyTransactions {
    const META: OpMeta = OpMeta {
        address: "promotion.loyalty.list_transactions",
        tool: "list_loyalty_transactions",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/customers/{customer_id}/loyalty/transactions",
        summary: "A member's store-point ledger: earned, spent, expired, granted and restored amounts.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::many("loyalty_transaction_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = LoyaltyTransactionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListLoyaltyTransactions {
    async fn read(&self, cx: &Cx, input: LoyaltyTransactionsInput) -> Result<Value, ToolError> {
        Page::new(input.limit, input.cursor.as_deref())?;
        // Every kind filters the same, still empty, ledger.
        let _kinds = input.r#type.unwrap_or_default();
        let customer_id = whose(cx, &input.customer_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        known_member(&mut tx, &customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(json!({"items": [], "total": 0}))
    }
}
