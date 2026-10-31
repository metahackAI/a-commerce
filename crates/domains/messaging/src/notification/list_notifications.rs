//! `list_notifications`: what a shopper has been sent. Every message the store sent is a
//! notification on its channel, so this reads the sent messages (their `msg_` ids).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::MessageChannel;
use acom_customer::service::for_messaging as customer;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::message::CUSTOMER_NOT_FOUND;
use crate::repo::{self, NotificationFilter};
use crate::template::INVALID_FILTER;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListNotificationsInput {
    /// The shopper; `me` for the signed-in shopper (a shopper only ever sees their own).
    #[schemars(regex(pattern = r"^(cus_[a-z0-9_]+|me)$"), example = &"cus_alex")]
    pub customer_id: String,
    /// Only this channel.
    #[schemars(example = &"email")]
    pub channel: Option<MessageChannel>,
    /// true: only opened ones; false: only unread ones.
    #[schemars(example = false)]
    pub read: Option<bool>,
    /// Only sent at or after this time (store time).
    #[schemars(example = &"2026-11-01T00:00")]
    pub sent_from: Option<String>,
    /// Only sent at or before this time (store time).
    #[schemars(example = &"2026-11-06T23:59")]
    pub sent_through: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListNotifications;

impl Operation for ListNotifications {
    const META: OpMeta = OpMeta {
        address: "messaging.notification.list",
        tool: "list_notifications",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/messaging/notifications",
        summary: "What one shopper was sent (in-app, push, email, sms), newest first, with whether they opened it.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::many("notification_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND, INVALID_FILTER],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListNotificationsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListNotifications {
    async fn read(&self, cx: &Cx, input: ListNotificationsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let not_found = || {
            ToolError::new(
                CUSTOMER_NOT_FOUND,
                format!("There is no customer {}.", input.customer_id),
            )
            .detail(json!({"entity": "customer", "id": input.customer_id}))
            .next("search_members")
        };
        // A shopper sees only their own; `me` means the signed-in shopper.
        let me = cx.principal.customer_id().map(str::to_owned);
        let recipient = match (&me, input.customer_id.as_str()) {
            (Some(me), "me") => me.clone(),
            (Some(me), asked) if asked == me => me.clone(),
            (Some(_), _) | (None, "me") => return Err(not_found()),
            (None, asked) => asked.to_owned(),
        };
        let from = crate::parse_time(cx, "sent_from", input.sent_from.as_deref())?;
        let through = crate::parse_time(cx, "sent_through", input.sent_through.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if customer::contact(&mut tx, &recipient).await?.is_none() {
            return Err(not_found());
        }
        let (total, rows) = repo::notifications(
            &mut tx,
            &NotificationFilter {
                recipient_id: &recipient,
                channel: input.channel.map(MessageChannel::as_str),
                read: input.read,
                from,
                through,
            },
            page.limit,
            page.offset,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|m| {
                json!({"id": m.id, "channel": m.message_channel, "kind": m.message_kind,
                       "headline": m.headline, "text": m.text, "about": m.about_id,
                       "date_sent": m.date_sent.map(|t| cx.time.format(t)),
                       "read": m.date_read.is_some()})
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(next) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(next);
        }
        Ok(out)
    }
}
