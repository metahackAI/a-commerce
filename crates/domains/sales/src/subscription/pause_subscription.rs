//! `pause_subscription` (`sales.subscription.pause`): no orders until it is resumed
//! (`active → paused`). Pausing a paused subscription changes nothing.

use acom_contract::{
    ActionType, Audience, Committed, ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned,
    Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    StatePlan, apply_state, basis, invalid_state, restore_state, state_is, visible_subscription,
};
use crate::repo::SubscriptionState;
use crate::{NOT_SIGNED_IN, approval};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PauseSubscriptionInput {
    /// The subscription (`list_subscriptions`).
    #[schemars(regex(pattern = r"^sub_[a-z0-9_]+$"), example = &"sub_3301")]
    pub subscription_id: String,
}

pub(crate) struct PauseSubscription;

impl Operation for PauseSubscription {
    const META: OpMeta = OpMeta {
        address: "sales.subscription.pause",
        tool: "pause_subscription",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/sales/subscriptions/{subscription_id}/pause",
        summary: "Pause a standing order: no orders are placed until it is resumed. The shopper's own, or any for operators.",
        consumes: &[Slot::one("subscription_id")],
        produces: &[Slot::one("subscription_id")],
        checks: &[],
        errors: &[
            ErrorCode::INVALID_STATE,
            ErrorCode::NOT_FOUND,
            NOT_SIGNED_IN,
        ],
        audience: Audience::Both,
        action_type: ActionType("SuspendAction"),
        person_scope: "",
    };
    type Input = PauseSubscriptionInput;
    type Output = Value;
}

impl WriteOp<Cx> for PauseSubscription {
    type Plan = StatePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: PauseSubscriptionInput,
    ) -> Result<Planned<StatePlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = visible_subscription(cx, &mut tx, &input.subscription_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let from = SubscriptionState::from(&r);
        let to = match r.subscription_status.as_str() {
            "active" => SubscriptionState {
                status: "paused".into(),
                paused_at: Some(cx.clock.now()),
                ..from.clone()
            },
            "paused" => from.clone(),
            _ => return Err(invalid_state(&r, "paused")),
        };
        let plan = StatePlan {
            subscription_id: r.id.clone(),
            from,
            to,
        };
        Ok(Planned {
            changes: plan.changes(),
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
        let s = closed_schema_for::<PauseSubscriptionInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["subscription_id"]));
    }

    /// Integration (`DATABASE_URL`, demo seed): Jamie pauses her merino tee subscription and
    /// undoes it; Grace's paused one is not hers to touch.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_pauses_and_puts_it_back() {
        let jamie = crate::testdb::cx("cus_jamie").await;
        let planned = PauseSubscription
            .preview(
                &jamie,
                PauseSubscriptionInput {
                    subscription_id: "sub_3301".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(planned.changes[0].to, "paused");
        let mut tx = jamie.begin().await.unwrap();
        let done = PauseSubscription
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap();
        assert_eq!(done.produced["status"], "paused");
        PauseSubscription
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap();
        drop(tx); // rolled back
        let e = PauseSubscription
            .preview(
                &jamie,
                PauseSubscriptionInput {
                    subscription_id: "sub_3302".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }
}
