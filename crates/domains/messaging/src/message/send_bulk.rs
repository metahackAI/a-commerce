//! `send_bulk` (docs/api/messaging.md#message): sends a campaign now to everyone in its audience
//! who opted in to marketing on its channel and is under the daily cap. A person always
//! approves, a campaign goes out once, and sent messages cannot be taken back. Delivery is
//! outside the store: the messages are recorded as sent, and the preview says so.

use std::collections::BTreeMap;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{NOT_REVOCABLE, QUIET_HOURS};
use crate::campaign::{
    ALREADY_SENT, CAMPAIGN_NOT_FOUND, CAMPAIGN_NOT_READY, DAILY_SEND_CAP_EXCEEDED,
    NO_CONSENTED_RECIPIENTS, NOT_DELIVERED, Recipient, Target, daily_cap, greeting, message_for,
    message_id, reach, send_now, stored_values, thousands, under_cap, variable_values,
};
use crate::model::QuietHours;
use crate::repo::{self, CampaignRow, TemplateRow};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SendBulkInput {
    /// The campaign to send (a draft, a scheduled one, or one whose send failed).
    #[schemars(regex(pattern = r"^cmp_[a-z0-9_]+$"), example = &"cmp_vip_weekend")]
    pub campaign_id: String,
}

/// The send as previewed: who gets the campaign.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BulkPlan {
    pub campaign_id: String,
    pub recipients: Vec<Recipient>,
}

pub struct SendBulk;

impl Operation for SendBulk {
    const META: OpMeta = OpMeta {
        address: "messaging.message.send_bulk",
        tool: "send_bulk",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/messaging/campaigns/{campaign_id}/send",
        summary: "Send a campaign now to everyone in its audience who opted in and is under the daily cap; the rest are left out by the server. A person always approves, it goes out once, and sent messages cannot be taken back.",
        consumes: &[Slot::one("campaign_id")],
        produces: &[Slot::many("message_id")],
        checks: &[
            "consent.marketing_gate",
            "messaging.quiet_hours",
            "messaging.daily_send_cap",
            "change_set.irreversible",
        ],
        errors: &[
            CAMPAIGN_NOT_FOUND,
            CAMPAIGN_NOT_READY,
            NO_CONSENTED_RECIPIENTS,
            DAILY_SEND_CAP_EXCEEDED,
            ALREADY_SENT,
            QUIET_HOURS,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::SEND,
        person_scope: "",
    };
    type Input = SendBulkInput;
    type Output = Value;
}

fn not_ready(c: &CampaignRow, why: impl Into<String>) -> ToolError {
    ToolError::new(CAMPAIGN_NOT_READY, why.into())
        .detail(json!({"entity": "campaign", "id": c.id, "status": c.campaign_status}))
        .next("get_campaign")
}

/// What a campaign is sent with: its template and the values every message shares.
pub(crate) async fn ready(
    tx: &mut TxCx,
    c: &CampaignRow,
) -> Result<(TemplateRow, BTreeMap<String, String>), ToolError> {
    match c.campaign_status.as_str() {
        "sent" | "sending" => {
            return Err(ToolError::new(
                ALREADY_SENT,
                format!("{} has already been sent.", c.name),
            )
            .detail(json!({"entity": "campaign", "id": c.id, "status": c.campaign_status}))
            .next("get_campaign"));
        }
        "cancelled" => return Err(not_ready(c, format!("{} was cancelled.", c.name))),
        _ => {}
    }
    let template = match &c.template_id {
        Some(id) => repo::template(tx, id)
            .await?
            .filter(|t| t.template_status != "archived"),
        None => None,
    }
    .ok_or_else(|| not_ready(c, format!("{} has no template to send.", c.name)))?;
    if template.message_channel != c.message_channel {
        return Err(not_ready(
            c,
            format!(
                "{} is written for {}, but the campaign goes out by {}.",
                template.name, template.message_channel, c.message_channel
            ),
        ));
    }
    let values = variable_values(&template, &stored_values(c)).map_err(|_| {
        not_ready(
            c,
            format!(
                "{} has no value for some of the template's variables ({}).",
                c.name,
                template.variables.join(", ")
            ),
        )
    })?;
    Ok((template, values))
}

impl WriteOp<Cx> for SendBulk {
    type Plan = BulkPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(&self, cx: &Cx, input: SendBulkInput) -> Result<Planned<BulkPlan>, ToolError> {
        let now = cx.clock.now();
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
        let (template, values) = ready(&mut tx, &c).await?;
        let channel = c.message_channel.as_str();
        let reached = reach(&mut tx, &Target::of(&c), channel).await?;
        if reached.consented.is_empty() {
            return Err(ToolError::new(
                NO_CONSENTED_RECIPIENTS,
                format!(
                    "Nobody {} reaches can get marketing {}: {}.",
                    c.name,
                    channel.replace('_', "-"),
                    reached.consent_detail(channel)
                ),
            )
            .detail(json!({"rule": "consent.marketing_gate", "addressed": reached.addressed}))
            .next("preview_segment"));
        }
        let opted_in = reached.consented.len();
        let (recipients, capped) = under_cap(&mut tx, reached.consented.clone(), now).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let cap = daily_cap(&cx.policy);
        if recipients.is_empty() {
            return Err(ToolError::new(
                DAILY_SEND_CAP_EXCEEDED,
                format!(
                    "Everyone {} reaches already got {} marketing message{} today; send it another day.",
                    c.name,
                    cap.unwrap_or_default(),
                    if cap == Some(1) { "" } else { "s" }
                ),
            )
            .detail(json!({"limit": cap, "actual": opted_in,
                           "policy_key": "messaging.daily_send_cap"}))
            .next("create_campaign"));
        }

        let n = recipients.len();
        let mut checks = vec![Check {
            rule: "consent.marketing_gate".into(),
            passed: true,
            detail: Some(reached.consent_detail(channel)),
            fix_tool: Some("get_consent".into()),
        }];
        if let Some(q) = QuietHours::from_policy(&cx.policy).filter(|q| q.applies("marketing")) {
            let local = now.with_timezone(&cx.time.zone).time();
            let quiet = q.contains(local);
            checks.push(Check {
                rule: "messaging.quiet_hours".into(),
                passed: !quiet,
                detail: Some(if quiet {
                    format!(
                        "{} store time is inside quiet hours {}–{}",
                        local.format("%H:%M"),
                        q.start.format("%H:%M"),
                        q.end.format("%H:%M")
                    )
                } else {
                    format!("Sends at {} store time", local.format("%H:%M"))
                }),
                fix_tool: Some("create_campaign".into()),
            });
        }
        if let Some(cap) = cap {
            checks.push(Check {
                rule: "messaging.daily_send_cap".into(),
                passed: true,
                detail: Some(if capped == 0 {
                    format!("Everyone is under {cap} marketing messages today")
                } else {
                    format!(
                        "{} already got {cap} marketing messages today and {} left out",
                        thousands(capped),
                        if capped == 1 { "is" } else { "are" }
                    )
                }),
                fix_tool: None,
            });
        }
        let sample = &recipients[0];
        let (subject, text) = message_for(&template, channel, &values, &sample.masked());
        let mut to = json!({
            "campaign_id": c.id,
            "name": c.name,
            "recipients": n,
            "left_out": reached.addressed - n,
            "channel": channel,
            "sample": {"recipient": sample.masked(), "text": text},
        });
        if let Some(s) = subject {
            to["sample"]["subject"] = json!(s);
        }
        Ok(Planned {
            plan: BulkPlan {
                campaign_id: c.id.clone(),
                recipients,
            },
            changes: vec![Change {
                op: ChangeOp::Send,
                entity: "message".into(),
                id: String::new(),
                from: Value::Null,
                to,
                note: Some(NOT_DELIVERED.into()),
            }],
            checks,
            basis: vec![Basis {
                entity: "messaging.campaign".into(),
                id: c.id.clone(),
                version: i64::from(c.version),
            }],
            approval: ApprovalNeed::Needed {
                reason: format!(
                    "Sends {} to {} customer{}",
                    channel.replace('_', "-"),
                    thousands(n),
                    if n == 1 { "" } else { "s" }
                ),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &BulkPlan) -> Result<Committed, ToolError> {
        let done = |ids: Vec<String>| Committed {
            id: plan.campaign_id.clone(),
            produced: Map::from_iter([
                ("campaign_id".to_owned(), json!(plan.campaign_id)),
                ("message_id".to_owned(), json!(ids)),
            ]),
        };
        let c = repo::campaign_for_send(tx, &plan.campaign_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    CAMPAIGN_NOT_FOUND,
                    format!("There is no campaign {}.", plan.campaign_id),
                )
                .next("search_campaigns")
            })?;
        if c.campaign_status == "sent" {
            // A campaign goes out once: the same send again is its first result.
            let ids = plan
                .recipients
                .iter()
                .map(|r| message_id(&c.id, &r.customer_id))
                .collect();
            return Ok(done(ids));
        }
        let (template, values) = ready(tx, &c).await?;
        let now = tx.clock.now();
        if let Some(q) = QuietHours::from_policy(&tx.policy).filter(|q| q.applies("marketing"))
            && q.contains(now.with_timezone(&tx.time.zone).time())
        {
            return Err(ToolError::new(
                QUIET_HOURS,
                "It is quiet hours at the store; send the campaign later.",
            )
            .detail(json!({"rule": "messaging.quiet_hours"}))
            .next("create_campaign"));
        }
        let (recipients, _) = under_cap(tx, plan.recipients.clone(), now).await?;
        if recipients.is_empty() {
            return Err(ToolError::new(
                DAILY_SEND_CAP_EXCEEDED,
                "Everyone left already got today's marketing messages; send it another day.",
            )
            .detail(json!({"policy_key": "messaging.daily_send_cap"}))
            .next("create_campaign"));
        }
        let ids = send_now(tx, &c, &template, &values, &recipients).await?;
        Ok(done(ids))
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &BulkPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::campaign(&mut tx, &plan.campaign_id).await?;
        let s = repo::send_stats(&mut tx, &plan.campaign_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match c {
            None => vec![format!("campaign {} is missing", plan.campaign_id)],
            Some(c) if c.campaign_status != "sent" => {
                vec![format!("campaign {} is {}", c.id, c.campaign_status)]
            }
            Some(c) if s.messages == 0 => vec![format!("campaign {} has no messages", c.id)],
            Some(_) => Vec::new(),
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, _: &mut TxCx, plan: &BulkPlan, _: &Committed) -> Result<(), ToolError> {
        Err(ToolError::new(
            NOT_REVOCABLE,
            format!(
                "Campaign {} has been sent and cannot be taken back.",
                plan.campaign_id
            ),
        ))
    }
}

/// The name a preview greets the sample shopper by (masked) and a sent message by (in full).
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_campaign_goes_out_once() {
        let r = Recipient {
            customer_id: "cus_jamie".into(),
            name: "Jamie Chen".into(),
            given_name: Some("Jamie".into()),
            family_name: Some("Chen".into()),
        };
        assert_eq!(greeting(&r), "Jamie");
        assert_eq!(r.masked(), "J*** C***");
        assert_eq!(SendBulk::META.kind, Kind::WriteApproval);
        assert!(matches!(SendBulk::UNDO, UndoSupport::NotSupported));
    }
}
