//! Status changes booked for later (`set_seller_status` with a future `valid_from`). The run
//! writes the change as a `potential` event of the seller and books one job at its time; the job
//! applies it then, unless it was undone or the seller moved on in between.

use acom_contract::{Job, JobMeta, JobOutcome, ToolError};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::repo;
use crate::seller::set_seller_status::apply_transition;

/// The job that runs one booked status change.
pub struct ChangeSellerStatus;

/// Books the job for the event at `position` of a seller (fixed id, so booking twice is one job).
pub(crate) async fn book(
    tx: &mut TxCx,
    seller_id: &str,
    position: i32,
    at: DateTime<Utc>,
) -> Result<(), ToolError> {
    let id = format!(
        "job_seller_status_{}_{position}",
        seller_id.trim_start_matches("slr_")
    );
    acom_changeset::jobs::book(
        tx,
        &id,
        ChangeSellerStatus::META.kind,
        at,
        &json!({"seller_id": seller_id, "position": position}),
    )
    .await?;
    Ok(())
}

impl Job<Cx> for ChangeSellerStatus {
    const META: JobMeta = JobMeta {
        kind: "partner.change_seller_status",
        summary: "Applies a seller status change booked for this time (set_seller_status with a later valid_from), unless it was undone or the seller's status changed since.",
        watcher: None,
    };

    async fn run(&self, tx: &mut TxCx, payload: &Value) -> Result<JobOutcome, ToolError> {
        let seller_id = payload["seller_id"]
            .as_str()
            .ok_or_else(|| ToolError::invalid_input("The job's payload names no seller_id."))?
            .to_owned();
        let position = payload["position"]
            .as_i64()
            .and_then(|p| i32::try_from(p).ok())
            .ok_or_else(|| ToolError::invalid_input("The job's payload names no position."))?;
        let Some(e) = repo::event(tx, &seller_id, position).await? else {
            return Ok(JobOutcome::done(
                json!({"seller_id": seller_id, "note": "no such booked change"}),
            ));
        };
        if e.action_status != "potential" {
            return Ok(JobOutcome::done(json!({"seller_id": seller_id,
                "note": format!("the booked change is {} already", e.action_status)})));
        }
        let (Some(from), Some(to)) = (e.from_state.clone(), e.to_state.clone()) else {
            return Ok(JobOutcome::done(
                json!({"seller_id": seller_id, "note": "the booked change names no states"}),
            ));
        };
        let now = tx.clock.now();
        let current = repo::seller(tx, &seller_id).await?.map(|s| s.seller_status);
        if current.as_deref() != Some(from.as_str()) {
            let message = format!(
                "The seller is {} now, not {from}, so the booked change to {to} did not run.",
                current.as_deref().unwrap_or("gone")
            );
            repo::finish_event(
                tx,
                &seller_id,
                position,
                "failed",
                None,
                Some(&json!({"code": "invalid_state", "class": "invalid_state",
                              "message": message})),
                now,
            )
            .await?;
            return Ok(JobOutcome::done(
                json!({"seller_id": seller_id, "note": message}),
            ));
        }
        let applied = apply_transition(tx, &seller_id, &from, &to).await?;
        let mut result = e.result.unwrap_or_else(|| json!({}));
        result["products_hidden"] = json!(applied.products_hidden);
        result["products_restored"] = json!(applied.products_restored);
        repo::finish_event(
            tx,
            &seller_id,
            position,
            "completed",
            Some(&result),
            None,
            now,
        )
        .await?;
        Ok(JobOutcome::done(json!({
            "seller_id": seller_id, "from": from, "to": to,
            "products_hidden": applied.products_hidden,
            "products_restored": applied.products_restored,
        })))
    }
}
