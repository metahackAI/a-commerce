//! `receive_carrier_event` (`integration.event.receive_carrier_event`): a carrier reports a
//! parcel's progress (label, pickup, on the way, delivered, delivery failed). The system calls it
//! for the carrier: the signature is checked, the event is recorded once, and fulfillment moves
//! the parcel when its rules allow. Not a tool a model sees (audience internal).

use acom_contract::{
    ActionType, Audience, Committed, Group, Kind, Method, OpMeta, Operation, Planned, Slot,
    ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::{CarrierEventType, ChannelType};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    ReceivePlan, Target, check_signature, event_key, not_undone, order_for, parcel, planned,
    record, verify_record,
};
use crate::channel::registered;
use crate::rules::EventRules;
use crate::{
    INVALID_SIGNATURE, ORDER_NOT_MAPPED, UNKNOWN_CHANNEL, check_id, new_id, required_text,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReceiveCarrierEventInput {
    /// The carrier's channel.
    #[schemars(regex(pattern = r"^chn_[a-z0-9_]+$"), example = &"chn_carrier_ups")]
    pub channel_id: String,
    /// Lowercase hex HMAC-SHA256 of channel_id, acom:event_type, tracking_number,
    /// external_order_ref and start_time joined by line feeds, keyed by the SHA-256 (hex) of the
    /// channel's signing secret.
    #[serde(rename = "acom:signature", alias = "signature")]
    #[schemars(length(min = 1, max = 200), example = &"5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843")]
    pub signature: String,
    /// What happened. label_created: the label was made; picked_up: the carrier took the parcel;
    /// in_transit: on its way; delivered: handed over; delivery_failed: could not be delivered.
    #[serde(rename = "acom:event_type", alias = "event_type")]
    #[schemars(example = &"delivered")]
    pub event_type: CarrierEventType,
    /// The carrier's tracking number.
    #[schemars(length(min = 1, max = 64), example = &"1ZNW204020001")]
    pub tracking_number: String,
    /// The order the carrier knows the parcel by: the store's order number, or a marketplace's
    /// order reference.
    #[schemars(length(min = 1, max = 120), example = &"20402")]
    pub external_order_ref: String,
    /// When it happened at the carrier (RFC 3339).
    #[schemars(example = &"2026-11-06T10:58:00-05:00")]
    pub start_time: String,
}

/// Receive a carrier's event.
#[derive(Debug)]
pub struct ReceiveCarrierEvent;

impl Operation for ReceiveCarrierEvent {
    const META: OpMeta = OpMeta {
        address: "integration.event.receive_carrier_event",
        tool: "receive_carrier_event",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/integration/events/carrier",
        summary: "System only: a carrier reports a parcel's progress. The signature is checked, the event recorded once, and the parcel moved the way fulfillment allows; an event whose parcel is not labelled yet waits.",
        consumes: &[],
        produces: &[Slot::one("delivery_id")],
        checks: &["events.signature_required", "events.replay_window"],
        errors: &[INVALID_SIGNATURE, UNKNOWN_CHANNEL, ORDER_NOT_MAPPED],
        audience: Audience::Internal,
        action_type: ActionType::RECEIVE,
        person_scope: "",
    };
    type Input = ReceiveCarrierEventInput;
    type Output = Value;
}

/// The fields the signature covers, in order.
pub(crate) fn signed_fields(i: &ReceiveCarrierEventInput) -> [&str; 5] {
    [
        i.channel_id.as_str(),
        i.event_type.as_str(),
        i.tracking_number.as_str(),
        i.external_order_ref.as_str(),
        i.start_time.as_str(),
    ]
}

/// The preview's work, inside one transaction.
pub(crate) async fn plan(
    tx: &mut TxCx,
    input: ReceiveCarrierEventInput,
) -> Result<Planned<ReceivePlan>, ToolError> {
    check_id("channel_id", "chn", &input.channel_id)?;
    let rules = EventRules::from_policy(&tx.policy)?;
    let channel = registered(tx, &input.channel_id, UNKNOWN_CHANNEL).await?;
    if channel.channel_type != ChannelType::Carrier.as_str() {
        return Err(ToolError::invalid_input(format!(
            "{} is a {}, not a carrier.",
            channel.id, channel.channel_type
        ))
        .detail(json!({"field": "channel_id", "reason": "enum", "allowed": ["carrier"]})));
    }
    check_signature(&rules, &channel, &signed_fields(&input), &input.signature)?;
    let tracking = required_text("tracking_number", &input.tracking_number)?;
    let reference = required_text("external_order_ref", &input.external_order_ref)?;
    let start_time = tx.time.parse(input.start_time.trim()).map_err(|e| {
        ToolError::invalid_input(format!("`start_time`: {e}"))
            .detail(json!({"field": "start_time", "reason": "format"}))
    })?;
    let event_type = input.event_type.as_str();
    let key = event_key(&channel.id, event_type, &tracking, start_time);
    let duplicate_of = crate::repo::event_by_key(tx, &key).await?;
    let order_id = order_for(tx, &reference).await?;
    let target = parcel(tx, &tracking).await?;
    if let Target::Parcel {
        id,
        order_id: other,
        ..
    } = &target
        && *other != order_id
        && duplicate_of.is_none()
    {
        return Err(ToolError::invalid_input(format!(
            "Tracking number {tracking} is on {id} of {other}, not on an order known as {reference}."
        ))
        .detail(json!({"field": "tracking_number", "reason": "format"})));
    }
    let plan = ReceivePlan {
        event_id: new_id("whk_c"),
        channel_id: channel.id,
        event_type: event_type.to_owned(),
        carrier: true,
        event_key: key,
        external_order_ref: reference,
        order_id,
        tracking_number: Some(tracking),
        payment_ref: None,
        amount: None,
        start_time,
        received_at: tx.clock.now(),
        duplicate_of,
    };
    Ok(planned(plan, &target, &rules))
}

impl WriteOp<Cx> for ReceiveCarrierEvent {
    type Plan = ReceivePlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ReceiveCarrierEventInput,
    ) -> Result<Planned<ReceivePlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let planned = plan(&mut tx, input).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(planned)
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ReceivePlan) -> Result<Committed, ToolError> {
        record(tx, plan, Self::META.tool).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ReceivePlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        verify_record(cx, plan, done).await
    }

    async fn undo(&self, _: &mut TxCx, _: &ReceivePlan, done: &Committed) -> Result<(), ToolError> {
        Err(not_undone(&done.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::sign;
    use acom_contract::schema::{closed_schema_for, violations};

    fn input(v: Value) -> ReceiveCarrierEventInput {
        serde_json::from_value(v).unwrap()
    }

    /// The seeded UPS channel's key: the stored hash of its signing secret.
    async fn ups_key(tx: &mut TxCx) -> String {
        crate::repo::channel(tx, "chn_carrier_ups")
            .await
            .unwrap()
            .and_then(|c| c.signing_secret_hash)
            .unwrap()
    }

    pub(crate) fn signed(mut v: Value, key: &str) -> ReceiveCarrierEventInput {
        let unsigned = input(v.clone());
        v["acom:signature"] = json!(sign(key, &signed_fields(&unsigned)));
        input(v)
    }

    #[test]
    fn the_input_is_closed_and_hidden_from_models() {
        let s = closed_schema_for::<ReceiveCarrierEventInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["properties"]["acom:event_type"]["enum"],
            json!([
                "label_created",
                "picked_up",
                "in_transit",
                "delivered",
                "delivery_failed"
            ])
        );
        assert_eq!(ReceiveCarrierEvent::META.audience, Audience::Internal);
        assert_eq!(ReceiveCarrierEvent::UNDO, UndoSupport::NotSupported);
    }

    fn event(kind: &str, tracking: &str, order: &str, at: &str) -> Value {
        json!({"channel_id": "chn_carrier_ups", "acom:signature": "", "acom:event_type": kind,
               "tracking_number": tracking, "external_order_ref": order, "start_time": at})
    }

    #[tokio::test]
    #[ignore = "integration: needs the demo database (DATABASE_URL)"]
    async fn checks_records_once_applies_and_waits() {
        let cx = crate::testing::db("sys_carrier").await;
        let mut tx = cx.begin().await.unwrap();
        let key = ups_key(&mut tx).await;
        // A bad signature is refused before anything is read.
        let mut bad = input(event(
            "delivered",
            "1ZNW204020001",
            "20402",
            "2026-11-06T10:58:00-05:00",
        ));
        bad.signature = "00".into();
        let e = plan(&mut tx, bad).await.unwrap_err();
        assert_eq!((e.code.as_ref(), e.next_tool), ("invalid_signature", None));
        // An order nobody knows.
        let e = plan(
            &mut tx,
            signed(
                event("delivered", "1Z0", "99999", "2026-11-06T10:58:00-05:00"),
                &key,
            ),
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, "order_not_mapped");
        // A parcel on its way is delivered: the parcel, its line and the order move.
        sqlx::query(
            "UPDATE fulfillment.parcel_delivery SET delivery_state = 'in_transit', delivered_at = NULL
             WHERE tenant_id = 'store_northwind' AND id = 'dlv_20402'",
        )
        .execute(&mut *tx.tx)
        .await
        .unwrap();
        let tracking: String = sqlx::query_scalar(
            "SELECT tracking_number FROM fulfillment.parcel_delivery
             WHERE tenant_id = 'store_northwind' AND id = 'dlv_20402'",
        )
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        let p = plan(
            &mut tx,
            signed(
                event(
                    "delivered",
                    &tracking,
                    "#20402",
                    "2026-11-06T10:58:00-05:00",
                ),
                &key,
            ),
        )
        .await
        .unwrap();
        assert!(p.checks.iter().all(|c| c.passed), "{:?}", p.checks);
        assert_eq!(p.changes[1].id, "dlv_20402");
        let done = ReceiveCarrierEvent.commit(&mut tx, &p.plan).await.unwrap();
        assert_eq!(done.produced["delivery_id"], "dlv_20402");
        assert_eq!(done.produced["action_status"], "completed");
        let state: String = sqlx::query_scalar(
            "SELECT delivery_state FROM fulfillment.parcel_delivery
             WHERE tenant_id = 'store_northwind' AND id = 'dlv_20402'",
        )
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        assert_eq!(state, "delivered");
        // The same event again: the first record, nothing new.
        let again = plan(
            &mut tx,
            signed(
                event("delivered", &tracking, "20402", "2026-11-06T10:58:00-05:00"),
                &key,
            ),
        )
        .await
        .unwrap();
        assert_eq!(again.plan.duplicate_of.as_deref(), Some(done.id.as_str()));
        let second = ReceiveCarrierEvent
            .commit(&mut tx, &again.plan)
            .await
            .unwrap();
        assert_eq!(second.id, done.id);
        // A label nobody made yet: recorded, waiting, and a retry is booked.
        let wait = plan(
            &mut tx,
            signed(
                event(
                    "picked_up",
                    "1ZNWNOTYET",
                    "20402",
                    "2026-11-06T10:59:00-05:00",
                ),
                &key,
            ),
        )
        .await
        .unwrap();
        let waiting = ReceiveCarrierEvent
            .commit(&mut tx, &wait.plan)
            .await
            .unwrap();
        assert_eq!(waiting.produced["action_status"], "potential");
        let booked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.job WHERE tenant_id = 'store_northwind' AND kind = 'integration.apply_event'
               AND payload->>'webhook_event_id' = $1",
        )
        .bind(&waiting.id)
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        assert_eq!(booked, 1);
        // An old event is kept, not applied.
        let old = plan(
            &mut tx,
            signed(
                event(
                    "in_transit",
                    &tracking,
                    "20402",
                    "2026-11-06T09:00:00-05:00",
                ),
                &key,
            ),
        )
        .await
        .unwrap();
        assert!(!old.checks[1].passed);
        let kept = ReceiveCarrierEvent
            .commit(&mut tx, &old.plan)
            .await
            .unwrap();
        assert_eq!(kept.produced["error"]["code"], "outside_replay_window");
        drop(tx);
    }
}
