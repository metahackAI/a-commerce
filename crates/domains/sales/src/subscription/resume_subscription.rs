//! `resume_subscription` (`sales.subscription.resume`): orders again (`paused → active`). The
//! next charge date is the first date of its schedule after today (`subscription.cycle_length`),
//! and the shopper must still have a saved card (`payment.subscription_card_only`).

use acom_contract::{
    ActionType, Audience, Committed, ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned,
    Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_customer::service::for_sales as customer;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::prepare_subscription::card_check;
use super::{
    StatePlan, apply_state, basis, invalid_state, restore_state, state_is, visible_subscription,
};
use crate::repo::SubscriptionState;
use crate::rules;
use crate::{NOT_CARD_PAYMENT, NOT_SIGNED_IN, approval, today};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResumeSubscriptionInput {
    /// The paused subscription (`list_subscriptions`).
    #[schemars(regex(pattern = r"^sub_[a-z0-9_]+$"), example = &"sub_3302")]
    pub subscription_id: String,
}

pub(crate) struct ResumeSubscription;

impl Operation for ResumeSubscription {
    const META: OpMeta = OpMeta {
        address: "sales.subscription.resume",
        tool: "resume_subscription",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/sales/subscriptions/{subscription_id}/resume",
        summary: "Resume a paused standing order: the next order is on the first date of its schedule after today, charged to the saved card.",
        consumes: &[Slot::one("subscription_id")],
        produces: &[Slot::one("subscription_id")],
        checks: &[
            "payment.subscription_card_only",
            "subscription.cycle_length",
        ],
        errors: &[
            ErrorCode::INVALID_STATE,
            ErrorCode::NOT_FOUND,
            NOT_CARD_PAYMENT,
            NOT_SIGNED_IN,
        ],
        audience: Audience::Both,
        action_type: ActionType("ResumeAction"),
        person_scope: "",
    };
    type Input = ResumeSubscriptionInput;
    type Output = Value;
}

impl WriteOp<Cx> for ResumeSubscription {
    type Plan = StatePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ResumeSubscriptionInput,
    ) -> Result<Planned<StatePlan>, ToolError> {
        let rules = rules::subscription_rules(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = visible_subscription(cx, &mut tx, &input.subscription_id).await?;
        let shopper = customer::shopper(&mut tx, &r.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let from = SubscriptionState::from(&r);
        let mut checks = Vec::new();
        let to = match r.subscription_status.as_str() {
            "paused" => {
                checks.push(card_check(
                    shopper.as_ref().and_then(|s| s.card_last4.as_deref()),
                    rules.card_only,
                )?);
                let length = rules
                    .cycle_length
                    .get(&r.billing_cycle)
                    .copied()
                    .ok_or_else(|| {
                        ToolError::new(
                            ErrorCode::INVALID_STATE,
                            format!(
                                "The store no longer offers {} subscriptions; change the cycle first.",
                                r.billing_cycle.replace('_', " ")
                            ),
                        )
                        .detail(json!({"policy_key": "subscription.billing_cycles"}))
                        .next("update_schedule")
                    })?;
                let today = today(cx.clock.as_ref(), &cx.time);
                SubscriptionState {
                    status: "active".into(),
                    next_charge_date: length.next_after(r.next_charge_date, today),
                    ..from.clone()
                }
            }
            "active" => from.clone(),
            _ => return Err(invalid_state(&r, "resumed")),
        };
        let plan = StatePlan {
            subscription_id: r.id.clone(),
            from,
            to,
        };
        Ok(Planned {
            changes: plan.changes(),
            checks,
            basis: basis(&r),
            approval: approval(cx, Self::META.tool, &json!({})),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StatePlan) -> Result<Committed, ToolError> {
        apply_state(tx, plan).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &StatePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        state_is(cx, plan).await
    }

    async fn undo(&self, tx: &mut TxCx, plan: &StatePlan, _: &Committed) -> Result<(), ToolError> {
        restore_state(tx, plan).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<ResumeSubscriptionInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
    }

    /// Integration (`DATABASE_URL`, demo seed): Grace's socks every two weeks were paused with
    /// Oct 30 as the next charge; resumed on Sat Nov 7 the next one is Nov 13. Rolled back.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn grace_resumes_on_the_next_date_of_her_schedule() {
        let grace = crate::testdb::cx("cus_grace").await;
        let planned = ResumeSubscription
            .preview(
                &grace,
                ResumeSubscriptionInput {
                    subscription_id: "sub_3302".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(planned.changes[0].to, "active");
        assert_eq!(
            planned.plan.to.next_charge_date.to_string(),
            "2026-11-13",
            "Oct 30 + 14 days"
        );
        assert_eq!(
            planned.checks[0].detail.as_deref(),
            Some("Card ending 1350")
        );
        let mut tx = grace.begin().await.unwrap();
        ResumeSubscription
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap();
        let jobs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.job WHERE tenant_id = $1 AND kind = 'subscription.renew'
               AND payload ->> 'subscription_id' = 'sub_3302' AND done_at IS NULL",
        )
        .bind("store_northwind")
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        drop(tx); // rolled back
        assert_eq!(jobs, 1, "the next order is scheduled");
    }
}
