//! `create_campaign` (docs/api/messaging.md#campaign): sets up a campaign for a tier, a segment
//! or a list of shoppers. With a send time it is booked, and the scheduled send
//! (`messaging.send_campaign`) sends it then; without one it waits as a draft for `send_bulk`.
//! It reaches customers, so a person always approves it.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::MessageChannel;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    AudienceMode, CampaignId, NO_CONSENTED_RECIPIENTS, TEMPLATE_CHANNEL_MISMATCH, Target,
    VariableProblem, VariableValue, reach, variable_values,
};
use crate::message::{CUSTOMER_NOT_FOUND, NOT_REVOCABLE, QUIET_HOURS};
use crate::model::QuietHours;
use crate::repo::{self, NewCampaign};
use crate::template::UNKNOWN_VARIABLE;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCampaignInput {
    /// The name operators know the campaign by.
    #[schemars(length(min = 1, max = 120), example = &"VIP weekend reminder")]
    pub name: String,
    /// email, sms, push or in_app; the template must be written for the same channel.
    #[schemars(example = &"email")]
    pub channel: MessageChannel,
    /// The template the message is made from (list_message_templates).
    #[schemars(regex(pattern = r"^tpl_[a-z0-9_]+$"), example = &"tpl_vip_weekend")]
    pub template_id: String,
    /// segment: the shoppers in segment_id; customers: the shoppers in customer_ids; tier: the
    /// members of valid_for_member_tier.
    #[schemars(example = &"tier")]
    pub audience_mode: AudienceMode,
    /// The segment, with audience_mode segment.
    #[schemars(regex(pattern = r"^seg_[a-z0-9_]+$"), example = &"seg_lapsed_90")]
    pub segment_id: Option<String>,
    /// The shoppers, with audience_mode customers.
    #[schemars(inner(regex(pattern = r"^cus_[a-z0-9_]+$")), length(min = 1, max = 1000), example = ["cus_jamie"])]
    pub customer_ids: Option<Vec<String>>,
    /// The tier id, with audience_mode tier.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub valid_for_member_tier: Option<String>,
    /// When to send, in store time. Left out, the campaign waits as a draft and send_bulk sends
    /// it once a person approves.
    #[schemars(example = &"2026-11-07T10:00")]
    pub scheduled_time: Option<String>,
    /// Values of the template variables that are the same for every shopper; customer_name is
    /// filled for each shopper.
    #[schemars(example = json!([{"name": "headline_item", "value": "the wool overcoat"}]))]
    pub variables: Option<Vec<VariableValue>>,
}

/// The campaign as it will be set up, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CampaignPlan {
    pub campaign_id: String,
    pub name: String,
    pub channel: String,
    pub template_id: String,
    pub target: Target,
    pub variables: Vec<VariableValue>,
    pub scheduled_time: Option<DateTime<Utc>>,
    /// Shoppers the campaign reaches after the consent gate, at preview time.
    pub recipients: i32,
}

pub struct CreateCampaign;

impl Operation for CreateCampaign {
    const META: OpMeta = OpMeta {
        address: "messaging.campaign.create",
        tool: "create_campaign",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/messaging/campaigns",
        summary: "Set up a marketing campaign for a tier, a segment or a list of shoppers from one template, at a send time or as a draft for send_bulk. Shoppers who did not opt in are left out; a person always approves.",
        consumes: &[
            Slot::one("segment_id"),
            Slot::many("customer_id"),
            Slot::one("template_id"),
        ],
        produces: &[Slot::one("campaign_id")],
        checks: &[
            "consent.marketing_gate",
            "messaging.quiet_hours",
            "messaging.template.channel_match",
        ],
        errors: &[
            NO_CONSENTED_RECIPIENTS,
            TEMPLATE_CHANNEL_MISMATCH,
            QUIET_HOURS,
            UNKNOWN_VARIABLE,
            CUSTOMER_NOT_FOUND,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::SCHEDULE,
        person_scope: "",
    };
    type Input = CreateCampaignInput;
    type Output = Value;
}

fn missing(field: &str, mode: &str) -> ToolError {
    ToolError::invalid_input(format!("With audience_mode {mode}, say {field}."))
        .detail(json!({"field": field, "reason": "missing"}))
        .next("create_campaign")
}

/// The audience the input names, refusing fields of another mode.
fn target(input: &CreateCampaignInput) -> Result<Target, ToolError> {
    let (seg, list, tier) = (
        input.segment_id.clone(),
        input.customer_ids.clone(),
        input.valid_for_member_tier.clone(),
    );
    let extra = match input.audience_mode {
        AudienceMode::Segment => list.is_some() || tier.is_some(),
        AudienceMode::Customers => seg.is_some() || tier.is_some(),
        AudienceMode::Tier => seg.is_some() || list.is_some(),
    };
    if extra {
        return Err(ToolError::invalid_input(
            "Give only the audience field of the audience_mode you chose.",
        )
        .detail(json!({"field": "audience_mode", "reason": "enum"}))
        .next("create_campaign"));
    }
    match input.audience_mode {
        AudienceMode::Segment => Ok(Target {
            segment_id: Some(seg.ok_or_else(|| missing("segment_id", "segment"))?),
            ..Target::default()
        }),
        AudienceMode::Customers => {
            let mut ids = list
                .filter(|l| !l.is_empty())
                .ok_or_else(|| missing("customer_ids", "customers"))?;
            ids.sort();
            ids.dedup();
            Ok(Target {
                customer_ids: Some(ids),
                ..Target::default()
            })
        }
        AudienceMode::Tier => Ok(Target {
            tier: Some(tier.ok_or_else(|| missing("valid_for_member_tier", "tier"))?),
            ..Target::default()
        }),
    }
}

impl WriteOp<Cx> for CreateCampaign {
    type Plan = CampaignPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateCampaignInput,
    ) -> Result<Planned<CampaignPlan>, ToolError> {
        let name = input.name.trim().to_owned();
        if name.is_empty() {
            return Err(ToolError::invalid_input("The campaign needs a name.")
                .detail(json!({"field": "name", "reason": "missing"})));
        }
        let target = target(&input)?;
        let channel = input.channel.as_str();
        let now = cx.clock.now();
        let scheduled = crate::parse_time(cx, "scheduled_time", input.scheduled_time.as_deref())?;
        if let Some(at) = scheduled
            && at <= now
        {
            return Err(ToolError::invalid_input(format!(
                "{} has passed; pick a later send time.",
                cx.time.format(at)
            ))
            .detail(json!({"field": "scheduled_time", "reason": "past"}))
            .next("create_campaign"));
        }
        let send_at = scheduled.unwrap_or(now);
        let local = send_at.with_timezone(&cx.time.zone).time();
        let quiet = QuietHours::from_policy(&cx.policy).filter(|q| q.applies("marketing"));
        if let Some(q) = &quiet
            && q.contains(local)
        {
            return Err(ToolError::new(
                QUIET_HOURS,
                format!(
                    "{} store time is inside quiet hours {}–{}; pick another send time.",
                    local.format("%H:%M"),
                    q.start.format("%H:%M"),
                    q.end.format("%H:%M")
                ),
            )
            .detail(json!({"rule": "messaging.quiet_hours"}))
            .next("create_campaign"));
        }

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let template = repo::template(&mut tx, &input.template_id)
            .await?
            .filter(|t| t.template_status != "archived")
            .ok_or_else(|| {
                ToolError::new(
                    ErrorCode::NOT_FOUND,
                    format!("There is no template {}.", input.template_id),
                )
                .detail(json!({"entity": "template", "id": input.template_id}))
                .next("list_message_templates")
            })?;
        if template.message_channel != channel {
            return Err(ToolError::new(
                TEMPLATE_CHANNEL_MISMATCH,
                format!(
                    "{} is written for {}, not {}.",
                    template.name,
                    template.message_channel.replace('_', "-"),
                    channel.replace('_', "-")
                ),
            )
            .detail(json!({"rule": "messaging.template.channel_match",
                           "template_channel": template.message_channel}))
            .next("list_message_templates"));
        }
        let given = input.variables.clone().unwrap_or_default();
        match variable_values(&template, &given) {
            Ok(_) => {}
            Err(VariableProblem::Undeclared(v)) => {
                return Err(ToolError::new(
                    UNKNOWN_VARIABLE,
                    format!("{} has no variable {v}.", template.name),
                )
                .detail(json!({"field": "variables", "items": [v]}))
                .next("list_message_templates"));
            }
            Err(VariableProblem::Missing(vs)) => {
                return Err(ToolError::invalid_input(format!(
                    "Give a value for {} ({} uses it).",
                    vs.join(", "),
                    template.name
                ))
                .detail(json!({"field": "variables", "reason": "missing", "items": vs}))
                .next("create_campaign"));
            }
        }
        let reached = reach(&mut tx, &target, channel).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if let Some(first) = reached.unknown.first() {
            return Err(ToolError::new(
                CUSTOMER_NOT_FOUND,
                format!("There is no customer {first}."),
            )
            .detail(json!({"entity": "customer", "id": first, "items": reached.unknown}))
            .next("search_members"));
        }
        if reached.consented.is_empty() {
            return Err(ToolError::new(
                NO_CONSENTED_RECIPIENTS,
                format!(
                    "Nobody in this audience can get marketing {}: {}.",
                    channel.replace('_', "-"),
                    reached.consent_detail(channel)
                ),
            )
            .detail(json!({"rule": "consent.marketing_gate", "addressed": reached.addressed,
                           "not_consented": reached.not_consented,
                           "unreachable": reached.unreachable}))
            .next("preview_segment"));
        }

        let n = reached.consented.len();
        let recipients = i32::try_from(n).unwrap_or(i32::MAX);
        let mut checks = vec![Check {
            rule: "consent.marketing_gate".into(),
            passed: true,
            detail: Some(reached.consent_detail(channel)),
            fix_tool: Some("get_consent".into()),
        }];
        if quiet.is_some() {
            checks.push(Check {
                rule: "messaging.quiet_hours".into(),
                passed: true,
                detail: Some(match scheduled {
                    Some(_) => format!("Sends at {} store time", local.format("%H:%M")),
                    None => "Sends when send_bulk runs, outside quiet hours".into(),
                }),
                fix_tool: Some("create_campaign".into()),
            });
        }
        checks.push(Check {
            rule: "messaging.template.channel_match".into(),
            passed: true,
            detail: Some(format!(
                "{} template on {}",
                capitalized(&channel.replace('_', "-")),
                channel.replace('_', "-")
            )),
            fix_tool: Some("list_message_templates".into()),
        });
        let status = if scheduled.is_some() {
            "scheduled"
        } else {
            "draft"
        };
        let mut to = json!({
            "name": name,
            "status": status,
            "scheduled_time": scheduled.map(|t| cx.time.format(t)),
            "channel": channel,
            "template_id": template.id,
            "recipients": n,
            "left_out": reached.addressed - n,
        });
        if let Value::Object(audience) = target.json() {
            for (k, v) in audience {
                to[k] = v;
            }
        }
        let verb = if scheduled.is_some() {
            "Schedules"
        } else {
            "Sets up"
        };
        let plan = CampaignPlan {
            campaign_id: CampaignId::generate().to_string(),
            name,
            channel: channel.to_owned(),
            template_id: template.id.clone(),
            target,
            variables: given,
            scheduled_time: scheduled,
            recipients,
        };
        Ok(Planned {
            changes: vec![Change {
                op: if scheduled.is_some() {
                    ChangeOp::Schedule
                } else {
                    ChangeOp::Create
                },
                entity: "campaign".into(),
                id: String::new(),
                from: Value::Null,
                to,
                note: None,
            }],
            checks,
            // Nothing read here changes under a campaign that is only being set up: the template
            // is fixed by id, and who is reached is counted again when it is sent.
            basis: Vec::new(),
            approval: ApprovalNeed::Needed {
                reason: format!(
                    "{verb} {} to {} customer{}",
                    channel.replace('_', "-"),
                    super::thousands(n),
                    if n == 1 { "" } else { "s" }
                ),
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &CampaignPlan) -> Result<Committed, ToolError> {
        let done = |id: &str| Committed {
            id: id.to_owned(),
            produced: Map::from_iter([("campaign_id".to_owned(), json!(id))]),
        };
        if repo::campaign(tx, &plan.campaign_id).await?.is_some() {
            return Ok(done(&plan.campaign_id));
        }
        let values = serde_json::to_value(&plan.variables).unwrap_or(Value::Null);
        let new = NewCampaign {
            id: &plan.campaign_id,
            name: &plan.name,
            channel: &plan.channel,
            template_id: &plan.template_id,
            tier: plan.target.tier.as_deref(),
            segment_id: plan.target.segment_id.as_deref(),
            recipient_ids: plan.target.customer_ids.as_deref(),
            variable_values: &values,
            status: if plan.scheduled_time.is_some() {
                "scheduled"
            } else {
                "draft"
            },
            scheduled_time: plan.scheduled_time,
            recipient_count: plan.recipients,
        };
        if let Some(existing) = repo::same_campaign(tx, &new).await? {
            return Ok(done(&existing));
        }
        repo::insert_campaign(tx, &new).await?;
        if let Some(at) = plan.scheduled_time {
            crate::schedule::book_send(tx, &plan.campaign_id, at).await?;
        }
        Ok(done(&plan.campaign_id))
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CampaignPlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::campaign(&mut tx, &done.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match c {
            None => vec![format!("campaign {} is missing", done.id)],
            Some(c) if c.template_id.as_deref() != Some(plan.template_id.as_str()) => {
                vec![format!("campaign {} uses another template", c.id)]
            }
            Some(c) if c.scheduled_time != plan.scheduled_time => {
                vec![format!("campaign {} has another send time", c.id)]
            }
            Some(_) => Vec::new(),
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    /// Cancels the campaign while nothing has gone out; the scheduled send then does nothing.
    async fn undo(&self, tx: &mut TxCx, _: &CampaignPlan, done: &Committed) -> Result<(), ToolError> {
        let Some(c) = repo::campaign_for_send(tx, &done.id).await? else {
            return Ok(());
        };
        match c.campaign_status.as_str() {
            "draft" | "scheduled" | "failed" => {
                repo::set_campaign_status(tx, &c.id, "cancelled", None, None, None).await
            }
            "cancelled" => Ok(()),
            _ => Err(ToolError::new(
                NOT_REVOCABLE,
                format!("{} has been sent and cannot be taken back.", c.name),
            )
            .detail(json!({"entity": "campaign", "id": c.id, "status": c.campaign_status}))),
        }
    }
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}
