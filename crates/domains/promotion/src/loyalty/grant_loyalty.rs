//! `grant_loyalty` (docs/api/promotion.md#loyalty): store points given by hand, added to the
//! ledger. Points are worth money, so a person always approves; over `thresholds.loyalty_grant`
//! the approval sentence says so, and over `loyalty.grant.max_amount` the grant is refused.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::Money;
use acom_store::{Cx, TxCx};
use chrono::{Days, NaiveDate};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{LoyaltyTransactionType, NewEntry, OVER_LIMIT, entry, insert, offset_of, store_date};
use crate::model::money_text;
use crate::rules::LoyaltyRules;
use crate::{check_id, known_member, money, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrantLoyaltyInput {
    /// The member (`search_members`).
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_alex")]
    pub customer_id: String,
    /// The points, as money in the store's currency.
    #[schemars(example = json!({"value": 5.00, "currency": "USD"}))]
    pub amount: Money,
    /// Why, in a sentence; the member and the change set see it.
    #[schemars(length(min = 1, max = 500), example = &"Sorry for the late parcel.")]
    pub reason: String,
    /// Last day the points can be spent (`YYYY-MM-DD`, store time). Default: today plus the
    /// policy's `loyalty.expiry.window_days`.
    #[schemars(regex(pattern = r"^\d{4}-\d{2}-\d{2}$"), example = &"2027-11-06")]
    pub expires: Option<String>,
}

/// What a run of `grant_loyalty` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrantRun {
    pub entry: NewEntry,
}

pub struct GrantLoyalty;

impl Operation for GrantLoyalty {
    const META: OpMeta = OpMeta {
        address: "promotion.loyalty.grant",
        tool: "grant_loyalty",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/promotion/customers/{customer_id}/loyalty/grant",
        summary: "Give a member store points by hand, with a reason and an expiry day. Points are worth money, so a person always approves.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::one("loyalty_transaction_id")],
        checks: &["loyalty.expiry", "thresholds.loyalty_grant"],
        errors: &[OVER_LIMIT, ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = GrantLoyaltyInput;
    type Output = Value;
}

impl WriteOp<Cx> for GrantLoyalty {
    type Plan = GrantRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: GrantLoyaltyInput,
    ) -> Result<Planned<GrantRun>, ToolError> {
        check_id("customer_id", "cus", &input.customer_id, false)?;
        let rules = LoyaltyRules::read(&cx.policy)?;
        let reason = input.reason.trim().to_owned();
        if reason.is_empty() {
            return Err(ToolError::invalid_input("`reason` must not be empty.")
                .detail(json!({"field": "reason", "reason": "missing"})));
        }
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
        if amount.currency == rules.grant_max.currency && amount.value > rules.grant_max.value {
            return Err(ToolError::new(
                OVER_LIMIT,
                format!(
                    "{} is over the {} a single grant may give.",
                    money_text(amount.value, amount.currency.as_str()),
                    money_text(rules.grant_max.value, rules.grant_max.currency.as_str())
                ),
            )
            .detail(
                json!({"limit": money(rules.grant_max.value, rules.grant_max.currency.as_str()),
                           "actual": money(amount.value, amount.currency.as_str()),
                           "policy_key": "loyalty.grant.max_amount"}),
            ));
        }
        let today = store_date(&cx.time, cx.clock.now());
        let default_expiry = today + Days::new(u64::try_from(rules.window_days).unwrap_or(0));
        let expires = match &input.expires {
            None => default_expiry,
            Some(s) => {
                let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
                    ToolError::invalid_input(format!(
                        "`expires` must be a date like 2027-11-06; got `{s}`."
                    ))
                    .detail(json!({"field": "expires", "reason": "format"}))
                })?;
                if d < today {
                    return Err(ToolError::invalid_input("`expires` is in the past.")
                        .detail(json!({"field": "expires", "reason": "past"})));
                }
                d
            }
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let member = known_member(&mut tx, &input.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let shown = money_text(amount.value, amount.currency.as_str());
        let facts = json!({"amount": money(amount.value, amount.currency.as_str())});
        let threshold = cx.policy.approval_for(Self::META.tool, &facts);
        let reason_text = match threshold.reason.as_deref() {
            Some(why) if threshold.needed => {
                format!("Grants {shown} store points; {}", why.to_lowercase())
            }
            _ => format!("Grants {shown} store points"),
        };
        let checks = vec![
            Check {
                rule: "loyalty.expiry".into(),
                passed: true,
                detail: Some(if input.expires.is_none() {
                    format!("Expires {expires}, {} days from today", rules.window_days)
                } else {
                    format!("Expires {expires}")
                }),
                fix_tool: None,
            },
            Check {
                rule: "thresholds.loyalty_grant".into(),
                passed: true,
                detail: Some(if threshold.needed {
                    format!(
                        "{} — the approver is told",
                        threshold.reason.unwrap_or_default()
                    )
                } else {
                    "Under the grant threshold".into()
                }),
                fix_tool: None,
            },
        ];
        let entry = NewEntry {
            id: repo::new_id("lyt"),
            customer_id: member.customer_id.clone(),
            transaction_type: LoyaltyTransactionType::Grant,
            amount,
            order_id: None,
            source_transaction_id: None,
            expires: Some(expires),
            description: Some(reason),
            agent_id: Some(cx.principal.subject.clone()),
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Grant,
                entity: "loyalty_transaction".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"customer_id": member.customer_id,
                           "amount": money(amount.value, amount.currency.as_str()),
                           "expires": expires.to_string()}),
                note: Some(format!("To {}", member.name)),
            }],
            plan: GrantRun { entry },
            checks,
            basis: Vec::<Basis>::new(),
            approval: ApprovalNeed::Needed {
                reason: reason_text,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &GrantRun) -> Result<Committed, ToolError> {
        insert(tx, &run.entry).await?;
        Ok(Committed {
            id: run.entry.id.clone(),
            produced: serde_json::Map::from_iter([(
                "loyalty_transaction_id".to_owned(),
                json!(run.entry.id),
            )]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &GrantRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let stored = entry(&mut tx, &run.entry.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = stored.as_ref().is_some_and(|e| {
            e.transaction_type == "grant"
                && e.amount_value == run.entry.amount.value
                && e.customer_id == run.entry.customer_id
                && e.expires == run.entry.expires
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is missing or differs", run.entry.id)]
            },
        })
    }

    /// The ledger is only added to: undoing a grant adds a reversal of the same amount.
    async fn undo(&self, tx: &mut TxCx, run: &GrantRun, _: &Committed) -> Result<(), ToolError> {
        if offset_of(tx, &run.entry.id, "reversal").await?.is_some() {
            return Ok(());
        }
        insert(
            tx,
            &NewEntry {
                id: repo::new_id("lyt"),
                customer_id: run.entry.customer_id.clone(),
                transaction_type: LoyaltyTransactionType::Reversal,
                amount: run.entry.amount,
                order_id: None,
                source_transaction_id: Some(run.entry.id.clone()),
                expires: None,
                description: Some(format!("Undoes grant {}", run.entry.id)),
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
    fn always_needs_a_person_and_says_why_over_the_threshold() {
        assert_eq!(GrantLoyalty::META.kind, Kind::WriteApproval);
        let s = closed_schema_for::<GrantLoyaltyInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        let p = crate::testing::policies(false);
        let small = p.approval_for(
            "grant_loyalty",
            &json!({"amount": {"value": 5.0, "currency": "USD"}}),
        );
        assert!(!small.needed);
        let big = p.approval_for(
            "grant_loyalty",
            &json!({"amount": {"value": 80.0, "currency": "USD"}}),
        );
        assert_eq!(big.reason.as_deref(), Some("Amount over $50.00"));
    }
}
