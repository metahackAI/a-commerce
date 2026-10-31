//! The scheduled send (`messaging.send_campaign` in `platform.job`): a campaign that
//! `create_campaign` booked for a time goes out then, without anyone asking again. A person
//! approved the campaign with its time, so the send needs no second approval; it still applies
//! the consent gate and the daily cap at the moment it runs. A campaign that was cancelled,
//! sent already, or can no longer be sent is left as it is (or marked failed with the reason), so
//! running the job again changes nothing.

use acom_contract::{DomainEvent, Job, JobMeta, JobOutcome, ToolError};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::campaign::{Target, reach, send_now, under_cap};
use crate::message::ready;
use crate::repo;

/// The job kind of a campaign's scheduled send.
pub const SEND_CAMPAIGN: &str = "messaging.send_campaign";

/// `messaging.send_campaign`: sends one scheduled campaign at its time.
#[derive(Debug)]
pub struct SendScheduledCampaign;

/// Books the campaign's send at `at` (booking the same campaign again does nothing).
pub(crate) async fn book_send(
    tx: &mut TxCx,
    campaign_id: &str,
    at: DateTime<Utc>,
) -> Result<(), ToolError> {
    acom_changeset::jobs::book(
        tx,
        &format!("job_send_{campaign_id}"),
        SEND_CAMPAIGN,
        at,
        &json!({"campaign_id": campaign_id}),
    )
    .await?;
    Ok(())
}

impl Job<Cx> for SendScheduledCampaign {
    const META: JobMeta = JobMeta {
        kind: SEND_CAMPAIGN,
        summary: "Sends a scheduled campaign at its time to everyone who opted in and is under the daily cap.",
        watcher: None,
    };

    async fn run(&self, tx: &mut TxCx, payload: &Value) -> Result<JobOutcome, ToolError> {
        let id = payload
            .get("campaign_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_input("The job's payload names no campaign_id."))?
            .to_owned();
        let Some(c) = repo::campaign_for_send(tx, &id).await? else {
            return Ok(JobOutcome::done(
                json!({"campaign_id": id, "sent": 0, "note": "no such campaign"}),
            ));
        };
        if c.campaign_status != "scheduled" {
            return Ok(JobOutcome::done(json!({"campaign_id": id, "sent": 0,
                "status": c.campaign_status, "note": "not scheduled any more"})));
        }
        let failed = |code: &str, message: String| json!({"code": code, "message": message});
        let (template, values) = match ready(tx, &c).await {
            Ok(r) => r,
            Err(e) => {
                let error = failed(&e.code, e.message.clone());
                repo::set_campaign_status(tx, &id, "failed", None, None, Some(&error)).await?;
                return Ok(outcome(&id, "failed", 0, Some(error)));
            }
        };
        let reached = reach(tx, &Target::of(&c), &c.message_channel).await?;
        let now = tx.clock.now();
        let (recipients, capped) = under_cap(tx, reached.consented, now).await?;
        if recipients.is_empty() {
            let error = failed(
                "no_consented_recipients",
                format!(
                    "Nobody it reaches could get it: {} opted in, {capped} at the daily cap",
                    reached.addressed - reached.not_consented - reached.unreachable
                ),
            );
            repo::set_campaign_status(tx, &id, "failed", None, Some(0), Some(&error)).await?;
            return Ok(outcome(&id, "failed", 0, Some(error)));
        }
        let ids = send_now(tx, &c, &template, &values, &recipients).await?;
        Ok(outcome(&id, "sent", ids.len(), None))
    }
}

fn outcome(id: &str, status: &str, sent: usize, error: Option<Value>) -> JobOutcome {
    JobOutcome::Done {
        result: json!({"campaign_id": id, "status": status, "sent": sent, "error": error}),
        events: vec![DomainEvent {
            topic: format!("campaign.{status}"),
            object_id: id.to_owned(),
            payload: json!({"campaign_id": id, "sent": sent}),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scheduled_send_is_one_job_per_campaign() {
        assert_eq!(SendScheduledCampaign::META.kind, "messaging.send_campaign");
        let JobOutcome::Done { result, events } = outcome("cmp_x", "sent", 3, None) else {
            panic!("done");
        };
        assert_eq!(result["sent"], 3);
        assert_eq!(events[0].topic, "campaign.sent");
    }
}
