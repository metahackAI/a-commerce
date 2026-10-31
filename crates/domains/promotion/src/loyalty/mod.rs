//! `get_loyalty_balance`, `list_loyalty_transactions` (docs/api/promotion.md#loyalty).
//!
//! Store points are a ledger of transactions and the balance is their sum. The schema has no
//! ledger table yet (`migrations/0005_promotion.sql`), so every member's ledger is empty: the
//! balance is zero in the currency of the loyalty policy and there are no transactions. The
//! reads still check who may look and that the member exists, and keep the spec's shapes, so
//! callers do not change when the ledger arrives.

pub(crate) mod get_loyalty_balance;
pub(crate) mod list_loyalty_transactions;

use acom_contract::{ErrorCode, ToolError};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

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
