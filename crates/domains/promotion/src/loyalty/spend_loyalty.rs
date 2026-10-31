//! `spend_loyalty` (docs/api/promotion.md#loyalty): store points spent on an order, added to the
//! ledger as a debit. An internal write commerce runs while an order is placed; the agent never
//! sees it. Once per member and order.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_store::{Cx, TxCx};
use chrono::Days;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    INSUFFICIENT_BALANCE, LoyaltyTransactionType, NewEntry, balance, insert, ledger, offset_of,
    spend_on, store_date,
};
use crate::model::money_text;
use crate::rules::LoyaltyRules;
use crate::service::for_sales::orders;
use crate::{check_id, known_member, money, not_found, repo, whose};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpendLoyaltyInput {
    /// The member, or `me` for the signed-in shopper.
    #[schemars(regex(pattern = r"^(cus_[a-z0-9_]+|me)$"), example = &"cus_jamie")]
    pub customer_id: String,
    /// The order the points pay towards.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20518")]
    pub order_id: String,
    /// The points to spend, as money; part of the balance is fine.
    #[schemars(example = json!({"value": 5.00, "currency": "USD"}))]
    pub amount: Money,
}

/// What a run of `spend_loyalty` writes; `already` when the order spent points before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpendRun {
    pub entry: NewEntry,
    pub already: bool,
}

pub struct SpendLoyalty;

impl Operation for SpendLoyalty {
    const META: OpMeta = OpMeta {
        address: "promotion.loyalty.spend",
        tool: "spend_loyalty",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/customers/{customer_id}/loyalty/spend",
        summary: "Internal: while an order is placed, take the store points it uses off the member's balance (once per order).",
        consumes: &[Slot::one("customer_id"), Slot::one("order_id")],
        produces: &[Slot::one("loyalty_transaction_id")],
        checks: &["loyalty.spend"],
        errors: &[INSUFFICIENT_BALANCE, ErrorCode::NOT_FOUND],
        audience: Audience::Internal,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SpendLoyaltyInput;
    type Output = Value;
}

impl WriteOp<Cx> for SpendLoyalty {
    type Plan = SpendRun;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SpendLoyaltyInput,
    ) -> Result<Planned<SpendRun>, ToolError> {
        let customer_id = whose(cx, &input.customer_id)?;
        check_id("order_id", "ord", &input.order_id, false)?;
        let rules = LoyaltyRules::read(&cx.policy)?;
        let amount = input.amount;
        if amount.currency != cx.currency
            || amount.value <= Decimal::ZERO
            || amount.value.normalize().scale() > amount.currency.minor_units()
        {
            return Err(ToolError::invalid_input(format!(
                "`amount` must be above zero, to the cent, in {}.",
                cx.currency
            ))
            .detail(json!({"field": "amount", "reason": "range"})));
        }
        let amount = amount.rounded();
        let today = store_date(&cx.time, cx.clock.now());
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        known_member(&mut tx, &customer_id).await?;
        if let Some(done) = spend_on(&mut tx, &customer_id, &input.order_id).await? {
            tx.commit().await.map_err(ToolError::from)?;
            let amount = super::stored_money(done.amount_value, &done.amount_currency)?;
            return Ok(Planned {
                plan: SpendRun {
                    entry: NewEntry {
                        id: done.id,
                        customer_id,
                        transaction_type: LoyaltyTransactionType::Spend,
                        amount,
                        order_id: Some(input.order_id),
                        source_transaction_id: None,
                        expires: None,
                        description: done.description,
                        agent_id: None,
                    },
                    already: true,
                },
                changes: Vec::new(),
                checks: Vec::new(),
                basis: Vec::new(),
                approval: ApprovalNeed::NotNeeded,
            });
        }
        // The order's minimum, only when the policy sets one.
        if rules.spend_min_order.value > Decimal::ZERO {
            let order = orders(&mut tx, std::slice::from_ref(&input.order_id))
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| not_found("order", &input.order_id, "search_orders"))?;
            if order.subtotal.value < rules.spend_min_order.value {
                return Err(ToolError::new(
                    INSUFFICIENT_BALANCE,
                    format!(
                        "Points can be spent on orders of {} or more.",
                        money_text(
                            rules.spend_min_order.value,
                            rules.spend_min_order.currency.as_str()
                        )
                    ),
                )
                .detail(json!({"rule": "loyalty.spend.min_order"})));
            }
        }
        let entries = ledger(&mut tx, &customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let have = balance(&entries, today);
        let cur = amount.currency.as_str();
        if amount.value > have {
            return Err(ToolError::new(
                INSUFFICIENT_BALANCE,
                format!(
                    "The member has {} in store points, less than {}.",
                    money_text(have, cur),
                    money_text(amount.value, cur)
                ),
            )
            .detail(json!({"rule": "loyalty.spend", "balance": money(have, cur)})));
        }
        if !rules.partial_allowed && amount.value != have {
            return Err(ToolError::new(
                INSUFFICIENT_BALANCE,
                format!("Points are spent all at once: {}.", money_text(have, cur)),
            )
            .detail(
                json!({"rule": "loyalty.spend.partial_allowed", "balance": money(have, cur)}),
            ));
        }
        let entry = NewEntry {
            id: repo::new_id("lyt"),
            customer_id,
            transaction_type: LoyaltyTransactionType::Spend,
            amount,
            order_id: Some(input.order_id.clone()),
            source_transaction_id: None,
            expires: None,
            description: Some(format!("Spent on order {}", input.order_id)),
            agent_id: Some(cx.principal.subject.clone()),
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "loyalty_transaction".into(),
                id: String::new(),
                from: json!({"balance": money(have, cur)}),
                to: json!({"type": "spend", "amount": money(amount.value, cur),
                           "balance": money(have - amount.value, cur)}),
                note: None,
            }],
            checks: vec![Check {
                rule: "loyalty.spend".into(),
                passed: true,
                detail: Some(format!(
                    "{} of {} spent",
                    money_text(amount.value, cur),
                    money_text(have, cur)
                )),
                fix_tool: None,
            }],
            plan: SpendRun {
                entry,
                already: false,
            },
            basis: Vec::new(),
            approval: ApprovalNeed::NotNeeded,
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &SpendRun) -> Result<Committed, ToolError> {
        let order = run.entry.order_id.clone().unwrap_or_default();
        let id = match spend_on(tx, &run.entry.customer_id, &order).await? {
            Some(done) => done.id,
            None => {
                insert(tx, &run.entry).await?;
                run.entry.id.clone()
            }
        };
        Ok(Committed {
            id: id.clone(),
            produced: serde_json::Map::from_iter([(
                "loyalty_transaction_id".to_owned(),
                json!(id),
            )]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &SpendRun,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let order = run.entry.order_id.clone().unwrap_or_default();
        let stored = spend_on(&mut tx, &run.entry.customer_id, &order).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = stored.is_some_and(|e| e.id == done.id);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("no spend recorded on {order}")]
            },
        })
    }

    /// Only the change-set runner rolls a failed set back this way; a placed order's points
    /// come back through a restore when it is cancelled or returned (`loyalty.refund_restore`).
    async fn undo(&self, tx: &mut TxCx, run: &SpendRun, done: &Committed) -> Result<(), ToolError> {
        if run.already || offset_of(tx, &done.id, "restore").await?.is_some() {
            return Ok(());
        }
        let rules = LoyaltyRules::read(&tx.policy)?;
        let today = store_date(&tx.time, tx.clock.now());
        insert(
            tx,
            &NewEntry {
                id: repo::new_id("lyt"),
                customer_id: run.entry.customer_id.clone(),
                transaction_type: LoyaltyTransactionType::Restore,
                amount: run.entry.amount,
                order_id: run.entry.order_id.clone(),
                source_transaction_id: Some(done.id.clone()),
                expires: Some(today + Days::new(u64::try_from(rules.window_days).unwrap_or(0))),
                description: Some(format!("Gives back {}", done.id)),
                agent_id: Some(tx.principal.subject.clone()),
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn is_internal_with_the_spec_inputs() {
        assert_eq!(SpendLoyalty::META.audience, Audience::Internal);
        let s = closed_schema_for::<SpendLoyaltyInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(s["required"], json!(["customer_id", "order_id", "amount"]));
    }
}
