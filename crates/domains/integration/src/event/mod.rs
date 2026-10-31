//! The `event` resource (docs/api/integration.md `## event`): what carriers and payment
//! providers send. Not tools a model picks: the system calls them for the outside sender, and
//! each event is checked, recorded once, and applied only through the owning domain's rules.
//!
//! - **Signature.** `acom:signature` is the lowercase hex HMAC-SHA256 of the event's fields
//!   joined by line feeds (see each operation's `signed_fields`), keyed by the channel's
//!   `signing_secret_hash` — the SHA-256 (hex) of the signing secret the channel was given when it
//!   was registered. The secret itself is never stored (`events.signature_required`).
//! - **Once.** An event is known by its channel, type, tracking number or payment reference, and
//!   the time it happened; a resent event answers with the first record and changes nothing.
//! - **Too old.** An event that happened longer ago than `events.replay_window.minutes` when it
//!   arrives is recorded and not applied.
//! - **Not yet.** When its parcel, payment or refund is not recorded yet, the event waits
//!   (`potential`) and the job `integration.apply_event` tries again every
//!   `events.pending.retry_every_minutes` until `events.pending.give_up_after_minutes`.

pub(crate) mod receive_carrier_event;
pub(crate) mod receive_payment_event;

pub use receive_carrier_event::ReceiveCarrierEvent;
pub use receive_payment_event::ReceivePaymentEvent;

use acom_contract::{
    ApprovalNeed, Change, ChangeOp, Check, Committed, ErrorClass, Job, JobMeta, JobOutcome,
    Planned, ToolError, Verification,
};
use acom_core::Money;
use acom_fulfillment::service::for_integration as fulfillment;
use acom_sales::service::for_integration as sales;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::repo::{self, ChannelRow, EventEnd, EventRow, NewEvent};
use crate::rules::EventRules;
use crate::{INVALID_SIGNATURE, ORDER_NOT_MAPPED, hmac_sha256_hex, money_text, same_secret};

/// The job that applies events waiting for their parcel, payment or refund.
pub const APPLY_EVENT: &str = "integration.apply_event";

/// Checks an event's signature against the channel's signing secret (its hash is the key).
pub(crate) fn check_signature(
    rules: &EventRules,
    channel: &ChannelRow,
    fields: &[&str],
    signature: &str,
) -> Result<(), ToolError> {
    if !rules.signature_required {
        return Ok(());
    }
    let refuse = |why: &str| {
        ToolError::new(
            INVALID_SIGNATURE,
            format!("The event is not signed by {}: {why}.", channel.id),
        )
        .detail(json!({"reason": "signature", "channel_id": channel.id}))
    };
    let key = channel
        .signing_secret_hash
        .as_deref()
        .ok_or_else(|| refuse("the channel has no signing secret"))?;
    let expected = sign(key, fields);
    let given = signature.trim().to_ascii_lowercase();
    let given = given.strip_prefix("sha256=").unwrap_or(&given);
    if same_secret(&expected, given) {
        Ok(())
    } else {
        Err(refuse("the signature does not match"))
    }
}

/// The signature of an event's fields under a channel's key (the hash of its signing secret).
pub(crate) fn sign(key: &str, fields: &[&str]) -> String {
    hmac_sha256_hex(key.as_bytes(), fields.join("\n").as_bytes())
}

/// The key a resent event is recognised by.
pub(crate) fn event_key(
    channel_id: &str,
    event_type: &str,
    reference: &str,
    at: DateTime<Utc>,
) -> String {
    format!(
        "{channel_id}|{event_type}|{}|{}",
        reference.trim(),
        at.to_rfc3339()
    )
}

/// Whether an event that happened at `at` arrived within the replay window at `now`.
pub(crate) fn in_window(rules: &EventRules, at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now - at <= Duration::minutes(rules.replay_window_minutes)
}

/// The store's order the sender's reference names: a marketplace order a channel mapped, our
/// order id, or the number people see. `order_not_mapped` (no next tool: the caller is an outside
/// system) when none.
pub(crate) async fn order_for(tx: &mut TxCx, reference: &str) -> Result<String, ToolError> {
    let r = reference.trim();
    if let Some(id) = repo::mapped_order(tx, r).await? {
        return Ok(id);
    }
    sales::order_by_reference(tx, r)
        .await?
        .map(|o| o.id)
        .ok_or_else(|| {
            ToolError::new(
                ORDER_NOT_MAPPED,
                format!("No order of this store is known as {r}."),
            )
            .detail(json!({"entity": "order", "external_order_ref": r}))
        })
}

/// What an event is about in the store, once it is recorded there.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Target {
    Parcel {
        id: String,
        order_id: String,
        state: String,
    },
    Payment {
        id: String,
        status: String,
        amount: Money,
    },
    Refund {
        id: String,
        status: String,
        amount: Money,
    },
    /// Not recorded yet: the event waits.
    Missing,
}

/// Finds what a carrier event is about: the parcel with its tracking number.
pub(crate) async fn parcel(tx: &mut TxCx, tracking_number: &str) -> Result<Target, ToolError> {
    Ok(
        match fulfillment::delivery_by_tracking(tx, tracking_number.trim()).await? {
            Some(d) => Target::Parcel {
                id: d.id,
                order_id: d.order_id,
                state: d.delivery_state,
            },
            None => Target::Missing,
        },
    )
}

/// Finds what a payment event is about: for a refund event, the order's refund with that
/// reference (or the one refund waiting for that amount); otherwise the order's payment with that
/// reference (or its primary payment, before the provider told us a reference).
pub(crate) async fn money_target(
    tx: &mut TxCx,
    refund: bool,
    order_id: &str,
    payment_ref: &str,
    amount: Money,
) -> Result<Target, ToolError> {
    let r = payment_ref.trim();
    if refund {
        let refunds = fulfillment::refunds_of(tx, order_id).await?;
        let by_ref = refunds
            .iter()
            .find(|f| f.id == r || f.external_id.as_deref() == Some(r));
        let waiting: Vec<_> = refunds
            .iter()
            .filter(|f| {
                f.refund_status == "requested" && f.external_id.is_none() && f.amount == amount
            })
            .collect();
        let found = by_ref.or(if waiting.len() == 1 {
            waiting.first().copied()
        } else {
            None
        });
        return Ok(found.map_or(Target::Missing, |f| Target::Refund {
            id: f.id.clone(),
            status: f.refund_status.clone(),
            amount: f.amount,
        }));
    }
    let payments = sales::payments_of(tx, order_id).await?;
    let found = payments
        .iter()
        .find(|p| p.id == r || p.external_id.as_deref() == Some(r))
        .or_else(|| payments.first().filter(|p| p.external_id.is_none()));
    Ok(found.map_or(Target::Missing, |p| Target::Payment {
        id: p.id.clone(),
        status: p.payment_status.clone(),
        amount: p.amount,
    }))
}

/// What the event would do, as a preview row: the move, nothing (there already), a refusal, or
/// waiting.
pub(crate) fn foreseen(
    event_type: &str,
    target: &Target,
    in_time: bool,
) -> (Option<Change>, Option<String>) {
    if !in_time {
        return (
            None,
            Some("it happened before the replay window; it is recorded, not applied".into()),
        );
    }
    let (entity, id, from, step) = match target {
        Target::Missing => {
            return (None, Some("its parcel, payment or refund is not recorded yet; it waits and is tried again".into()));
        }
        Target::Parcel { id, state, .. } => (
            "parcel_delivery",
            id,
            state,
            fulfillment::carrier_move(event_type, state),
        ),
        Target::Refund { id, status, .. } => (
            "refund",
            id,
            status,
            fulfillment::refund_move(event_type, status),
        ),
        Target::Payment { id, status, .. } => {
            let step = match sales::payment_move(event_type, status) {
                sales::PaymentMove::To(to) => fulfillment::Move::To(to),
                sales::PaymentMove::Already => fulfillment::Move::Already,
                sales::PaymentMove::NotFrom(a) => fulfillment::Move::NotFrom(a),
            };
            ("payment", id, status, step)
        }
    };
    match step {
        fulfillment::Move::To(to) => (
            Some(Change {
                op: ChangeOp::Update,
                entity: entity.into(),
                id: id.clone(),
                from: json!(from),
                to: json!(to),
                note: Some(format!(
                    "{} reported by the sender",
                    event_type.replace('_', " ")
                )),
            }),
            None,
        ),
        fulfillment::Move::Already => (
            None,
            Some(format!(
                "{id} is {} already; nothing changes",
                from.replace('_', " ")
            )),
        ),
        fulfillment::Move::NotFrom(_) => (
            None,
            Some(format!(
                "{id} is {}; {} does not apply, so it is recorded, not applied",
                from.replace('_', " "),
                event_type.replace('_', " ")
            )),
        ),
    }
}

/// The amount a payment event names must be the payment's or the refund's.
pub(crate) fn same_amount(target: &Target, amount: Money) -> Result<(), ToolError> {
    let (id, expected) = match target {
        Target::Payment { id, amount, .. } | Target::Refund { id, amount, .. } => (id, *amount),
        _ => return Ok(()),
    };
    if expected.currency == amount.currency && expected.value == amount.value {
        return Ok(());
    }
    Err(ToolError::new(
        crate::AMOUNT_MISMATCH,
        format!(
            "The event says {} but {id} is {}.",
            money_text(&amount),
            money_text(&expected)
        ),
    )
    .detail(json!({"entity": id, "expected": expected, "actual": amount})))
}

/// What happened to a recorded event.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Settled {
    /// Applied, or already true: `(entity, id, from → to)`.
    Applied {
        slot: &'static str,
        id: String,
        moved: Option<(String, String)>,
    },
    /// Recorded, not applied, and why.
    Refused { code: &'static str, message: String },
    /// Waiting for its parcel, payment or refund.
    Waiting,
}

/// Applies a recorded event through the owning domain and ends its record; a waiting event stays
/// `potential`. `instrument` is the operation or job that applies it.
pub(crate) async fn settle(
    tx: &mut TxCx,
    ev: &EventRow,
    instrument: &str,
) -> Result<Settled, ToolError> {
    let now = tx.clock.now();
    let rules = EventRules::from_policy(&tx.policy)?;
    if !in_window(&rules, ev.start_time, ev.received_at) {
        let s = Settled::Refused {
            code: "outside_replay_window",
            message: format!(
                "It happened more than {} minutes before it arrived (events.replay_window).",
                rules.replay_window_minutes
            ),
        };
        end(tx, ev, &s, now).await?;
        return Ok(s);
    }
    let (event_type, target) = match (&ev.carrier_event_type, &ev.payment_event_type) {
        (Some(t), _) => (
            t.clone(),
            parcel(tx, ev.tracking_number.as_deref().unwrap_or("")).await?,
        ),
        (None, Some(t)) => {
            let amount = ev
                .amount()
                .ok_or_else(|| ToolError::invalid_input("The payment event has no amount."))?;
            let refund = t.starts_with("refund_");
            let target = money_target(
                tx,
                refund,
                &ev.order_id,
                ev.payment_ref.as_deref().unwrap_or(""),
                amount,
            )
            .await?;
            // A payment or refund recorded after the event came is held to its amount too.
            if let Err(e) = same_amount(&target, amount) {
                let s = Settled::Refused {
                    code: "amount_mismatch",
                    message: e.message,
                };
                end(tx, ev, &s, now).await?;
                return Ok(s);
            }
            (t.clone(), target)
        }
        (None, None) => return Err(ToolError::invalid_input("The event has no type.")),
    };
    let applied = match &target {
        Target::Missing => return Ok(Settled::Waiting),
        Target::Parcel { id, order_id, .. } if *order_id != ev.order_id => {
            Err(ToolError::invalid_input(format!(
                "Tracking number {} is on {id} of {order_id}, not of {}.",
                ev.tracking_number.as_deref().unwrap_or(""),
                ev.order_id
            )))
        }
        Target::Parcel { id, .. } => {
            fulfillment::apply_carrier_event(tx, id, &event_type, ev.start_time, instrument)
                .await
                .map(|m| ("delivery_id", id.clone(), m))
        }
        Target::Payment { id, .. } => sales::apply_payment_event(
            tx,
            id,
            &event_type,
            ev.payment_ref.as_deref().unwrap_or(""),
            ev.start_time,
            instrument,
        )
        .await
        .map(|m| ("payment_id", id.clone(), m)),
        Target::Refund { id, .. } => fulfillment::apply_refund_event(
            tx,
            id,
            &event_type,
            ev.payment_ref.as_deref().unwrap_or(""),
            instrument,
        )
        .await
        .map(|m| ("refund_id", id.clone(), m)),
    };
    let s = match applied {
        Ok((slot, id, moved)) => Settled::Applied { slot, id, moved },
        Err(e) if matches!(e.class, ErrorClass::InvalidState | ErrorClass::InvalidInput) => {
            Settled::Refused {
                code: if e.class == ErrorClass::InvalidState {
                    "invalid_transition"
                } else {
                    "invalid_input"
                },
                message: e.message,
            }
        }
        Err(e) => return Err(e),
    };
    end(tx, ev, &s, now).await?;
    Ok(s)
}

async fn end(
    tx: &mut TxCx,
    ev: &EventRow,
    s: &Settled,
    now: DateTime<Utc>,
) -> Result<(), ToolError> {
    let e = match s {
        Settled::Waiting => return Ok(()),
        Settled::Refused { code, message } => EventEnd {
            action_status: "failed",
            error: Some(json!({"code": code, "message": message})),
            at: Some(now),
            ..EventEnd::default()
        },
        Settled::Applied { slot, id, moved } => {
            let entity = match *slot {
                "delivery_id" => "parcel_delivery",
                "payment_id" => "payment",
                _ => "refund",
            };
            let mut end = EventEnd {
                action_status: "completed",
                result: Some(match moved {
                    Some((from, to)) => json!({"entity": entity, "id": id, "from": from, "to": to}),
                    None => {
                        json!({"entity": entity, "id": id, "note": "already so; nothing changed"})
                    }
                }),
                at: Some(now),
                ..EventEnd::default()
            };
            match *slot {
                "delivery_id" => end.delivery_id = Some(id.clone()),
                "payment_id" => end.payment_id = Some(id.clone()),
                _ => end.refund_id = Some(id.clone()),
            }
            end
        }
    };
    repo::end_event(tx, &ev.id, &e).await
}

/// Books the next try of a waiting event.
pub(crate) async fn book_retry(
    tx: &mut TxCx,
    event_id: &str,
    attempt: u64,
) -> Result<(), ToolError> {
    let rules = EventRules::from_policy(&tx.policy)?;
    let at = tx.clock.now() + Duration::minutes(rules.retry_every_minutes);
    let job_id = format!("job_{event_id}_{attempt}");
    acom_changeset::jobs::book(
        tx,
        &job_id,
        APPLY_EVENT,
        at,
        &json!({"webhook_event_id": event_id, "attempt": attempt}),
    )
    .await?;
    Ok(())
}

/// What a receive answers besides its id.
pub(crate) fn produced(s: &Settled) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    match s {
        Settled::Applied { slot, id, .. } => {
            out.insert((*slot).to_owned(), json!(id));
            out.insert("action_status".into(), json!("completed"));
        }
        Settled::Refused { code, message } => {
            out.insert("action_status".into(), json!("failed"));
            out.insert("error".into(), json!({"code": code, "message": message}));
        }
        Settled::Waiting => {
            out.insert("action_status".into(), json!("potential"));
        }
    }
    out
}

/// What receiving an event runs, fixed at preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReceivePlan {
    /// The record the run makes (`whk_…`).
    pub event_id: String,
    pub channel_id: String,
    /// `acom:carrierEventType` or `acom:paymentEventType` word.
    pub event_type: String,
    pub carrier: bool,
    pub event_key: String,
    pub external_order_ref: String,
    pub order_id: String,
    pub tracking_number: Option<String>,
    pub payment_ref: Option<String>,
    pub amount: Option<Money>,
    pub start_time: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    /// The first record of the same event, when it came before: the run answers with it.
    pub duplicate_of: Option<String>,
}

/// The preview of a received event: what it records and what it would change.
pub(crate) fn planned(
    plan: ReceivePlan,
    target: &Target,
    rules: &EventRules,
) -> Planned<ReceivePlan> {
    let in_time = in_window(rules, plan.start_time, plan.received_at);
    let signed = Check {
        rule: "events.signature_required".into(),
        passed: true,
        detail: Some(if rules.signature_required {
            format!("Signed with {}'s signing secret", plan.channel_id)
        } else {
            "The store does not ask for signed events".into()
        }),
        fix_tool: None,
    };
    let minutes = (plan.received_at - plan.start_time).num_minutes();
    let window = Check {
        rule: "events.replay_window".into(),
        passed: in_time,
        detail: Some(if in_time {
            format!(
                "Happened {} minutes before it arrived (window {} minutes)",
                minutes.max(0),
                rules.replay_window_minutes
            )
        } else {
            format!(
                "Happened {minutes} minutes before it arrived, outside the {}-minute window: recorded, not applied",
                rules.replay_window_minutes
            )
        }),
        fix_tool: None,
    };
    let mut changes = Vec::new();
    if let Some(first) = &plan.duplicate_of {
        changes.push(Change {
            op: ChangeOp::Exclude,
            entity: "webhook_event".into(),
            id: first.clone(),
            from: Value::Null,
            to: Value::Null,
            note: Some(format!(
                "Already received as {first}; nothing is recorded or applied again"
            )),
        });
    } else {
        let (moved, note) = foreseen(&plan.event_type, target, in_time);
        changes.push(Change {
            op: ChangeOp::Create,
            entity: "webhook_event".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"channel_id": plan.channel_id, "acom:event_type": plan.event_type,
                       "order_id": plan.order_id, "tracking_number": plan.tracking_number,
                       "acom:payment_ref": plan.payment_ref, "amount": plan.amount}),
            note,
        });
        changes.extend(moved);
    }
    Planned {
        plan,
        changes,
        checks: vec![signed, window],
        basis: Vec::new(),
        approval: ApprovalNeed::NotNeeded,
    }
}

/// Records a received event once and applies it (see [`settle`]); a resent event answers with
/// its first record.
pub(crate) async fn record(
    tx: &mut TxCx,
    plan: &ReceivePlan,
    instrument: &str,
) -> Result<Committed, ToolError> {
    let first = match &plan.duplicate_of {
        Some(id) => Some(id.clone()),
        None => repo::event_by_key(tx, &plan.event_key).await?,
    };
    if let Some(id) = first {
        let mut produced = serde_json::Map::new();
        produced.insert("acom:duplicate".into(), json!(true));
        return Ok(Committed { id, produced });
    }
    repo::insert_event(
        tx,
        &NewEvent {
            id: &plan.event_id,
            channel_id: &plan.channel_id,
            carrier_event_type: plan.carrier.then_some(plan.event_type.as_str()),
            payment_event_type: (!plan.carrier).then_some(plan.event_type.as_str()),
            event_key: &plan.event_key,
            external_order_ref: &plan.external_order_ref,
            order_id: &plan.order_id,
            tracking_number: plan.tracking_number.as_deref(),
            payment_ref: plan.payment_ref.as_deref(),
            amount: plan.amount,
            start_time: plan.start_time,
            received_at: plan.received_at,
        },
    )
    .await?;
    let ev = repo::event(tx, &plan.event_id).await?.ok_or_else(|| {
        ToolError::new(
            acom_contract::ErrorCode::INTERNAL,
            "The event was not recorded.",
        )
    })?;
    let s = settle(tx, &ev, instrument).await?;
    if s == Settled::Waiting {
        book_retry(tx, &plan.event_id, 1).await?;
    }
    repo::touch_channel(tx, &plan.channel_id, plan.received_at).await?;
    Ok(Committed {
        id: plan.event_id.clone(),
        produced: produced(&s),
    })
}

/// Reads the record back.
pub(crate) async fn verify_record(
    cx: &Cx,
    plan: &ReceivePlan,
    done: &Committed,
) -> Result<Verification, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let ev = repo::event(&mut tx, &done.id).await?;
    tx.commit().await.map_err(ToolError::from)?;
    let mismatches = match ev {
        None => vec![format!("{} was not recorded", done.id)],
        Some(e) if e.order_id != plan.order_id => vec![format!("{} is about {}", e.id, e.order_id)],
        Some(_) => Vec::new(),
    };
    Ok(Verification {
        matched: mismatches.is_empty(),
        mismatches,
    })
}

/// A received event is a fact from outside: it is not taken back.
pub(crate) fn not_undone(event_id: &str) -> ToolError {
    ToolError::new(
        acom_contract::ErrorCode::UNDO_WINDOW_CLOSED,
        format!("{event_id} records what an outside system reported; it is not taken back."),
    )
}

/// Applies a waiting event once its parcel, payment or refund is recorded; tries again later, and
/// gives up after `events.pending.give_up_after_minutes`.
#[derive(Debug)]
pub struct ApplyPendingEvent;

impl Job<Cx> for ApplyPendingEvent {
    const META: JobMeta = JobMeta {
        kind: APPLY_EVENT,
        summary: "Applies a received carrier or payment event whose parcel, payment or refund was not recorded when it came; tries again later and gives up after events.pending.give_up_after_minutes.",
        watcher: None,
    };

    async fn run(&self, tx: &mut TxCx, payload: &Value) -> Result<JobOutcome, ToolError> {
        let id = payload
            .get("webhook_event_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_input("The job names no webhook_event_id."))?
            .to_owned();
        let attempt = payload.get("attempt").and_then(Value::as_u64).unwrap_or(1);
        let Some(ev) = repo::event(tx, &id).await? else {
            return Ok(JobOutcome::done(
                json!({"webhook_event_id": id, "action_status": "gone"}),
            ));
        };
        if ev.action_status != "potential" {
            return Ok(JobOutcome::done(
                json!({"webhook_event_id": id, "action_status": ev.action_status}),
            ));
        }
        let s = settle(tx, &ev, APPLY_EVENT).await?;
        if s == Settled::Waiting {
            let rules = EventRules::from_policy(&tx.policy)?;
            let now = tx.clock.now();
            if now - ev.received_at >= Duration::minutes(rules.give_up_after_minutes) {
                let gave_up = Settled::Refused {
                    code: "target_not_recorded",
                    message: format!(
                        "No parcel, payment or refund matched within {} minutes (events.pending.give_up_after_minutes).",
                        rules.give_up_after_minutes
                    ),
                };
                end(tx, &ev, &gave_up, now).await?;
                return Ok(JobOutcome::done(
                    json!({"webhook_event_id": id, "action_status": "failed"}),
                ));
            }
            book_retry(tx, &id, attempt + 1).await?;
        }
        let mut result = produced(&s);
        result.insert("webhook_event_id".into(), json!(id));
        Ok(JobOutcome::done(Value::Object(result)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn rules() -> EventRules {
        EventRules::from_policy(&crate::testing::policies()).unwrap()
    }

    fn channel(hash: Option<&str>) -> ChannelRow {
        ChannelRow {
            id: "chn_carrier_ups".into(),
            name: "UPS".into(),
            channel_type: "carrier".into(),
            url: "https://api.ups.example/v1".into(),
            connection_status: "connected".into(),
            last_synced_at: None,
            seller_id: None,
            signing_secret_hash: hash.map(str::to_owned),
            status_mapping: json!({}),
        }
    }

    #[test]
    fn signatures_are_checked_with_the_hash_of_the_secret() {
        let key = crate::sha256_hex("whsec_demo");
        let fields = [
            "chn_carrier_ups",
            "delivered",
            "1Z999",
            "20402",
            "2026-11-06T10:58:00-05:00",
        ];
        let good = sign(&key, &fields);
        let c = channel(Some(&key));
        assert!(check_signature(&rules(), &c, &fields, &good).is_ok());
        assert!(
            check_signature(
                &rules(),
                &c,
                &fields,
                &format!("sha256={}", good.to_uppercase())
            )
            .is_ok()
        );
        let e = check_signature(&rules(), &c, &fields, &sign("other", &fields)).unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.class),
            ("invalid_signature", ErrorClass::Unauthenticated)
        );
        assert_eq!(e.next_tool, None);
        assert!(check_signature(&rules(), &channel(None), &fields, &good).is_err());
    }

    #[test]
    fn old_events_fall_outside_the_replay_window() {
        let now = Utc.with_ymd_and_hms(2026, 11, 6, 16, 0, 0).unwrap();
        assert!(in_window(&rules(), now - Duration::minutes(15), now));
        assert!(!in_window(&rules(), now - Duration::minutes(16), now));
        assert_eq!(
            event_key("chn_carrier_ups", "delivered", " 1Z999 ", now),
            "chn_carrier_ups|delivered|1Z999|2026-11-06T16:00:00+00:00"
        );
    }

    #[test]
    fn the_preview_foresees_moves_waits_and_refusals() {
        let parcel = Target::Parcel {
            id: "dlv_1".into(),
            order_id: "ord_1".into(),
            state: "in_transit".into(),
        };
        let (change, note) = foreseen("delivered", &parcel, true);
        assert_eq!(change.unwrap().to, json!("delivered"));
        assert!(note.is_none());
        assert!(
            foreseen("delivered", &Target::Missing, true)
                .1
                .unwrap()
                .contains("waits")
        );
        assert!(
            foreseen("delivered", &parcel, false)
                .1
                .unwrap()
                .contains("replay window")
        );
        let done = Target::Parcel {
            id: "dlv_1".into(),
            order_id: "ord_1".into(),
            state: "delivered".into(),
        };
        assert!(
            foreseen("picked_up", &done, true)
                .1
                .unwrap()
                .contains("does not apply")
        );
    }

    #[test]
    fn a_payment_event_must_name_the_payments_amount() {
        let usd = |v: i64| Money::new(rust_decimal::Decimal::new(v, 2), acom_core::Currency::USD);
        let t = Target::Payment {
            id: "pay_1".into(),
            status: "payment_due".into(),
            amount: usd(31200),
        };
        assert!(same_amount(&t, usd(31200)).is_ok());
        assert_eq!(
            same_amount(&t, usd(31000)).unwrap_err().code,
            "amount_mismatch"
        );
        assert!(same_amount(&Target::Missing, usd(1)).is_ok());
    }
}
