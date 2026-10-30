//! Watcher `returns_over_refund_threshold` (`policies/platform.yml watchers`): return requests
//! waiting for a person because their amount is over the refund approval threshold
//! (`approval.yml thresholds.refund`). It reads claims, which this domain owns, and reports one
//! finding for the store; the job runner turns it into the operator's task.

use acom_contract::{EntityRef, ErrorCode, Job, JobMeta, JobOutcome, TaskDraft, ToolError};
use acom_core::{Money, StoreTime};
use acom_policy::AmountThreshold;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};

use crate::repo::db;

/// One return request waiting for review.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct Waiting {
    pub id: String,
    pub start_time: DateTime<Utc>,
    pub amount_value: Decimal,
}

#[derive(Debug)]
pub struct ReturnsOverRefundThreshold;

impl Job<Cx> for ReturnsOverRefundThreshold {
    const META: JobMeta = JobMeta {
        kind: "watcher.returns_over_refund_threshold",
        summary: "Return requests over the refund approval threshold that wait for a person.",
        watcher: Some("returns_over_refund_threshold"),
    };

    async fn run(&self, tx: &mut TxCx, _: &Value) -> Result<JobOutcome, ToolError> {
        let w = tx
            .policy
            .watchers()
            .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?
            .returns_over_refund_threshold;
        let threshold = tx
            .policy
            .amount_threshold(&w.amount_over_from)
            .ok_or_else(|| {
                ToolError::new(
                    ErrorCode::INTERNAL,
                    format!(
                        "The policy `{}` has no amount to compare with.",
                        w.amount_over_from
                    ),
                )
            })?;
        let sent_before = tx.clock.now() - Duration::hours(i64::from(w.min_waiting_hours));
        let waiting = sqlx::query_as::<_, Waiting>(
            "SELECT id, start_time, amount_value FROM fulfillment.claim
             WHERE tenant_id = $1 AND claim_type = 'return' AND claim_status = $2
               AND amount_currency = $3 AND start_time <= $4
             ORDER BY start_time, id",
        )
        .bind(tx.tenant.as_str())
        .bind(&w.claim_status)
        .bind(threshold.amount.currency.as_str())
        .bind(sent_before)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        Ok(JobOutcome::Watched(
            finding(&threshold, &waiting, w.min_count, &tx.time)
                .into_iter()
                .collect(),
        ))
    }
}

/// `$200` for whole amounts, `$200.50` otherwise; other currencies as `200 EUR`.
fn amount_words(m: &Money) -> String {
    let v = m.value.normalize();
    let text = if v.fract().is_zero() {
        v.trunc().to_string()
    } else {
        format!("{:.2}", m.value)
    };
    match m.currency.as_str() {
        "USD" => format!("${text}"),
        c => format!("{text} {c}"),
    }
}

/// What the watcher reports for the requests now waiting: `None` under `min_count`.
pub(crate) fn finding(
    threshold: &AmountThreshold,
    waiting: &[Waiting],
    min_count: u32,
    time: &StoreTime,
) -> Option<TaskDraft> {
    let over: Vec<&Waiting> = waiting
        .iter()
        .filter(|c| threshold.crossed_by(c.amount_value))
        .collect();
    let count = over.len();
    if count == 0 || count < usize::try_from(min_count).unwrap_or(usize::MAX) {
        return None;
    }
    let oldest = over.iter().map(|c| c.start_time).min()?;
    let amount = amount_words(&threshold.amount);
    let (title, request) = if count == 1 {
        (
            format!("1 return request over {amount} is waiting"),
            format!("Review the return request over {amount} and suggest what to do with it."),
        )
    } else {
        (
            format!("{count} return requests over {amount} are waiting"),
            format!(
                "Review the {count} return requests over {amount} and suggest what to do with each."
            ),
        )
    };
    let since = oldest.with_timezone(&time.zone).format("%a %b %-d %H:%M");
    Some(TaskDraft {
        cause: None,
        title,
        summary: format!("Waiting since {since}; refunds over {amount} need your approval"),
        tag: "returns".into(),
        request_text: request,
        observation: json!({
            "threshold": {"policy_key": threshold.policy_key, "value": threshold.amount},
            "observed": {"count": count, "oldest_requested_at": time.format(oldest)},
        }),
        related: over
            .iter()
            .map(|c| EntityRef::new("claim", &c.id))
            .collect(),
        fix: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_core::Currency;

    fn threshold() -> AmountThreshold {
        AmountThreshold {
            policy_key: "thresholds.refund".into(),
            amount: Money::new(Decimal::new(20000, 2), Currency::USD),
            op: "gt".into(),
        }
    }

    fn claim(id: &str, at: &str, cents: i64) -> Waiting {
        Waiting {
            id: id.into(),
            start_time: DateTime::parse_from_rfc3339(at)
                .unwrap()
                .with_timezone(&Utc),
            amount_value: Decimal::new(cents, 2),
        }
    }

    #[test]
    fn counts_only_requests_over_the_threshold() {
        let time = StoreTime::new(chrono_tz::America::New_York);
        let waiting = [
            claim("clm_a", "2026-11-05T10:00:00-05:00", 24_000),
            claim("clm_b", "2026-11-04T09:12:00-05:00", 31_900),
            claim("clm_c", "2026-11-03T08:00:00-05:00", 20_000),
            claim("clm_d", "2026-11-02T08:00:00-05:00", 9_900),
        ];
        let d = finding(&threshold(), &waiting, 1, &time).unwrap();
        assert_eq!(d.title, "2 return requests over $200 are waiting");
        assert_eq!(
            d.request_text,
            "Review the 2 return requests over $200 and suggest what to do with each."
        );
        assert_eq!(
            d.observation.to_string(),
            r#"{"threshold":{"policy_key":"thresholds.refund","value":{"value":200.00,"currency":"USD"}},"observed":{"count":2,"oldest_requested_at":"2026-11-04T09:12:00-05:00"}}"#
        );
        assert_eq!(
            d.related.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["clm_a", "clm_b"]
        );
        assert!(d.cause.is_none() && d.fix.is_none());
        assert!(finding(&threshold(), &waiting, 3, &time).is_none());
        assert!(finding(&threshold(), &waiting[2..], 1, &time).is_none());
    }

    #[test]
    fn one_request_reads_in_the_singular() {
        let time = StoreTime::new(chrono_tz::America::New_York);
        let d = finding(
            &threshold(),
            &[claim("clm_a", "2026-11-05T10:00:00-05:00", 24_050)],
            1,
            &time,
        )
        .unwrap();
        assert_eq!(d.title, "1 return request over $200 is waiting");
        assert_eq!(
            amount_words(&Money::new(Decimal::new(20050, 2), Currency::USD)),
            "$200.50"
        );
    }
}
