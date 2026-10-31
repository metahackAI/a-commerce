//! `cancel_subscription` (`sales.subscription.cancel`): ends a standing order for good
//! (`active | paused → cancelled`). Orders it already placed stay as they are. A cancelled
//! subscription is not taken back; the shopper prepares a new one.

use acom_contract::{
    ActionType, Audience, Committed, ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned,
    Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{StatePlan, apply_state, basis, invalid_state, state_is, visible_subscription};
use crate::repo::SubscriptionState;
use crate::{NOT_SIGNED_IN, approval};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelSubscriptionInput {
    /// The subscription (`list_subscriptions`).
    #[schemars(regex(pattern = r"^sub_[a-z0-9_]+$"), example = &"sub_3301")]
    pub subscription_id: String,
    /// Why, in the shopper's words. Kept with the subscription.
    #[schemars(length(max = 500), example = &"Have enough tees for now")]
    pub reason: Option<String>,
}

pub(crate) struct CancelSubscription;

impl Operation for CancelSubscription {
    const META: OpMeta = OpMeta {
        address: "sales.subscription.cancel",
        tool: "cancel_subscription",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/sales/subscriptions/{subscription_id}/cancel",
        summary: "Cancel a standing order for good: no more orders; the ones already placed stay. Cannot be undone; a new one is prepared with prepare_subscription.",
        consumes: &[Slot::one("subscription_id")],
        produces: &[Slot::one("subscription_id")],
        checks: &[],
        errors: &[
            ErrorCode::INVALID_STATE,
            ErrorCode::NOT_FOUND,
            NOT_SIGNED_IN,
        ],
        audience: Audience::Both,
        action_type: ActionType::CANCEL,
        person_scope: "",
    };
    type Input = CancelSubscriptionInput;
    type Output = Value;
}

impl WriteOp<Cx> for CancelSubscription {
    type Plan = StatePlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CancelSubscriptionInput,
    ) -> Result<Planned<StatePlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = visible_subscription(cx, &mut tx, &input.subscription_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if !matches!(r.subscription_status.as_str(), "active" | "paused") {
            return Err(invalid_state(&r, "cancelled"));
        }
        let from = SubscriptionState::from(&r);
        let to = SubscriptionState {
            status: "cancelled".into(),
            cancelled_at: Some(cx.clock.now()),
            cancel_reason_text: input
                .reason
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_owned),
            ..from.clone()
        };
        let plan = StatePlan {
            subscription_id: r.id.clone(),
            from,
            to,
        };
        let mut changes = plan.changes();
        changes[0].note = Some("Final: orders already placed stay; it cannot be undone".into());
        Ok(Planned {
            changes,
            checks: vec![],
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

    async fn undo(&self, _: &mut TxCx, plan: &StatePlan, _: &Committed) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "Subscription {} is cancelled for good; prepare a new one.",
                plan.subscription_id
            ),
        )
        .next("prepare_subscription"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<CancelSubscriptionInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["subscription_id"]));
    }

    /// Integration (`DATABASE_URL`, demo seed): an operator previews cancelling Jamie's
    /// subscription; an unknown one is pointed at `list_subscriptions`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn cancelling_is_final_and_unknown_ones_are_not_found() {
        let sj = crate::testdb::cx("adm_sj").await;
        let planned = CancelSubscription
            .preview(
                &sj,
                CancelSubscriptionInput {
                    subscription_id: "sub_3301".into(),
                    reason: Some("Have enough tees".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(planned.changes[0].to, "cancelled");
        assert_eq!(
            planned.plan.to.cancel_reason_text.as_deref(),
            Some("Have enough tees")
        );
        let e = CancelSubscription
            .preview(
                &sj,
                CancelSubscriptionInput {
                    subscription_id: "sub_nope".into(),
                    reason: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("not_found", Some("list_subscriptions"))
        );
    }
}
