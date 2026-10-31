//! `get_campaign` (docs/api/messaging.md#campaign): one campaign, who it is for, and what its
//! send did.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CAMPAIGN_NOT_FOUND, NOT_DELIVERED, Target, stored_values};
use crate::repo::{self, CampaignRow, SendStats};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCampaignInput {
    /// The campaign.
    #[schemars(regex(pattern = r"^cmp_[a-z0-9_]+$"), example = &"cmp_weekly_1105")]
    pub campaign_id: String,
    /// Top-level fields to return.
    #[schemars(example = ["status", "stats"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetCampaign;

impl Operation for GetCampaign {
    const META: OpMeta = OpMeta {
        address: "messaging.campaign.get",
        tool: "get_campaign",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/messaging/campaigns/{campaign_id}",
        summary: "One campaign: template, audience, status, send time, why a send failed, and how many messages went out and were opened.",
        consumes: &[Slot::one("campaign_id")],
        produces: &[],
        checks: &[],
        errors: &[CAMPAIGN_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCampaignInput;
    type Output = Value;
}

/// The send's figures. Messages a send recorded count as sent; delivery is not reported by any
/// provider, and campaigns sent before messages were recorded only know their recipient count.
pub(crate) fn stats(c: &CampaignRow, s: SendStats) -> Value {
    match c.campaign_status.as_str() {
        "sent" if s.messages > 0 => {
            json!({"sent": s.messages, "delivered": null, "opened": s.opened, "failed": 0})
        }
        "sent" => json!({"sent": c.recipient_count, "delivered": null, "opened": null, "failed": 0}),
        "failed" => json!({"sent": 0, "delivered": 0, "opened": 0, "failed": null}),
        _ => json!({"sent": 0, "delivered": 0, "opened": 0, "failed": 0}),
    }
}

/// A campaign as `get_campaign` shows it.
pub(crate) fn campaign_json(cx: &Cx, c: &CampaignRow, s: SendStats) -> Value {
    let mut v = json!({
        "id": c.id,
        "name": c.name,
        "channel": c.message_channel,
        "template_id": c.template_id,
        "valid_for_member_tier": c.valid_for_member_tier,
        "audience": Target::of(c).json(),
        "variables": stored_values(c),
        "status": c.campaign_status,
        "scheduled_time": c.scheduled_time.map(|t| cx.time.format(t)),
        "date_sent": c.date_sent.map(|t| cx.time.format(t)),
        "recipient_count": c.recipient_count,
        "error": c.error,
        "change_set_id": c.change_set_id,
        "stats": stats(c, s),
    });
    if c.campaign_status == "sent" {
        v["delivery_note"] = json!(NOT_DELIVERED);
    }
    v
}

impl ReadOp<Cx> for GetCampaign {
    async fn read(&self, cx: &Cx, input: GetCampaignInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::campaign(&mut tx, &input.campaign_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    CAMPAIGN_NOT_FOUND,
                    format!("There is no campaign {}.", input.campaign_id),
                )
                .detail(json!({"entity": "campaign", "id": input.campaign_id}))
                .next("search_campaigns")
            })?;
        let s = repo::send_stats(&mut tx, &c.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(crate::select(
            campaign_json(cx, &c, s),
            &input.fields.unwrap_or_default(),
        ))
    }
}
