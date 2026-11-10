//! What the gateway's workers tell commerce while Ailism Customer Bot runs (internal endpoints,
//! system only; docs/api/platform.md#customer_bot): the next run to work on, its sessions, every
//! step a session takes, the persona it made, and how the session ended. A run ends when its last
//! session does.

use acom_contract::{ErrorCode, ToolError};
use acom_core::enums::{CustomerBotStepKind, CustomerBotStepStatus};
use acom_policy::CustomerBotMode;
use acom_store::TxCx;
use serde_json::{Map, Value, json};

use super::{
    CUSTOMER_BOT_SESSION_NOT_FOUND, STALE_RUN_MINUTES, db, not_found, run_event, settings_of,
};
use crate::bot::fresh_policy;

/// The longest text one stored request, response or detail keeps; larger values are cut.
const MOST_JSON: usize = 24_000;
/// The most steps one call records.
const MOST_STEPS: usize = 50;

fn invalid(field: &str, message: impl Into<String>) -> ToolError {
    ToolError::new(ErrorCode::INVALID_INPUT, message.into())
        .detail(json!({"field": field, "reason": "invalid"}))
}

/// A run the workers should start now, and what its sessions need to know.
#[derive(Debug, Clone)]
pub struct NextRun {
    pub id: String,
    pub trigger: String,
    pub workers: u32,
    pub new_account_percent: u32,
    pub never_shop_as: Vec<String>,
    pub settings: Value,
}

/// Closes runs whose workers went away, then takes the oldest queued run, or opens a scheduled
/// one when its time has come by the server's clock. Only one run works at a time.
pub async fn claim_next_run(tx: &mut TxCx) -> Result<Option<NextRun>, ToolError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('customer_bot_run:' || $1))")
        .bind(tx.tenant.as_str())
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    let now = tx.clock.now();
    let stale: Vec<String> = sqlx::query_scalar(
        "UPDATE platform.customer_bot_run SET run_state = 'failed', end_time = $2,
           finished_at = now(), error = 'The workers stopped before the run ended.',
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND run_state = 'running'
           AND greatest(claimed_at,
                 (SELECT max(t.created_at) FROM platform.customer_bot_step t
                    JOIN platform.customer_bot_session s
                      ON s.tenant_id = t.tenant_id AND s.id = t.session_id
                   WHERE s.tenant_id = customer_bot_run.tenant_id AND s.run_id = customer_bot_run.id))
               < now() - make_interval(mins => $3)
         RETURNING id",
    )
    .bind(tx.tenant.as_str())
    .bind(now)
    .bind(STALE_RUN_MINUTES as i32)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    for id in &stale {
        sqlx::query(
            "UPDATE platform.customer_bot_session SET session_state = 'failed', end_time = $3,
               finished_at = now(), error = coalesce(error, 'The worker stopped.'),
               updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND run_id = $2 AND session_state = 'running'",
        )
        .bind(tx.tenant.as_str())
        .bind(id)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        run_event(tx, id, "failed").await?;
    }
    let running: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM platform.customer_bot_run WHERE tenant_id = $1 AND run_state = 'running'",
    )
    .bind(tx.tenant.as_str())
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    if running > 0 {
        return Ok(None);
    }
    let policy = fresh_policy(tx).await?;
    let settings = settings_of(&policy)?;
    let queued: Option<(String, i32)> = sqlx::query_as(
        "SELECT id, workers FROM platform.customer_bot_run
         WHERE tenant_id = $1 AND run_state = 'queued' ORDER BY created_at, id LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let (id, trigger, workers) = if let Some((id, workers)) = queued {
        (id, "manual", workers)
    } else {
        if settings.mode == CustomerBotMode::Off {
            return Ok(None);
        }
        let due: bool = sqlx::query_scalar(
            "SELECT coalesce(max(created_at) <= now() - make_interval(mins => $2), true)
             FROM platform.customer_bot_run WHERE tenant_id = $1",
        )
        .bind(tx.tenant.as_str())
        .bind(settings.every_minutes as i32)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        if !due {
            return Ok(None);
        }
        let id = format!("cbr_{}", acom_core::id::new_suffix());
        let workers = settings.workers.clamp(1, 20) as i32;
        sqlx::query(
            "INSERT INTO platform.customer_bot_run (tenant_id, id, bot_trigger, run_state, workers,
               start_time)
             VALUES ($1, $2, 'schedule', 'queued', $3, $4)",
        )
        .bind(tx.tenant.as_str())
        .bind(&id)
        .bind(workers)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        (id, "schedule", workers)
    };
    sqlx::query(
        "UPDATE platform.customer_bot_run SET run_state = 'running', claimed_at = now(),
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(&id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    run_event(tx, &id, "running").await?;
    Ok(Some(NextRun {
        id,
        trigger: trigger.to_owned(),
        workers: u32::try_from(workers).unwrap_or(1),
        new_account_percent: settings.new_account_percent.min(100),
        never_shop_as: settings.never_shop_as.clone(),
        settings: super::settings_json(&settings),
    }))
}

/// Members in a session of a run still running: two workers never shop as the same member.
pub async fn members_shopping_now(tx: &mut TxCx) -> Result<Vec<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT customer_id FROM platform.customer_bot_session
         WHERE tenant_id = $1 AND session_state = 'running' AND customer_id IS NOT NULL",
    )
    .bind(tx.tenant.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Opens a session for each account of the run: an existing member's id, or `None` for one that
/// signs up first. Each session comes with the member's persona when there is one.
pub async fn open_sessions(
    tx: &mut TxCx,
    run_id: &str,
    accounts: &[Option<String>],
) -> Result<Vec<Value>, ToolError> {
    let now = tx.clock.now();
    let mut out = Vec::with_capacity(accounts.len());
    for account in accounts {
        let id = format!("cbs_{}", acom_core::id::new_suffix());
        let origin = if account.is_some() {
            "existing"
        } else {
            "created"
        };
        sqlx::query(
            "INSERT INTO platform.customer_bot_session (tenant_id, id, run_id, customer_id,
               account_origin, session_state, start_time)
             VALUES ($1, $2, $3, $4, $5, 'running', $6)",
        )
        .bind(tx.tenant.as_str())
        .bind(&id)
        .bind(run_id)
        .bind(account.as_deref())
        .bind(origin)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        let persona: Option<(String, Value)> = match account {
            Some(c) => sqlx::query_as(
                "SELECT description, persona FROM platform.shopper_persona
                 WHERE tenant_id = $1 AND customer_id = $2",
            )
            .bind(tx.tenant.as_str())
            .bind(c)
            .fetch_optional(&mut *tx.tx)
            .await
            .map_err(db)?,
            None => None,
        };
        out.push(json!({"id": id, "customer_id": account, "account": origin,
                        "persona": persona.map(|(d, p)| json!({"description": d, "persona": p}))}));
    }
    if accounts.is_empty() {
        finish_run_if_done(tx, run_id).await?;
    }
    Ok(out)
}

/// A JSON value kept whole when small, else cut to its first characters.
fn bounded(v: Value) -> Value {
    let text = v.to_string();
    if text.len() <= MOST_JSON {
        return v;
    }
    let cut: String = text.chars().take(MOST_JSON / 4).collect();
    json!({"truncated": true, "characters": text.len(), "start": cut})
}

fn object_or_none(v: Option<&Value>) -> Option<Value> {
    v.filter(|v| v.is_object() || v.is_array())
        .cloned()
        .map(bounded)
}

/// A running session's id, or why there is none.
async fn running_session(tx: &mut TxCx, id: &str) -> Result<(String, String), ToolError> {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT run_id, session_state FROM platform.customer_bot_session
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let Some((run, state)) = row else {
        return Err(not_found(
            CUSTOMER_BOT_SESSION_NOT_FOUND,
            "customer_bot_session",
            id,
            "list_customer_bot_sessions",
        ));
    };
    if state != "running" {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("Session {id} has ended; nothing more is recorded for it."),
        )
        .detail(json!({"entity": "customer_bot_session", "id": id, "from": state})));
    }
    Ok((run, state))
}

/// Records steps of a running session, in the order given, after the ones it has. The body may
/// also give the account a new session signed up (`customer_id`), a conversation it had
/// (`conversation_id`) and what it came for (`goal`).
pub async fn append(tx: &mut TxCx, session_id: &str, body: &Value) -> Result<Value, ToolError> {
    running_session(tx, session_id).await?;
    let steps = body
        .get("steps")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if steps.len() > MOST_STEPS {
        return Err(invalid(
            "steps",
            format!("Record up to {MOST_STEPS} steps at a time."),
        ));
    }
    if let Some(c) = body.get("customer_id").and_then(Value::as_str) {
        let ok = c.starts_with("cus_")
            && c[4..]
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_');
        if !ok {
            return Err(invalid("customer_id", "A member id looks like cus_…."));
        }
        sqlx::query(
            "UPDATE platform.customer_bot_session SET customer_id = $3, updated_at = now(),
               version = version + 1
             WHERE tenant_id = $1 AND id = $2 AND customer_id IS NULL",
        )
        .bind(tx.tenant.as_str())
        .bind(session_id)
        .bind(c)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    if let Some(c) = body.get("conversation_id").and_then(Value::as_str) {
        sqlx::query(
            "UPDATE platform.customer_bot_session
               SET conversation_ids = conversation_ids || to_jsonb($3::text), updated_at = now(),
                   version = version + 1
             WHERE tenant_id = $1 AND id = $2 AND NOT conversation_ids ? $3",
        )
        .bind(tx.tenant.as_str())
        .bind(session_id)
        .bind(c)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    if let Some(g) = body.get("goal").and_then(Value::as_str) {
        let g: String = g.trim().chars().take(500).collect();
        if !g.is_empty() {
            sqlx::query(
                "UPDATE platform.customer_bot_session SET goal = $3, updated_at = now(),
                   version = version + 1
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tx.tenant.as_str())
            .bind(session_id)
            .bind(&g)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        }
    }
    let mut position: i32 = sqlx::query_scalar(
        "SELECT coalesce(max(position), 0) FROM platform.customer_bot_step
         WHERE tenant_id = $1 AND session_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(session_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let now = tx.clock.now();
    for (i, s) in steps.iter().enumerate() {
        let kind = s.get("kind").and_then(Value::as_str).unwrap_or_default();
        if kind.parse::<CustomerBotStepKind>().is_err() {
            return Err(invalid(
                &format!("steps[{i}].kind"),
                format!("{kind:?} is not a step kind."),
            ));
        }
        let status = s.get("status").and_then(Value::as_str).unwrap_or("ok");
        if status.parse::<CustomerBotStepStatus>().is_err() {
            return Err(invalid(
                &format!("steps[{i}].status"),
                format!("{status:?} is not a step status."),
            ));
        }
        let title: String = s
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .chars()
            .take(300)
            .collect();
        if title.is_empty() {
            return Err(invalid(
                &format!("steps[{i}].title"),
                "Every step says what it did.",
            ));
        }
        let detail = s
            .get("detail")
            .filter(|d| d.is_object())
            .cloned()
            .map_or_else(|| json!({}), bounded);
        let ms = s
            .get("ms")
            .and_then(Value::as_u64)
            .map_or(0, |m| i32::try_from(m).unwrap_or(i32::MAX));
        position += 1;
        sqlx::query(
            "INSERT INTO platform.customer_bot_step (tenant_id, session_id, position, step_kind,
               step_status, title, request, response, detail, duration_ms, start_time)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(tx.tenant.as_str())
        .bind(session_id)
        .bind(position)
        .bind(kind)
        .bind(status)
        .bind(&title)
        .bind(object_or_none(s.get("request")).filter(Value::is_object))
        .bind(object_or_none(s.get("response")))
        .bind(&detail)
        .bind(ms)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    Ok(json!({"ok": true, "session_id": session_id, "steps": position}))
}

/// Keeps the persona a session made for a member. A member has one persona: a second save
/// replaces its description and contents and keeps who made it first.
pub async fn save_persona(
    tx: &mut TxCx,
    customer_id: &str,
    body: &Value,
) -> Result<Value, ToolError> {
    let description: String = body
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .chars()
        .take(500)
        .collect();
    let Some(persona) = body.get("persona").filter(|p| p.is_object()).cloned() else {
        return Err(invalid("persona", "A persona is an object."));
    };
    if description.is_empty() {
        return Err(invalid(
            "description",
            "A persona has a one-line description.",
        ));
    }
    let session = body.get("session_id").and_then(Value::as_str);
    if let Some(s) = session {
        running_session(tx, s).await?;
    }
    let id = format!("spn_{}", acom_core::id::new_suffix());
    let saved: (String, bool) = sqlx::query_as(
        "INSERT INTO platform.shopper_persona (tenant_id, id, customer_id, description, persona,
           built_by_session_id)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (tenant_id, customer_id) DO UPDATE
           SET description = EXCLUDED.description, persona = EXCLUDED.persona,
               updated_at = now(), version = platform.shopper_persona.version + 1
         RETURNING id, (xmax = 0)",
    )
    .bind(tx.tenant.as_str())
    .bind(&id)
    .bind(customer_id)
    .bind(&description)
    .bind(bounded(persona))
    .bind(session)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    if let Some(s) = session.filter(|_| saved.1) {
        sqlx::query(
            "UPDATE platform.customer_bot_session SET persona_built = true, updated_at = now(),
               version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(s)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    Ok(json!({"ok": true, "id": saved.0, "created": saved.1}))
}

/// What a session's outcome adds to its run's summary.
const COUNTS: &[&str] = &["chat_turns", "orders", "reviews", "memories", "cart_adds"];

/// Ends a session as `done` or `failed` with what it did; the run ends with its last session.
pub async fn finish_session(
    tx: &mut TxCx,
    session_id: &str,
    body: &Value,
) -> Result<Value, ToolError> {
    let (run_id, _) = running_session(tx, session_id).await?;
    let state = body.get("state").and_then(Value::as_str).unwrap_or("done");
    if state != "done" && state != "failed" {
        return Err(invalid("state", "A session ends done or failed."));
    }
    let outcome = body
        .get("outcome")
        .filter(|o| o.is_object())
        .cloned()
        .map_or_else(|| json!({}), bounded);
    let error: Option<String> = body
        .get("error")
        .and_then(Value::as_str)
        .map(|e| e.chars().take(1000).collect());
    let now = tx.clock.now();
    let customer: Option<String> = sqlx::query_scalar(
        "UPDATE platform.customer_bot_session SET session_state = $3, outcome = $4, error = $5,
           end_time = $6, finished_at = now(), updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2
         RETURNING customer_id",
    )
    .bind(tx.tenant.as_str())
    .bind(session_id)
    .bind(state)
    .bind(&outcome)
    .bind(error.as_deref())
    .bind(now)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    if let Some(c) = &customer {
        sqlx::query(
            "UPDATE platform.shopper_persona SET session_count = session_count + 1,
               last_session_at = now(), updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND customer_id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(c)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    let run_state = finish_run_if_done(tx, &run_id).await?;
    Ok(json!({"ok": true, "session_id": session_id, "state": state,
              "run_id": run_id, "run_state": run_state}))
}

/// Ends a running run whose sessions have all ended, with what they did together.
async fn finish_run_if_done(tx: &mut TxCx, run_id: &str) -> Result<String, ToolError> {
    let rows: Vec<(String, String, bool, Value)> = sqlx::query_as(
        "SELECT session_state, account_origin, persona_built, outcome
         FROM platform.customer_bot_session WHERE tenant_id = $1 AND run_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(run_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    if rows.iter().any(|(s, ..)| s == "running") {
        return Ok("running".to_owned());
    }
    let mut summary = Map::new();
    let done = rows.iter().filter(|(s, ..)| s == "done").count();
    summary.insert("sessions".into(), json!(rows.len()));
    summary.insert("done".into(), json!(done));
    summary.insert("failed".into(), json!(rows.len() - done));
    summary.insert(
        "accounts_created".into(),
        json!(rows.iter().filter(|(_, a, ..)| a == "created").count()),
    );
    summary.insert(
        "personas_built".into(),
        json!(rows.iter().filter(|(_, _, p, _)| *p).count()),
    );
    for key in COUNTS {
        let n: u64 = rows
            .iter()
            .filter_map(|(.., o)| o.get(*key).and_then(Value::as_u64))
            .sum();
        summary.insert((*key).into(), json!(n));
    }
    // Each session's spending as the decimal it was written as, so the total is exact.
    let spent: rust_decimal::Decimal = rows
        .iter()
        .filter_map(|(.., o)| o.get("spent").and_then(|s| s.get("value")))
        .filter_map(|v| match v {
            Value::String(s) => s.parse::<rust_decimal::Decimal>().ok(),
            other => other.to_string().parse::<rust_decimal::Decimal>().ok(),
        })
        .sum();
    summary.insert(
        "spent".into(),
        serde_json::to_value(acom_core::Money::new(spent, tx.currency)).unwrap_or(Value::Null),
    );
    let state = if !rows.is_empty() && done == 0 {
        "failed"
    } else {
        "done"
    };
    let now = tx.clock.now();
    let updated = sqlx::query(
        "UPDATE platform.customer_bot_run SET run_state = $3, summary = $4, end_time = $5,
           finished_at = now(),
           error = CASE WHEN $3 = 'failed' THEN coalesce(error, 'Every session failed.') END,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND run_state = 'running'",
    )
    .bind(tx.tenant.as_str())
    .bind(run_id)
    .bind(state)
    .bind(Value::Object(summary))
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected();
    if updated > 0 {
        run_event(tx, run_id, state).await?;
    }
    Ok(state.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_value_is_cut_and_says_so() {
        let big = json!({"text": "x".repeat(MOST_JSON * 2)});
        let cut = bounded(big);
        assert_eq!(cut["truncated"], true);
        assert!(cut["start"].as_str().unwrap().len() <= MOST_JSON / 4);
        let small = json!({"a": 1});
        assert_eq!(bounded(small.clone()), small);
    }
}
