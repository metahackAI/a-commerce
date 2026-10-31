//! The `campaign` resource (docs/api/messaging.md#campaign): `create_campaign`, `get_campaign`
//! and `search_campaigns`, one file each, and what campaigns share with `send_bulk` and the
//! scheduled send (`crate::schedule`): who a campaign reaches (`consent.marketing_gate`, the
//! daily cap), the message each shopper gets, and the send itself.
//!
//! Email, SMS and push delivery are outside the store: a send records each shopper's message as
//! sent and says that no delivery provider delivered it.

use std::collections::BTreeMap;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_customer::service::for_messaging::{self as customer, Contact};
use acom_policy::PolicySet;
use acom_store::TxCx;
use chrono::{DateTime, Days, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{PER_SHOPPER_VARIABLES, has_subject, marketing_reachable, masked_name, render};
use crate::repo::{self, CampaignMessage, CampaignRow, TemplateRow};
use crate::service::providers;

mod create_campaign;
mod get_campaign;
mod search_campaigns;

pub use create_campaign::*;
pub use get_campaign::*;
pub use search_campaigns::*;

acom_core::define_id!(
    /// A campaign (`messaging.campaign`).
    CampaignId,
    "cmp"
);

pub const CAMPAIGN_NOT_FOUND: ErrorCode =
    ErrorCode::new("campaign_not_found", ErrorClass::NotFound);
pub const TEMPLATE_CHANNEL_MISMATCH: ErrorCode =
    ErrorCode::new("template_channel_mismatch", ErrorClass::Domain);
pub const NO_CONSENTED_RECIPIENTS: ErrorCode =
    ErrorCode::new("no_consented_recipients", ErrorClass::Domain);
pub const CAMPAIGN_NOT_READY: ErrorCode =
    ErrorCode::new("campaign_not_ready", ErrorClass::Domain);
pub const ALREADY_SENT: ErrorCode = ErrorCode::new("already_sent", ErrorClass::InvalidState);
pub const DAILY_SEND_CAP_EXCEEDED: ErrorCode =
    ErrorCode::new("daily_send_cap_exceeded", ErrorClass::LimitExceeded);

/// Who a campaign goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AudienceMode {
    /// The shoppers in a segment (`segment_id`).
    Segment,
    /// The shoppers listed in `customer_ids`.
    Customers,
    /// The members of a tier (`valid_for_member_tier`).
    Tier,
}

/// One value of a template variable, the same for every shopper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariableValue {
    /// A variable the template declares.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"headline_item")]
    pub name: String,
    /// What the variable says in every shopper's message.
    #[schemars(length(min = 1, max = 500), example = &"the wool overcoat")]
    pub value: String,
}

/// Who a campaign is for, as stored on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Target {
    pub tier: Option<String>,
    pub segment_id: Option<String>,
    pub customer_ids: Option<Vec<String>>,
}

impl Target {
    pub(crate) fn of(c: &CampaignRow) -> Self {
        Self {
            tier: c.valid_for_member_tier.clone(),
            segment_id: c.segment_id.clone(),
            customer_ids: c.recipient_ids.clone(),
        }
    }

    pub(crate) fn mode(&self) -> Option<AudienceMode> {
        if self.segment_id.is_some() {
            Some(AudienceMode::Segment)
        } else if self.customer_ids.is_some() {
            Some(AudienceMode::Customers)
        } else if self.tier.is_some() {
            Some(AudienceMode::Tier)
        } else {
            None
        }
    }

    /// The audience as the API shows it.
    pub(crate) fn json(&self) -> Value {
        match self.mode() {
            Some(AudienceMode::Segment) => {
                json!({"audience_mode": "segment", "segment_id": self.segment_id})
            }
            Some(AudienceMode::Customers) => {
                json!({"audience_mode": "customers", "customer_ids": self.customer_ids})
            }
            Some(AudienceMode::Tier) => {
                json!({"audience_mode": "tier", "valid_for_member_tier": self.tier})
            }
            None => json!({"audience_mode": null, "note": "every member who agreed to marketing"}),
        }
    }
}

/// A shopper a campaign message goes to, with the name it greets them by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipient {
    pub customer_id: String,
    pub name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
}

impl Recipient {
    fn of(c: &Contact) -> Self {
        Self {
            customer_id: c.id.clone(),
            name: c.name.clone(),
            given_name: c.given_name.clone(),
            family_name: c.family_name.clone(),
        }
    }

    /// `R*** P***`, as previews show a shopper.
    pub fn masked(&self) -> String {
        masked_name(
            self.given_name.as_deref(),
            self.family_name.as_deref(),
            &self.name,
        )
    }
}

/// Who an audience reaches on a channel.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reached {
    /// Shoppers in the audience.
    pub addressed: usize,
    /// Reachable on the channel and opted in to marketing on it.
    pub consented: Vec<Recipient>,
    /// Reachable but not opted in.
    pub not_consented: usize,
    /// No address or number for the channel, or an account that cannot get marketing.
    pub unreachable: usize,
    /// Listed shoppers the store does not have.
    pub unknown: Vec<String>,
}

impl Reached {
    /// The `consent.marketing_gate` sentence: "1 of 4 opted in to email; 3 left out".
    pub fn consent_detail(&self, channel: &str) -> String {
        let left = self.addressed - self.consented.len();
        format!(
            "{} of {} opted in to {}; {left} left out",
            thousands(self.consented.len()),
            thousands(self.addressed),
            channel.replace('_', "-"),
        )
    }
}

/// `1,198`.
pub fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Who an audience reaches on `channel`, after the marketing consent gate.
pub(crate) async fn reach(
    tx: &mut TxCx,
    target: &Target,
    channel: &str,
) -> Result<Reached, ToolError> {
    let members: Vec<String>;
    let who = match target.mode() {
        Some(AudienceMode::Segment) => {
            let id = target.segment_id.as_deref().unwrap_or_default();
            let Some(reader) = providers::segment_members() else {
                return Err(ToolError::invalid_input(
                    "Segments cannot be read in this store yet; set the audience by tier or as a list of shoppers.",
                )
                .detail(json!({"field": "audience_mode", "reason": "enum"}))
                .next("create_campaign"));
            };
            members = reader(tx, id).await?.ok_or_else(|| {
                ToolError::new(ErrorCode::NOT_FOUND, format!("There is no segment {id}."))
                    .detail(json!({"entity": "segment", "id": id}))
                    .next("list_segments")
            })?;
            customer::Audience::Customers(&members)
        }
        Some(AudienceMode::Customers) => {
            customer::Audience::Customers(target.customer_ids.as_deref().unwrap_or_default())
        }
        Some(AudienceMode::Tier) => {
            let tier = target.tier.as_deref().unwrap_or_default();
            if !customer::tier_exists(tx, tier).await? {
                return Err(
                    ToolError::new(ErrorCode::NOT_FOUND, format!("There is no tier {tier}."))
                        .detail(json!({"entity": "tier", "id": tier}))
                        .next("list_tiers"),
                );
            }
            customer::Audience::Tier(tier)
        }
        None => customer::Audience::Everyone,
    };
    let rows = customer::audience(tx, who, channel).await?;
    let mut out = Reached {
        addressed: rows.len(),
        ..Reached::default()
    };
    if let customer::Audience::Customers(listed) = who {
        out.unknown = listed
            .iter()
            .filter(|id| !rows.iter().any(|r| &r.contact.id == *id))
            .cloned()
            .collect();
    }
    for r in rows {
        let c = &r.contact;
        if !marketing_reachable(
            channel,
            c.email.as_deref(),
            c.telephone.as_deref(),
            &c.member_state,
        ) {
            out.unreachable += 1;
        } else if !r.consented {
            out.not_consented += 1;
        } else {
            out.consented.push(Recipient::of(c));
        }
    }
    Ok(out)
}

/// `messaging.daily_send_cap`: marketing messages one shopper may get in a store day.
pub(crate) fn daily_cap(policy: &PolicySet) -> Option<i64> {
    policy
        .lookup("messaging.messaging.daily_send_cap")
        .and_then(Value::as_i64)
}

/// The recipients still under the daily cap on the store day of `now`, and how many are at it.
pub(crate) async fn under_cap(
    tx: &mut TxCx,
    recipients: Vec<Recipient>,
    now: DateTime<Utc>,
) -> Result<(Vec<Recipient>, usize), ToolError> {
    let Some(cap) = daily_cap(&tx.policy) else {
        return Ok((recipients, 0));
    };
    let zone = tx.time.zone;
    let day = now.with_timezone(&zone).date_naive();
    let start = |d: chrono::NaiveDate| {
        d.and_hms_opt(0, 0, 0)
            .and_then(|t| t.and_local_timezone(zone).earliest())
            .map(|t| t.with_timezone(&Utc))
    };
    let (Some(from), Some(to)) = (start(day), day.checked_add_days(Days::new(1)).and_then(start))
    else {
        return Ok((recipients, 0));
    };
    let ids: Vec<String> = recipients.iter().map(|r| r.customer_id.clone()).collect();
    let sent: BTreeMap<String, i64> = repo::marketing_sent(tx, &ids, from, to)
        .await?
        .into_iter()
        .collect();
    let (keep, capped): (Vec<Recipient>, Vec<Recipient>) = recipients
        .into_iter()
        .partition(|r| sent.get(&r.customer_id).copied().unwrap_or(0) < cap);
    Ok((keep, capped.len()))
}

/// The values every shopper's message is filled with, or the variables that have none. A value
/// for a variable the template does not declare is refused.
pub(crate) fn variable_values(
    template: &TemplateRow,
    given: &[VariableValue],
) -> Result<BTreeMap<String, String>, VariableProblem> {
    let mut values = BTreeMap::new();
    for v in given {
        if !template.variables.contains(&v.name) {
            return Err(VariableProblem::Undeclared(v.name.clone()));
        }
        values.insert(v.name.clone(), v.value.clone());
    }
    let missing: Vec<String> = template
        .variables
        .iter()
        .filter(|v| !values.contains_key(*v) && !PER_SHOPPER_VARIABLES.contains(&v.as_str()))
        .cloned()
        .collect();
    if missing.is_empty() {
        Ok(values)
    } else {
        Err(VariableProblem::Missing(missing))
    }
}

/// Why a campaign's template cannot be filled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VariableProblem {
    Undeclared(String),
    Missing(Vec<String>),
}

/// The values stored on a campaign (`variable_values`, `[{name, value}]`).
pub(crate) fn stored_values(c: &CampaignRow) -> Vec<VariableValue> {
    c.variable_values
        .clone()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// One shopper's subject and text.
pub(crate) fn message_for(
    template: &TemplateRow,
    channel: &str,
    values: &BTreeMap<String, String>,
    greet_as: &str,
) -> (Option<String>, String) {
    let mut values = values.clone();
    values.insert("customer_name".into(), greet_as.to_owned());
    let headline = template
        .headline
        .as_deref()
        .filter(|_| has_subject(channel))
        .map(|h| render(h, &values));
    (headline, render(&template.text, &values))
}

/// The name a message greets a shopper by: the given name, else the full name.
pub(crate) fn greeting(r: &Recipient) -> String {
    r.given_name
        .clone()
        .filter(|g| !g.trim().is_empty())
        .unwrap_or_else(|| r.name.clone())
}

/// The message id of one shopper's copy of a campaign: the same send never makes two.
pub(crate) fn message_id(campaign_id: &str, customer_id: &str) -> String {
    format!("msg_{campaign_id}_{customer_id}")
}

/// Records each recipient's message as sent now and the campaign as sent. Delivery is outside
/// the store, so nothing is handed to a provider. Returns the message ids.
pub(crate) async fn send_now(
    tx: &mut TxCx,
    campaign: &CampaignRow,
    template: &TemplateRow,
    values: &BTreeMap<String, String>,
    recipients: &[Recipient],
) -> Result<Vec<String>, ToolError> {
    let channel = campaign.message_channel.as_str();
    let messages: Vec<CampaignMessage> = recipients
        .iter()
        .map(|r| {
            let (headline, text) = message_for(template, channel, values, &greeting(r));
            CampaignMessage {
                id: message_id(&campaign.id, &r.customer_id),
                recipient_id: r.customer_id.clone(),
                headline,
                text,
            }
        })
        .collect();
    repo::insert_campaign_messages(tx, &campaign.id, channel, &messages).await?;
    let now = tx.clock.now();
    let count = i32::try_from(messages.len()).unwrap_or(i32::MAX);
    repo::set_campaign_status(tx, &campaign.id, "sent", Some(now), Some(count), None).await?;
    Ok(messages.into_iter().map(|m| m.id).collect())
}

/// What a send leaves to say about delivery.
pub const NOT_DELIVERED: &str = "No email, SMS or push provider is connected: the messages are recorded as sent and nothing was delivered outside the store.";

#[cfg(test)]
mod tests {
    use super::*;

    fn template(vars: &[&str]) -> TemplateRow {
        TemplateRow {
            id: "tpl_vip_weekend".into(),
            name: "VIP weekend".into(),
            message_channel: "email".into(),
            message_purpose: "marketing".into(),
            headline: Some("VIP weekend: {{headline_item}}".into()),
            text: "Hi {{customer_name}}, {{headline_item}} is yours first.".into(),
            variables: vars.iter().map(|v| (*v).to_owned()).collect(),
            in_language: "en-US".into(),
            template_status: "in_use".into(),
            created_at: chrono::DateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn every_shared_variable_needs_a_value() {
        let t = template(&["customer_name", "headline_item"]);
        assert_eq!(
            variable_values(&t, &[]),
            Err(VariableProblem::Missing(vec!["headline_item".into()]))
        );
        let given = [VariableValue {
            name: "headline_item".into(),
            value: "the wool overcoat".into(),
        }];
        let values = variable_values(&t, &given).unwrap();
        let (headline, text) = message_for(&t, "email", &values, "Jamie");
        assert_eq!(headline.as_deref(), Some("VIP weekend: the wool overcoat"));
        assert_eq!(text, "Hi Jamie, the wool overcoat is yours first.");
        let (headline, _) = message_for(&t, "sms", &values, "Jamie");
        assert_eq!(headline, None);
        let wrong = [VariableValue {
            name: "size".into(),
            value: "M".into(),
        }];
        assert_eq!(
            variable_values(&t, &wrong),
            Err(VariableProblem::Undeclared("size".into()))
        );
    }

    #[test]
    fn audiences_read_as_people_say_them() {
        assert_eq!(thousands(1198), "1,198");
        assert_eq!(thousands(12), "12");
        assert_eq!(thousands(1_000_000), "1,000,000");
        let r = Reached {
            addressed: 1284,
            consented: vec![
                Recipient {
                    customer_id: "cus_jamie".into(),
                    name: "Jamie Chen".into(),
                    given_name: Some("Jamie".into()),
                    family_name: Some("Chen".into()),
                };
                1198
            ],
            ..Reached::default()
        };
        assert_eq!(r.consent_detail("email"), "1,198 of 1,284 opted in to email; 86 left out");
        assert_eq!(r.consented[0].masked(), "J*** C***");
        assert_eq!(greeting(&r.consented[0]), "Jamie");
        assert_eq!(
            message_id("cmp_vip_weekend", "cus_jamie"),
            "msg_cmp_vip_weekend_cus_jamie"
        );
        let t = Target {
            tier: Some("vip".into()),
            ..Target::default()
        };
        assert_eq!(t.mode(), Some(AudienceMode::Tier));
        assert_eq!(Target::default().mode(), None);
    }
}
