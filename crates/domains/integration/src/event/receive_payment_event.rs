//! `receive_payment_event` (`integration.event.receive_payment_event`): a payment provider
//! reports a payment or a refund (authorized, deposit confirmed, cancelled, refund completed,
//! refund failed). The system calls it for the provider: the signature is checked, the amount
//! must be the payment's or the refund's, the event is recorded once, and sales (payments) or
//! fulfillment (refunds, `refund.settlement: payment_event_confirmed`) apply it when their rules
//! allow. The same event never charges or refunds twice. Not a tool a model sees.

use acom_contract::{
    ActionType, Audience, Committed, Group, Kind, Method, OpMeta, Operation, Planned, Slot,
    ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::{ChannelType, PaymentEventType};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    ReceivePlan, check_signature, event_key, money_target, not_undone, order_for, planned, record,
    same_amount, verify_record,
};
use crate::channel::registered;
use crate::rules::EventRules;
use crate::{
    AMOUNT_MISMATCH, INVALID_SIGNATURE, ORDER_NOT_MAPPED, UNKNOWN_CHANNEL, check_id, new_id,
    required_text,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReceivePaymentEventInput {
    /// The payment provider's channel.
    #[schemars(regex(pattern = r"^chn_[a-z0-9_]+$"), example = &"chn_payment_cards")]
    pub channel_id: String,
    /// Lowercase hex HMAC-SHA256 of channel_id, acom:event_type, acom:payment_ref, the amount as
    /// `<value> <currency>` (as written in the body), external_order_ref and start_time joined by
    /// line feeds, keyed by the SHA-256 (hex) of the channel's signing secret.
    #[serde(rename = "acom:signature", alias = "signature")]
    #[schemars(length(min = 1, max = 200), example = &"9a1f…")]
    pub signature: String,
    /// What happened. authorized: the card payment was approved; deposit_confirmed: the money of
    /// a bank transfer or virtual account arrived; cancelled: the payment was called off;
    /// refund_completed: a refund reached the shopper; refund_failed: a refund did not go
    /// through.
    #[serde(rename = "acom:event_type", alias = "event_type")]
    #[schemars(example = &"refund_completed")]
    pub event_type: PaymentEventType,
    /// The provider's reference of the payment or the refund.
    #[serde(rename = "acom:payment_ref", alias = "payment_ref")]
    #[schemars(length(min = 1, max = 120), example = &"pi_3PqRZ2")]
    pub payment_ref: String,
    /// The amount the provider reports; it must be the payment's or the refund's.
    #[schemars(example = json!({"value": 312.00, "currency": "USD"}))]
    pub amount: Money,
    /// The order the provider knows the payment by: the store's order number, or a
    /// marketplace's order reference.
    #[schemars(length(min = 1, max = 120), example = &"20402")]
    pub external_order_ref: String,
    /// When it happened at the provider (RFC 3339).
    #[schemars(example = &"2026-11-06T10:58:00-05:00")]
    pub start_time: String,
}

/// Receive a payment provider's event.
#[derive(Debug)]
pub struct ReceivePaymentEvent;

impl Operation for ReceivePaymentEvent {
    const META: OpMeta = OpMeta {
        address: "integration.event.receive_payment_event",
        tool: "receive_payment_event",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/integration/events/payment",
        summary: "System only: a payment provider reports a payment or a refund. The signature and amount are checked, the event recorded once, and the payment (sales) or refund (fulfillment) moved the way its domain allows; never twice.",
        consumes: &[],
        produces: &[Slot::one("payment_id"), Slot::one("refund_id")],
        checks: &["events.signature_required", "events.replay_window"],
        errors: &[
            INVALID_SIGNATURE,
            UNKNOWN_CHANNEL,
            AMOUNT_MISMATCH,
            ORDER_NOT_MAPPED,
        ],
        audience: Audience::Internal,
        action_type: ActionType::RECEIVE,
        person_scope: "",
    };
    type Input = ReceivePaymentEventInput;
    type Output = Value;
}

/// The fields the signature covers, in order.
pub(crate) fn signed_fields(i: &ReceivePaymentEventInput) -> [String; 6] {
    [
        i.channel_id.clone(),
        i.event_type.as_str().to_owned(),
        i.payment_ref.clone(),
        format!("{} {}", i.amount.value, i.amount.currency),
        i.external_order_ref.clone(),
        i.start_time.clone(),
    ]
}

/// The preview's work, inside one transaction.
pub(crate) async fn plan(
    tx: &mut TxCx,
    input: ReceivePaymentEventInput,
) -> Result<Planned<ReceivePlan>, ToolError> {
    check_id("channel_id", "chn", &input.channel_id)?;
    let rules = EventRules::from_policy(&tx.policy)?;
    let channel = registered(tx, &input.channel_id, UNKNOWN_CHANNEL).await?;
    if channel.channel_type != ChannelType::Payment.as_str() {
        return Err(ToolError::invalid_input(format!(
            "{} is a {}, not a payment provider.",
            channel.id, channel.channel_type
        ))
        .detail(json!({"field": "channel_id", "reason": "enum", "allowed": ["payment"]})));
    }
    let fields = signed_fields(&input);
    let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
    check_signature(&rules, &channel, &fields, &input.signature)?;
    let payment_ref = required_text("acom:payment_ref", &input.payment_ref)?;
    let reference = required_text("external_order_ref", &input.external_order_ref)?;
    if input.amount.value.is_sign_negative() {
        return Err(ToolError::invalid_input("`amount` cannot be negative.")
            .detail(json!({"field": "amount", "reason": "range"})));
    }
    let start_time = tx.time.parse(input.start_time.trim()).map_err(|e| {
        ToolError::invalid_input(format!("`start_time`: {e}"))
            .detail(json!({"field": "start_time", "reason": "format"}))
    })?;
    let event_type = input.event_type.as_str();
    let key = event_key(&channel.id, event_type, &payment_ref, start_time);
    let duplicate_of = crate::repo::event_by_key(tx, &key).await?;
    let order_id = order_for(tx, &reference).await?;
    let refund = matches!(
        input.event_type,
        PaymentEventType::RefundCompleted | PaymentEventType::RefundFailed
    );
    let target = money_target(tx, refund, &order_id, &payment_ref, input.amount).await?;
    if duplicate_of.is_none() {
        same_amount(&target, input.amount)?;
    }
    let plan = ReceivePlan {
        event_id: new_id("whk_p"),
        channel_id: channel.id,
        event_type: event_type.to_owned(),
        carrier: false,
        event_key: key,
        external_order_ref: reference,
        order_id,
        tracking_number: None,
        payment_ref: Some(payment_ref),
        amount: Some(input.amount),
        start_time,
        received_at: tx.clock.now(),
        duplicate_of,
    };
    Ok(planned(plan, &target, &rules))
}

impl WriteOp<Cx> for ReceivePaymentEvent {
    type Plan = ReceivePlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ReceivePaymentEventInput,
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

    /// The seeded card processor's key: the stored hash of its signing secret.
    async fn key(tx: &mut TxCx) -> String {
        crate::repo::channel(tx, "chn_payment_cards")
            .await
            .unwrap()
            .and_then(|c| c.signing_secret_hash)
            .unwrap()
    }

    fn signed(v: Value, key: &str) -> ReceivePaymentEventInput {
        let mut v = v;
        let unsigned: ReceivePaymentEventInput = serde_json::from_value(v.clone()).unwrap();
        let fields = signed_fields(&unsigned);
        let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
        v["acom:signature"] = json!(sign(key, &fields));
        serde_json::from_value(v).unwrap()
    }

    fn event(kind: &str, payment_ref: &str, amount: f64, order: &str) -> Value {
        json!({"channel_id": "chn_payment_cards", "acom:signature": "", "acom:event_type": kind,
               "acom:payment_ref": payment_ref, "amount": {"value": amount, "currency": "USD"},
               "external_order_ref": order, "start_time": "2026-11-06T10:58:00-05:00"})
    }

    #[test]
    fn the_input_is_closed_and_hidden_from_models() {
        let s = closed_schema_for::<ReceivePaymentEventInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(ReceivePaymentEvent::META.audience, Audience::Internal);
        assert_eq!(
            ReceivePaymentEvent::META.produces,
            &[Slot::one("payment_id"), Slot::one("refund_id")]
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs the demo database (DATABASE_URL)"]
    async fn confirms_a_deposit_once_and_refuses_a_wrong_amount() {
        let cx = crate::testing::db("sys_payments").await;
        let mut tx = cx.begin().await.unwrap();
        let key = key(&mut tx).await;
        let due: (String, rust_decimal::Decimal) = sqlx::query_as(
            "SELECT id, amount_value FROM sales.payment
             WHERE tenant_id = 'store_northwind' AND order_id = 'ord_29001'",
        )
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        let amount: f64 = due.1.to_string().parse().unwrap();
        let e = plan(
            &mut tx,
            signed(
                event("deposit_confirmed", "va_29001", amount + 1.0, "29001"),
                &key,
            ),
        )
        .await
        .unwrap_err();
        assert_eq!((e.code.as_ref(), e.next_tool), ("amount_mismatch", None));
        let p = plan(
            &mut tx,
            signed(
                event("deposit_confirmed", "va_29001", amount, "29001"),
                &key,
            ),
        )
        .await
        .unwrap();
        assert_eq!(p.changes[1].id, due.0);
        assert_eq!(p.changes[1].to, json!("payment_complete"));
        let done = ReceivePaymentEvent.commit(&mut tx, &p.plan).await.unwrap();
        assert_eq!(done.produced["payment_id"], json!(due.0));
        let status: String = sqlx::query_scalar(
            "SELECT order_status FROM sales.sales_order WHERE tenant_id = 'store_northwind' AND id = 'ord_29001'",
        )
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        assert_eq!(status, "processing");
        let again = plan(
            &mut tx,
            signed(
                event("deposit_confirmed", "va_29001", amount, "29001"),
                &key,
            ),
        )
        .await
        .unwrap();
        let second = ReceivePaymentEvent
            .commit(&mut tx, &again.plan)
            .await
            .unwrap();
        assert_eq!(second.id, done.id, "the same event is not applied twice");
        drop(tx);
    }
}
