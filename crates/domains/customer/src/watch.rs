//! Scheduled work (`platform.job`): the regular tier evaluation (`customer.evaluate_tiers`).
//!
//! A program's schedule (`evaluation.schedule`, or `tier.evaluation.schedule`) says when its
//! members are evaluated. The approved program definition is the approval of these runs
//! (`tier.evaluation.scheduled_runs: standing_approval`), so the job applies the moves itself as
//! the scheduler, records the run (`customer.tier_evaluation`), and books the next run. When the
//! store's policy says `each_run`, the job only reports what a run would do and leaves it to a
//! person (`run_tier_evaluation`). Manual tiers that ended return to evaluation at the same time.

use acom_contract::{DomainEvent, Job, JobMeta, JobOutcome, ToolError};
use acom_core::enums::TierChangeCause;
use acom_store::{Cx, TxCx};
use serde_json::{Value, json};

use crate::access::db;
use crate::membership::{apply_moves, evaluate_members, grouped, tier_defs};
use crate::progress::{Direction, next_evaluation};
use crate::repo;

/// `customer.evaluate_tiers`: evaluate one program at its scheduled time.
#[derive(Debug)]
pub struct EvaluateTiers;

impl Job<Cx> for EvaluateTiers {
    const META: JobMeta = JobMeta {
        kind: "customer.evaluate_tiers",
        summary: "Evaluates a tier program at its scheduled time and books the next run.",
        watcher: None,
    };

    async fn run(&self, tx: &mut TxCx, payload: &Value) -> Result<JobOutcome, ToolError> {
        let program_id = payload
            .get("program_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_input("The job's payload names no program_id."))?
            .to_owned();
        let Some(program) = repo::program(tx, &program_id).await? else {
            return Ok(JobOutcome::done(
                json!({"program_id": program_id, "note": "no such program"}),
            ));
        };
        let now = tx.clock.now();
        let standing = tx
            .policy
            .lookup("customer.tier.evaluation.scheduled_runs")
            .and_then(Value::as_str)
            != Some("each_run");

        // Manual tiers that ended go back to evaluation (tier.override.after_expiry).
        let ended = end_expired_overrides(tx, &program_id).await?;

        let tiers = tier_defs(&repo::tiers(tx, Some(&program_id)).await?);
        let outcome = evaluate_members(
            tx,
            &program,
            &tiers,
            &program.evaluation,
            &program.movement,
            None,
            now,
        )
        .await?;
        let moves: Vec<_> = outcome
            .moves
            .into_iter()
            .filter(|(_, m)| {
                matches!(
                    m.direction,
                    Direction::Up | Direction::Down | Direction::Grace
                )
            })
            .collect();
        let paths: Vec<Value> = grouped(&moves)
            .into_iter()
            .map(|(f, t, d, n)| json!({"from": f, "to": t, "direction": d.word(), "members": n}))
            .collect();
        let mut result = json!({"program_id": program_id, "measured": outcome.measured,
                                "moves": paths, "manual_tiers_ended": ended});
        let mut events = Vec::new();
        if standing && !moves.is_empty() {
            let id = format!("tev_{}", acom_core::id::new_suffix());
            sqlx::query(
                "INSERT INTO customer.tier_evaluation (tenant_id, id, member_program_id, as_of,
                   scope, action_status, agent_id, start_time)
                 VALUES ($1, $2, $3, $4, '{\"scope\": \"all\"}', 'active', $5, $4)",
            )
            .bind(tx.tenant.as_str())
            .bind(&id)
            .bind(&program_id)
            .bind(now)
            .bind(&tx.principal.subject)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
            let changed = apply_moves(
                tx,
                &program_id,
                Some(&id),
                &moves,
                TierChangeCause::Evaluation,
            )
            .await?;
            sqlx::query(
                "UPDATE customer.tier_evaluation SET action_status = 'completed', summary = $3,
                   end_time = $4, updated_at = now(), version = version + 1
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tx.tenant.as_str())
            .bind(&id)
            .bind(json!({"changed": changed, "paths": paths}))
            .bind(tx.clock.now())
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
            result["evaluation_id"] = json!(id);
            events.push(DomainEvent {
                topic: "customer.tiers_evaluated".into(),
                object_id: program_id.clone(),
                payload: json!({"evaluation_id": id, "changed": changed}),
            });
        } else if !standing {
            result["note"] =
                json!("Scheduled runs need a person here (each_run): run_tier_evaluation");
        }

        // Book the next run from the program's schedule.
        let schedule = program
            .evaluation
            .get("schedule")
            .cloned()
            .or_else(|| {
                tx.policy
                    .lookup("customer.tier.evaluation.schedule")
                    .cloned()
            })
            .unwrap_or(Value::Null);
        if let Some(next) = next_evaluation(&schedule, now, tx.time) {
            book(tx, &program_id, next).await?;
            result["next_run_at"] = json!(tx.time.format(next));
        }
        Ok(JobOutcome::Done { result, events })
    }
}

/// Clears manual tiers whose end has passed: `revert_to_previous` puts the member back in the
/// tier before; `re_evaluate` leaves the tier for this evaluation to decide. History says so.
async fn end_expired_overrides(tx: &mut TxCx, program_id: &str) -> Result<usize, ToolError> {
    let now = tx.clock.now();
    let rows: Vec<(String, String, Value)> = sqlx::query_as(
        "SELECT customer_id, tier_id, tier_override FROM customer.program_membership
         WHERE tenant_id = $1 AND member_program_id = $2 AND tier_override IS NOT NULL
           AND (tier_override->>'expires_at')::timestamptz <= $3",
    )
    .bind(tx.tenant.as_str())
    .bind(program_id)
    .bind(now)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    for (customer_id, tier_id, o) in &rows {
        let back = (o.get("after_expiry").and_then(Value::as_str) == Some("revert_to_previous"))
            .then(|| o.get("previous_tier_id").and_then(Value::as_str))
            .flatten()
            .unwrap_or(tier_id);
        repo::set_membership_tier(tx, customer_id, program_id, back, now, None).await?;
        repo::add_history(
            tx,
            &repo::TierChange {
                customer_id: customer_id.clone(),
                program_id: program_id.to_owned(),
                from_tier_id: Some(tier_id.clone()),
                to_tier_id: Some(back.to_owned()),
                cause: TierChangeCause::OverrideExpired.as_str().into(),
                evaluation_id: None,
                snapshot: None,
                agent_id: tx.principal.subject.clone(),
                reason: None,
                expires: None,
            },
        )
        .await?;
    }
    Ok(rows.len())
}

/// Books the program's next evaluation (one waiting job per program and time).
async fn book(
    tx: &mut TxCx,
    program_id: &str,
    at: chrono::DateTime<chrono::Utc>,
) -> Result<(), ToolError> {
    let day = at.with_timezone(&tx.time.zone).format("%Y%m%d");
    let id = format!(
        "job_evaluate_{}_{day}",
        program_id.trim_start_matches("prg_")
    );
    sqlx::query(
        "INSERT INTO platform.job (tenant_id, id, kind, run_at, payload)
         VALUES ($1, $2, 'customer.evaluate_tiers', $3, $4) ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&id)
    .bind(at)
    .bind(json!({"program_id": program_id}))
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_is_domain_dot_verb() {
        assert_eq!(EvaluateTiers::META.kind, "customer.evaluate_tiers");
        assert!(EvaluateTiers::META.watcher.is_none());
    }

    /// Integration: a scheduled run with no order reader moves nobody and books the next run.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn a_scheduled_run_books_the_next_one() {
        let mut tx = crate::testdb::tx("sys_scheduler").await;
        let out = EvaluateTiers
            .run(&mut tx, &json!({"program_id": "prg_rewards"}))
            .await
            .unwrap();
        let JobOutcome::Done { result, .. } = out else {
            panic!("a scheduled run is done, not watched");
        };
        assert_eq!(result["next_run_at"], "2026-12-01T03:00:00-05:00");
        let booked: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.job WHERE id = 'job_evaluate_rewards_20261201'",
        )
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        assert_eq!(booked, 1);
        // Rolled back on drop.
    }
}
