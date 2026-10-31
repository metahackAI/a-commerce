//! `list_loyalty_transactions` (docs/api/promotion.md#loyalty).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{LoyaltyTransactionType, ledger};
use crate::{known_member, money, whose};

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
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let kinds: Vec<&str> = input.r#type.iter().flatten().map(|k| k.as_str()).collect();
        let customer_id = whose(cx, &input.customer_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        known_member(&mut tx, &customer_id).await?;
        let mut entries = ledger(&mut tx, &customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        // Newest first; the kinds asked for only.
        entries.reverse();
        entries.retain(|e| kinds.is_empty() || kinds.contains(&e.transaction_type.as_str()));
        let total = entries.len() as i64;
        let from = usize::try_from(page.offset).unwrap_or(usize::MAX);
        let items: Vec<Value> = entries
            .iter()
            .skip(from)
            .take(usize::try_from(page.limit).unwrap_or(20))
            .map(|e| {
                json!({
                    "id": e.id,
                    "type": e.transaction_type,
                    "amount": money(e.amount_value, e.amount_currency.trim()),
                    "order_id": e.order_id,
                    "at": cx.time.format(e.start_time),
                    "expires": e.expires.map(|d| d.to_string()),
                    "description": e.description,
                })
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(items.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
