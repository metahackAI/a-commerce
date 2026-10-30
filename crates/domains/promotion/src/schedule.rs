//! Scheduled starts and ends (`promotion.start`, `promotion.end` in `platform.job`).
//!
//! Reads already compute a promotion's status from its period and the store clock
//! (`repo::status_sql`), so these jobs change no behaviour: at the start or end time they write
//! the status every read already shows into `promotion_status` and report the change as a domain
//! event (`promotion.started`, `promotion.ended`). A paused, draft or ended promotion keeps its
//! status; a promotion that no longer exists is reported and left alone.

use acom_contract::{DomainEvent, Job, JobMeta, JobOutcome, ToolError};
use acom_store::{Cx, TxCx};
use serde_json::{Value, json};

use crate::repo::{db, status_sql};

/// `promotion.start`: the promotion's period began.
#[derive(Debug)]
pub struct PromotionStarts;

/// `promotion.end`: the promotion's period is over.
#[derive(Debug)]
pub struct PromotionEnds;

impl Job<Cx> for PromotionStarts {
    const META: JobMeta = JobMeta {
        kind: "promotion.start",
        summary: "Records that a promotion's period began (its status reads running).",
        watcher: None,
    };
    async fn run(&self, tx: &mut TxCx, payload: &Value) -> Result<JobOutcome, ToolError> {
        persist_status(tx, payload).await
    }
}

impl Job<Cx> for PromotionEnds {
    const META: JobMeta = JobMeta {
        kind: "promotion.end",
        summary: "Records that a promotion's period is over (its status reads ended).",
        watcher: None,
    };
    async fn run(&self, tx: &mut TxCx, payload: &Value) -> Result<JobOutcome, ToolError> {
        persist_status(tx, payload).await
    }
}

/// The event a status change is reported as; `None` for changes nobody waits for.
pub(crate) fn topic(status: &str) -> Option<&'static str> {
    match status {
        "running" => Some("promotion.started"),
        "ended" => Some("promotion.ended"),
        _ => None,
    }
}

async fn persist_status(tx: &mut TxCx, payload: &Value) -> Result<JobOutcome, ToolError> {
    let id = payload
        .get("promotion_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::invalid_input("The job's payload names no promotion_id."))?
        .to_owned();
    let now = tx.clock.now();
    let row: Option<(String, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT promotion_status, {} FROM promotion.promotion
         WHERE tenant_id = $1 AND id = $3 FOR UPDATE",
        status_sql("$2")
    )))
    .bind(tx.tenant.as_str())
    .bind(now)
    .bind(&id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let Some((stored, current)) = row else {
        return Ok(JobOutcome::done(
            json!({"promotion_id": id, "status": null, "note": "no such promotion"}),
        ));
    };
    let changed = stored != current;
    let mut events = Vec::new();
    if changed {
        sqlx::query(
            "UPDATE promotion.promotion SET promotion_status = $3, updated_at = now(),
               version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&id)
        .bind(&current)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        if let Some(t) = topic(&current) {
            events.push(DomainEvent {
                topic: t.to_owned(),
                object_id: id.clone(),
                payload: json!({"promotion_id": id, "status": current, "at": tx.time.format(now)}),
            });
        }
    }
    Ok(JobOutcome::Done {
        result: json!({"promotion_id": id, "status": current, "changed": changed}),
        events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_and_ends_are_reported_as_events() {
        assert_eq!(topic("running"), Some("promotion.started"));
        assert_eq!(topic("ended"), Some("promotion.ended"));
        assert_eq!(topic("paused"), None);
        assert_eq!(topic("scheduled"), None);
        assert_eq!(PromotionStarts::META.kind, "promotion.start");
        assert_eq!(PromotionEnds::META.kind, "promotion.end");
    }
}
