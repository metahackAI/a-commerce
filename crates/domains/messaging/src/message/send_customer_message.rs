//! `send_customer_message`: one transactional message to one shopper about their order. A
//! person always approves it, and once sent it cannot be taken back.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorClass,
    ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_core::enums::MessageChannel;
use acom_core::ids::MessageId;
use acom_customer::service::for_messaging as customer;
use acom_fulfillment::service::for_messaging as fulfillment;
use acom_sales::service::for_messaging as sales;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::model::{QuietHours, default_subject, has_subject, masked_name, reachable};
use crate::repo::{self, NewMessage};

pub const CUSTOMER_NOT_FOUND: ErrorCode =
    ErrorCode::new("customer_not_found", ErrorClass::NotFound);
pub const ORDER_NOT_FOUND: ErrorCode = ErrorCode::new("order_not_found", ErrorClass::NotFound);
pub const ORDER_NOT_FOR_CUSTOMER: ErrorCode =
    ErrorCode::new("order_not_for_customer", ErrorClass::Forbidden);
pub const ORDER_REQUIRED: ErrorCode = ErrorCode::new("order_required", ErrorClass::InvalidInput);
pub const CUSTOMER_REQUIRED: ErrorCode =
    ErrorCode::new("customer_required", ErrorClass::InvalidInput);
pub const REFUND_NOT_FOUND: ErrorCode = ErrorCode::new("refund_not_found", ErrorClass::NotFound);
pub const NO_VERIFIED_CONTACT: ErrorCode =
    ErrorCode::new("no_verified_contact", ErrorClass::Domain);
pub const EMPTY_TEXT: ErrorCode = ErrorCode::new("empty_text", ErrorClass::InvalidInput);
pub const NOT_REVOCABLE: ErrorCode = ErrorCode::new("not_revocable", ErrorClass::UndoWindowClosed);
pub const QUIET_HOURS: ErrorCode = ErrorCode::new("quiet_hours", ErrorClass::PolicyViolation);

/// Longest body a customer message may have (`messaging.message.text`).
const MAX_TEXT: usize = 2000;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendCustomerMessageInput {
    /// The shopper the message goes to. Leave it out when order_id is given: the message goes to
    /// the shopper who placed the order. Needed when there is no order_id.
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_riley")]
    pub customer_id: Option<String>,
    /// The order the message is about. Required unless kind is custom.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20366")]
    pub order_id: Option<String>,
    /// ask_for_photos: ask for photos of damage; decline: a return or exchange is declined;
    /// refund_done: the refund went out; custom: any other note about an order.
    #[schemars(example = &"ask_for_photos")]
    pub kind: CustomerMessageKind,
    /// email, sms, push or in_app. Default email; the shopper must be reachable on it.
    #[schemars(example = &"email")]
    pub channel: Option<MessageChannel>,
    /// Subject line (email and push). Made from the kind and order number when left out.
    #[schemars(length(max = 200), example = &"About your return for order 20366")]
    pub subject: Option<String>,
    /// The message exactly as the shopper reads it, in plain sentences.
    #[schemars(length(max = 2000), example = &"Sorry your rain parkas arrived damaged. Could you reply with a photo or two of the damage? We'll take it from there.")]
    pub text: String,
}

/// What a customer message does: `acom:message_kind` without `campaign`, which only campaigns
/// send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CustomerMessageKind {
    AskForPhotos,
    Decline,
    RefundDone,
    Custom,
}

impl CustomerMessageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AskForPhotos => "ask_for_photos",
            Self::Decline => "decline",
            Self::RefundDone => "refund_done",
            Self::Custom => "custom",
        }
    }
}

/// The message as it will be sent, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessagePlan {
    pub message_id: String,
    pub customer_id: String,
    pub order_id: Option<String>,
    pub kind: String,
    pub channel: String,
    pub subject: Option<String>,
    pub text: String,
    /// Return requests that move to `photos_requested` once a photo request is sent.
    pub photo_claims: Vec<String>,
}

pub struct SendCustomerMessage;

impl Operation for SendCustomerMessage {
    const META: OpMeta = OpMeta {
        address: "messaging.message.send",
        tool: "send_customer_message",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/messaging/messages",
        summary: "Send one shopper a message about their order or return (photo request, decline, refund notice, other). The preview shows the message exactly as they will read it; a person always approves, and a sent message cannot be taken back.",
        consumes: &[Slot::one("customer_id"), Slot::one("order_id")],
        produces: &[Slot::one("message_id")],
        checks: &[
            "customer_messages",
            "damage_claim_needs_photo",
            "return_window_days",
            "refund.to",
            "messaging.quiet_hours",
            "change_set.irreversible",
        ],
        errors: &[
            CUSTOMER_NOT_FOUND,
            CUSTOMER_REQUIRED,
            ORDER_NOT_FOUND,
            ORDER_NOT_FOR_CUSTOMER,
            ORDER_REQUIRED,
            REFUND_NOT_FOUND,
            NO_VERIFIED_CONTACT,
            EMPTY_TEXT,
        ],
        audience: Audience::Operator,
        action_type: ActionType::SEND,
        person_scope: "",
    };
    type Input = SendCustomerMessageInput;
    type Output = Value;
}

/// Customer messages are about an order or a claim, never promotions.
const PURPOSE: &str = "transactional";

impl WriteOp<Cx> for SendCustomerMessage {
    type Plan = MessagePlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SendCustomerMessageInput,
    ) -> Result<Planned<MessagePlan>, ToolError> {
        let text = input.text.trim().to_owned();
        if text.is_empty() {
            return Err(ToolError::new(EMPTY_TEXT, "The message has no text.")
                .detail(json!({"field": "text", "reason": "missing"}))
                .next("send_customer_message"));
        }
        if text.chars().count() > MAX_TEXT {
            return Err(ToolError::invalid_input(format!(
                "The message is longer than {MAX_TEXT} characters."
            ))
            .detail(json!({"field": "text", "reason": "range"})));
        }
        let kind = input.kind.as_str();
        let channel = input.channel.unwrap_or(MessageChannel::Email).as_str();
        let now = cx.clock.now();
        let customer_required = || {
            ToolError::new(
                CUSTOMER_REQUIRED,
                "Say which order the message is about (it goes to the order's customer), or which customer it goes to.",
            )
            .detail(json!({"field": "customer_id", "reason": "missing"}))
            .next("send_customer_message")
        };
        if input.customer_id.is_none() && input.order_id.is_none() {
            return Err(customer_required());
        }
        if kind != "custom" && input.order_id.is_none() {
            return Err(
                ToolError::new(ORDER_REQUIRED, "Say which order the message is about.")
                    .detail(json!({"field": "order_id", "reason": "missing"}))
                    .next("send_customer_message"),
            );
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        // The order comes first: without a customer_id the message goes to whoever placed it.
        let order = match &input.order_id {
            Some(id) => Some(sales::order_owner(&mut tx, id).await?.ok_or_else(|| {
                ToolError::new(ORDER_NOT_FOUND, format!("There is no order {id}."))
                    .detail(json!({"entity": "order", "id": id}))
                    .next("get_order")
            })?),
            None => None,
        };
        let customer_id = match (&input.customer_id, &order) {
            (Some(id), _) => id.clone(),
            (None, Some(o)) => o.customer_id.clone().ok_or_else(|| {
                ToolError::new(
                    CUSTOMER_NOT_FOUND,
                    format!(
                        "Order {} was placed as a guest, so there is no customer account to write to.",
                        o.order_number
                    ),
                )
                .detail(json!({"entity": "customer", "order_id": o.id, "reason": "guest_order"}))
                .next("get_order")
            })?,
            (None, None) => return Err(customer_required()),
        };
        let person = customer::contact(&mut tx, &customer_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    CUSTOMER_NOT_FOUND,
                    format!("There is no customer {customer_id}."),
                )
                .detail(json!({"entity": "customer", "id": customer_id}))
                .next("search_members")
            })?;
        if let Some(o) = &order
            && o.customer_id.as_deref() != Some(person.id.as_str())
        {
            return Err(ToolError::new(
                ORDER_NOT_FOR_CUSTOMER,
                format!("Order {} is not {}'s order.", o.order_number, person.name),
            )
            .detail(json!({"reason": "owner"}))
            .next("get_order"));
        }
        if !reachable(
            channel,
            person.email.as_deref(),
            person.telephone.as_deref(),
            &person.member_state,
        ) {
            return Err(ToolError::new(
                NO_VERIFIED_CONTACT,
                format!(
                    "{} cannot be reached by {}.",
                    person.name,
                    channel.replace('_', "-")
                ),
            )
            .detail(json!({"field": "channel", "channel": channel}))
            .next("send_customer_message"));
        }

        let number = order.as_ref().map(|o| o.order_number.clone());
        let mut checks = Vec::new();
        if cx
            .policy
            .lookup("approval.customer_messages")
            .and_then(Value::as_str)
            == Some("always_ask")
        {
            checks.push(Check {
                rule: "customer_messages".into(),
                passed: true,
                detail: Some("Every customer message needs a person's approval".into()),
                fix_tool: None,
            });
        }
        let mut photo_claims = Vec::new();
        let mut basis = Vec::new();
        if let (Some(o), true) = (&order, kind != "custom") {
            let requests = fulfillment::open_requests(&mut tx, &o.id, now).await?;
            let n = &o.order_number;
            match kind {
                "ask_for_photos" => {
                    let waiting: Vec<&fulfillment::OpenRequest> = requests
                        .iter()
                        .filter(|r| {
                            r.reason.as_deref() == Some("damaged_in_transit")
                                && r.photos == 0
                                && matches!(r.status.as_str(), "requested" | "photos_requested")
                        })
                        .collect();
                    for r in waiting.iter().filter(|r| r.status == "requested") {
                        photo_claims.push(r.claim_id.clone());
                        basis.push(Basis {
                            entity: "fulfillment.claim".into(),
                            id: r.claim_id.clone(),
                            version: i64::from(r.version),
                        });
                    }
                    checks.push(Check {
                        rule: "damage_claim_needs_photo".into(),
                        passed: !waiting.is_empty(),
                        detail: Some(if waiting.is_empty() {
                            format!("Order {n} has no damage claim waiting for photos")
                        } else {
                            format!("Order {n} has no photo of the damage")
                        }),
                        fix_tool: waiting.is_empty().then(|| "evaluate_return_request".into()),
                    });
                }
                "decline" => {
                    let limit = cx
                        .policy
                        .lookup("fulfillment.return_window_days")
                        .and_then(Value::as_i64);
                    let late = requests.iter().find(
                        |r| matches!((r.days_since_delivery, limit), (Some(d), Some(l)) if d > l),
                    );
                    checks.push(match (late, limit) {
                        (Some(r), Some(l)) => Check {
                            rule: "return_window_days".into(),
                            passed: true,
                            detail: Some(format!(
                                "Delivered {}, {} days ago (limit {l})",
                                r.delivered_on
                                    .map(|d| d.format("%b %-d").to_string())
                                    .unwrap_or_default(),
                                r.days_since_delivery.unwrap_or_default()
                            )),
                            fix_tool: None,
                        },
                        _ => Check {
                            rule: "return_window_days".into(),
                            passed: false,
                            detail: Some(if requests.is_empty() {
                                format!("Order {n} has no open return request")
                            } else {
                                format!("Order {n}'s request is inside the return window")
                            }),
                            fix_tool: Some("evaluate_return_request".into()),
                        },
                    });
                }
                "refund_done" => {
                    let refunds = fulfillment::refunds(&mut tx, &o.id).await?;
                    let detail = match refunds.first() {
                        Some(r) => r.sentence(),
                        None if requests.iter().any(|r| r.status == "requested") => format!(
                            "The refund for order {n} runs before this message in the change set"
                        ),
                        None => {
                            return Err(ToolError::new(
                                REFUND_NOT_FOUND,
                                format!("Order {n} has no refund to tell the shopper about."),
                            )
                            .detail(json!({"entity": "refund", "order_id": o.id}))
                            .next("refund_return"));
                        }
                    };
                    checks.push(Check {
                        rule: "refund.to".into(),
                        passed: true,
                        detail: Some(detail),
                        fix_tool: None,
                    });
                }
                _ => {}
            }
        }
        tx.commit().await.map_err(ToolError::from)?;

        // Quiet hours hold back only the purposes the policy names (by default marketing).
        if let Some(q) = QuietHours::from_policy(&cx.policy)
            && q.applies(PURPOSE)
        {
            let local = now.with_timezone(&cx.time.zone).time();
            let quiet = q.contains(local);
            checks.push(Check {
                rule: "messaging.quiet_hours".into(),
                passed: !quiet,
                detail: Some(format!(
                    "{} store time{}",
                    local.format("%H:%M"),
                    if quiet {
                        format!(
                            ", inside quiet hours {}–{}",
                            q.start.format("%H:%M"),
                            q.end.format("%H:%M")
                        )
                    } else {
                        String::new()
                    }
                )),
                fix_tool: None,
            });
        }

        let subject = has_subject(channel).then(|| {
            input
                .subject
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| default_subject(kind, number.as_deref()))
        });
        let plan = MessagePlan {
            message_id: MessageId::generate().to_string(),
            customer_id: person.id.clone(),
            order_id: input.order_id.clone(),
            kind: kind.to_owned(),
            channel: channel.to_owned(),
            subject: subject.clone(),
            text: text.clone(),
            photo_claims,
        };
        let mut to = json!({
            "recipient": masked_name(person.given_name.as_deref(), person.family_name.as_deref(), &person.name),
            "customer_id": person.id,
            "about": input.order_id,
            "kind": kind,
            "channel": channel,
            "text": text,
        });
        if let Some(s) = &subject {
            to["subject"] = json!(s);
        }
        Ok(Planned {
            plan,
            changes: vec![Change {
                op: ChangeOp::Send,
                entity: "message".into(),
                id: String::new(),
                from: Value::Null,
                to,
                note: None,
            }],
            checks,
            basis,
            approval: ApprovalNeed::Needed {
                reason: "Sends a message to a customer".into(),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &MessagePlan) -> Result<Committed, ToolError> {
        let done = |id: &str| Committed {
            id: id.to_owned(),
            produced: Map::from_iter([("message_id".to_owned(), json!(id))]),
        };
        if repo::message(tx, &plan.message_id).await?.is_some() {
            return Ok(done(&plan.message_id));
        }
        if let Some(existing) = repo::same_message(
            tx,
            &plan.customer_id,
            plan.order_id.as_deref(),
            &plan.kind,
            &plan.text,
        )
        .await?
        {
            return Ok(done(&existing));
        }
        if plan.kind == "refund_done"
            && let Some(order) = &plan.order_id
            && fulfillment::refunds(tx, order).await?.is_empty()
        {
            return Err(ToolError::new(
                REFUND_NOT_FOUND,
                "No refund has gone out for this order, so the notice is not sent.",
            )
            .detail(json!({"entity": "refund", "order_id": order}))
            .next("refund_return"));
        }
        if let Some(q) = QuietHours::from_policy(&tx.policy)
            && q.applies(PURPOSE)
            && q.contains(tx.clock.now().with_timezone(&tx.time.zone).time())
        {
            return Err(ToolError::new(
                QUIET_HOURS,
                "It is quiet hours at the store; send the message later.",
            )
            .detail(json!({"rule": "messaging.quiet_hours"})));
        }
        repo::insert_message(
            tx,
            &NewMessage {
                id: &plan.message_id,
                recipient_id: &plan.customer_id,
                about_id: plan.order_id.as_deref(),
                kind: &plan.kind,
                channel: &plan.channel,
                headline: plan.subject.as_deref(),
                text: &plan.text,
            },
        )
        .await?;
        if !plan.photo_claims.is_empty() {
            fulfillment::photos_requested(tx, &plan.photo_claims, &plan.message_id).await?;
        }
        Ok(done(&plan.message_id))
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &MessagePlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let m = repo::message(&mut tx, &done.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match m {
            None => vec![format!("message {} is missing", done.id)],
            Some(m) if m.recipient_id != plan.customer_id => {
                vec![format!("message {} went to {}", m.id, m.recipient_id)]
            }
            Some(m) if m.date_sent.is_none() => vec![format!("message {} was not sent", m.id)],
            Some(_) => Vec::new(),
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, _: &mut TxCx, plan: &MessagePlan, _: &Committed) -> Result<(), ToolError> {
        Err(ToolError::new(
            NOT_REVOCABLE,
            format!(
                "Message {} has been sent and cannot be taken back.",
                plan.message_id
            ),
        ))
    }
}
