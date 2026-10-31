//! `search_campaigns` (docs/api/messaging.md#campaign): campaigns by status, channel, send time
//! and template.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::{CampaignStatus, MessageChannel};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::repo::{self, CampaignFilter};
use crate::template::INVALID_FILTER;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchCampaignsInput {
    /// draft, scheduled, sending, sent, failed (stopped before any message went out) or
    /// cancelled.
    #[schemars(example = &"failed")]
    pub status: Option<CampaignStatus>,
    /// Only this channel.
    #[schemars(example = &"email")]
    pub channel: Option<MessageChannel>,
    /// Only sends due at or after this time (store time).
    #[schemars(example = &"2026-11-01T00:00")]
    pub scheduled_from: Option<String>,
    /// Only sends due at or before this time (store time).
    #[schemars(example = &"2026-11-07T23:59")]
    pub scheduled_through: Option<String>,
    /// Only sends of this template (the sends of one template are one recurring campaign).
    #[schemars(regex(pattern = r"^tpl_[a-z0-9_]+$"), example = &"tpl_weekly_email")]
    pub template_id: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchCampaigns;

impl Operation for SearchCampaigns {
    const META: OpMeta = OpMeta {
        address: "messaging.campaign.search",
        tool: "search_campaigns",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/messaging/campaigns",
        summary: "Campaigns by status, channel, send time and template, latest send first.",
        consumes: &[],
        produces: &[Slot::many("campaign_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchCampaignsInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchCampaigns {
    async fn read(&self, cx: &Cx, input: SearchCampaignsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let from = crate::parse_time(cx, "scheduled_from", input.scheduled_from.as_deref())?;
        let through = crate::parse_time(cx, "scheduled_through", input.scheduled_through.as_deref())?;
        if let (Some(f), Some(t)) = (from, through)
            && t < f
        {
            return Err(ToolError::new(
                INVALID_FILTER,
                "scheduled_through is before scheduled_from.",
            )
            .detail(json!({"field": "scheduled_through", "reason": "range"}))
            .next("search_campaigns"));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (total, rows) = repo::campaigns(
            &mut tx,
            &CampaignFilter {
                status: input.status.map(CampaignStatus::as_str),
                channel: input.channel.map(MessageChannel::as_str),
                template_id: input.template_id.as_deref(),
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
            .map(|c| {
                json!({"id": c.id, "name": c.name, "channel": c.message_channel,
                       "template_id": c.template_id, "status": c.campaign_status,
                       "scheduled_time": c.scheduled_time.map(|t| cx.time.format(t)),
                       "date_sent": c.date_sent.map(|t| cx.time.format(t)),
                       "recipient_count": c.recipient_count,
                       "error_message": c.error.as_ref().and_then(|e| e.get("message")).cloned()})
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(next) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(next);
        }
        Ok(out)
    }
}
