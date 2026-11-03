//! The end of a cycle (`POST /v1/platform/bot/cycles/{bot_cycle_id}/report`, internal, system
//! only; docs/api/platform.md#bot steps 4–5): the agent's snapshot and analysis come back, the
//! proposals are filtered by the store's settings, and each one left is opened as a decision and
//! a task for the operations agent to draft.

use acom_contract::{ActionType, ErrorCode, ToolError};
use acom_core::enums::BotArea;
use acom_policy::BotMode;
use acom_store::TxCx;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use super::{
    BOT_CYCLE_NOT_FOUND, cause_key, cycle_event, db, decisions_sql, fresh_policy, not_found,
    settings_of,
};

/// States in which a decision still waits for something.
const OPEN_STATES: &[&str] = &["drafting", "needs_you", "waiting", "running"];

fn text(v: &Value, key: &str, most: usize) -> String {
    let mut s: String = v
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .chars()
        .take(most)
        .collect();
    s.truncate(s.trim_end().len());
    s
}

fn word(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The entities a proposal names, kept only when they look like store ids.
fn about(v: &Value) -> Vec<(String, String)> {
    v.get("about")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let e = a.get("entity")?.as_str()?.trim();
            let id = a.get("id")?.as_str()?.trim();
            (word(e) && id.contains('_') && word(id)).then(|| (e.to_owned(), id.to_owned()))
        })
        .take(10)
        .collect()
}

/// The proposal as the decision stores it.
pub(crate) struct Proposal {
    pub area: String,
    pub title: String,
    pub request: String,
    pub reason: String,
    pub evidence: Value,
    pub expected_effect: Value,
    pub risk: String,
    pub about: Vec<(String, String)>,
}

fn proposal(v: &Value) -> Result<Proposal, &'static str> {
    let title = text(v, "title", 200);
    let request = text(v, "request", 1000);
    if title.is_empty() || request.is_empty() {
        return Err("It had no title or request.");
    }
    let area = text(v, "area", 40);
    if area.parse::<BotArea>().is_err() {
        return Err("It named no known area.");
    }
    let reason = Some(text(v, "reason", 1000))
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| title.clone());
    let evidence: Vec<String> = v
        .get("evidence")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| {
            e.as_str()
                .map(|s| s.trim().chars().take(300).collect::<String>())
        })
        .filter(|s| !s.is_empty())
        .take(8)
        .collect();
    let effect = v.get("expected_effect").cloned().unwrap_or(Value::Null);
    let expected_effect = if effect.is_object() {
        effect
    } else {
        json!({})
    };
    let risk = text(v, "risk", 10);
    let risk = if matches!(risk.as_str(), "low" | "medium" | "high") {
        risk
    } else {
        "medium".to_owned()
    };
    Ok(Proposal {
        area,
        title,
        request,
        reason,
        evidence: json!(evidence),
        expected_effect,
        risk,
        about: about(v),
    })
}

/// Records the cycle's end and opens what passes. Answers what was opened and what was not.
pub async fn record(tx: &mut TxCx, cycle_id: &str, body: &Value) -> Result<Value, ToolError> {
    let state: Option<String> = sqlx::query_scalar(
        "SELECT cycle_state FROM platform.bot_cycle WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tx.tenant.as_str())
    .bind(cycle_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let state = state.ok_or_else(|| {
        not_found(
            BOT_CYCLE_NOT_FOUND,
            "bot_cycle",
            cycle_id,
            "list_bot_cycles",
        )
    })?;
    if state != "observing" {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("Cycle {cycle_id} already ended ({state})."),
        )
        .detail(json!({"entity": "bot_cycle", "from": state})));
    }
    let now = tx.clock.now();
    let obj = |k: &str| {
        body.get(k)
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or(json!({}))
    };
    let snapshot = obj("snapshot");
    let usage = obj("usage");
    let anomalies = body
        .get("anomalies")
        .filter(|v| v.is_array())
        .cloned()
        .unwrap_or(json!([]));

    if body.get("state").and_then(Value::as_str) == Some("failed") {
        let error = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("The analysis failed.")
            .chars()
            .take(1000)
            .collect::<String>();
        sqlx::query(
            "UPDATE platform.bot_cycle SET cycle_state = 'failed', end_time = $3, snapshot = $4,
               anomalies = $5, usage = $6, error = $7, updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(cycle_id)
        .bind(now)
        .bind(&snapshot)
        .bind(&anomalies)
        .bind(&usage)
        .bind(&error)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        cycle_event(tx, cycle_id, "failed", None).await?;
        return Ok(json!({"cycle_id": cycle_id, "state": "failed", "opened": [], "skipped": []}));
    }

    let policy = fresh_policy(tx).await?;
    let (bot, watch) = settings_of(&policy)?;
    let mut report = obj("report");
    let mut proposals: Vec<Value> = report
        .get("proposals")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let open_now: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {} WHERE d.tenant_id = $1 AND d.state = ANY($2)",
        decisions_sql()
    )))
    .bind(tx.tenant.as_str())
    .bind(OPEN_STATES)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let since = now - Duration::days(i64::from(bot.cooldown_days));
    let (mut opened, mut skipped) = (Vec::new(), Vec::new());
    let mut seen_causes: Vec<String> = Vec::new();
    for (n, p) in proposals.iter_mut().enumerate() {
        let title = text(p, "title", 200);
        let why: Option<String> = match proposal(p) {
            Err(why) => Some(why.to_owned()),
            Ok(pr) => {
                let key = cause_key(&pr.area, &pr.about, &pr.title);
                let earlier: Vec<(String, String, DateTime<Utc>)> =
                    sqlx::query_as(sqlx::AssertSqlSafe(format!(
                        "SELECT d.id, d.state, d.updated_at FROM {} WHERE d.tenant_id = $1
                         AND d.cause_key = $2 ORDER BY d.updated_at DESC",
                        decisions_sql()
                    )))
                    .bind(tx.tenant.as_str())
                    .bind(&key)
                    .fetch_all(&mut *tx.tx)
                    .await
                    .map_err(db)?;
                let still_open = earlier
                    .iter()
                    .find(|(_, s, _)| OPEN_STATES.contains(&s.as_str()));
                let turned_down = earlier.iter().find(|(_, s, at)| {
                    matches!(s.as_str(), "rejected" | "dismissed") && *at >= since
                });
                if !bot.areas.is_on(&pr.area) {
                    Some(format!("The {} area is off.", pr.area.replace('_', " ")))
                } else if bot.mode != BotMode::Propose {
                    Some("Observe mode: reported only.".to_owned())
                } else if seen_causes.contains(&key) {
                    Some("This cycle already proposed the same thing.".to_owned())
                } else if let Some((id, _, _)) = still_open {
                    Some(format!(
                        "A decision about the same thing is still open ({id})."
                    ))
                } else if let Some((id, s, at)) = turned_down {
                    let until = *at + Duration::days(i64::from(bot.cooldown_days));
                    Some(format!(
                        "{id} about the same thing was {s} on {}; not proposed again until {}.",
                        tx.time.format(*at),
                        tx.time.format(until)
                    ))
                } else if opened.len() as u32 >= bot.max_proposals_per_cycle {
                    Some(format!(
                        "This cycle already opened {} (the most per cycle).",
                        bot.max_proposals_per_cycle
                    ))
                } else if open_now + opened.len() as i64 >= i64::from(bot.max_open_decisions) {
                    Some(format!(
                        "{} decisions already wait for you (the most open at once).",
                        bot.max_open_decisions
                    ))
                } else {
                    seen_causes.push(key.clone());
                    let suffix = format!(
                        "{}_{}",
                        cycle_id
                            .trim_start_matches("bcy_")
                            .chars()
                            .take(26)
                            .collect::<String>(),
                        n + 1
                    );
                    let (decision, task) = open_decision(
                        tx,
                        cycle_id,
                        &suffix,
                        &pr,
                        &key,
                        watch.origin.as_str(),
                        None,
                    )
                    .await?;
                    p["outcome"] = json!({"opened": decision});
                    opened.push(json!({"decision_id": decision, "task_id": task}));
                    None
                }
            }
        };
        if let Some(why) = why {
            p["outcome"] = json!({"skipped": why});
            skipped.push(json!({"title": title, "why": why}));
        }
    }
    report["proposals"] = json!(proposals);
    sqlx::query(
        "UPDATE platform.bot_cycle SET cycle_state = 'done', end_time = $3, snapshot = $4,
           anomalies = $5, report = $6, usage = $7, error = NULL, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(cycle_id)
    .bind(now)
    .bind(&snapshot)
    .bind(&anomalies)
    .bind(&report)
    .bind(&usage)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    acom_changeset::log_action(
        tx,
        acom_changeset::LogEntry {
            object: Some(("bot_cycle", cycle_id)),
            result: Some(json!({"opened": opened, "skipped": skipped.len()})),
            ..acom_changeset::LogEntry::new(
                ActionType::ASSESS.0,
                acom_core::enums::ActionStatus::Completed,
                "ailism_bot",
            )
        },
    )
    .await?;
    cycle_event(tx, cycle_id, "done", None).await?;
    Ok(json!({"cycle_id": cycle_id, "state": "done", "opened": opened, "skipped": skipped}))
}

/// The task the operations agent drafts from, and the decision that remembers why. `suffix`
/// makes both ids (`bdc_{suffix}`, `task_bot_{suffix}`); `revises` is the decision a person asked
/// again.
pub(crate) async fn open_decision(
    tx: &mut TxCx,
    cycle_id: &str,
    suffix: &str,
    p: &Proposal,
    key: &str,
    origin: &str,
    revises: Option<&str>,
) -> Result<(String, String), ToolError> {
    let now = tx.clock.now();
    let decision = format!("bdc_{suffix}");
    let task = format!("task_bot_{suffix}");
    let related: Vec<Value> = p
        .about
        .iter()
        .map(|(e, id)| json!({"entity": e, "id": id}))
        .collect();
    // The operations agent drafts from the sentence alone: it gets the ids too, so it acts on
    // the question or post the bot meant instead of searching for it.
    let ids: Vec<&str> = p.about.iter().map(|(_, id)| id.as_str()).collect();
    let request: String = if ids.is_empty() || ids.iter().all(|id| p.request.contains(id)) {
        p.request.clone()
    } else {
        format!("{} ({})", p.request, ids.join(", "))
    }
    .chars()
    .take(1000)
    .collect();
    let observation = json!({"bot": {"cycle_id": cycle_id, "decision_id": decision,
        "reason": p.reason, "evidence": p.evidence, "expected_effect": p.expected_effect,
        "risk": p.risk}});
    sqlx::query(
        "INSERT INTO platform.task (tenant_id, id, title, task_state, task_origin, summary, tag,
           request_text, watcher, watcher_observation, related_entities, start_time, created_at,
           updated_at)
         VALUES ($1, $2, $3, 'needs_you', $4, $5, $6, $7, 'ailism_bot', $8, $9, $10, $10, $10)",
    )
    .bind(tx.tenant.as_str())
    .bind(&task)
    .bind(&p.title)
    .bind(origin)
    .bind("Proposed by Ailism Bot; the operations agent drafts the change.")
    .bind(&p.area)
    .bind(&request)
    .bind(&observation)
    .bind(json!(related))
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    sqlx::query(
        "INSERT INTO platform.bot_decision (tenant_id, id, cycle_id, task_id, bot_area, title,
           request_text, reason, evidence, expected_effect, bot_risk, related_entities, cause_key,
           revises_id, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $15)",
    )
    .bind(tx.tenant.as_str())
    .bind(&decision)
    .bind(cycle_id)
    .bind(&task)
    .bind(&p.area)
    .bind(&p.title)
    .bind(&request)
    .bind(&p.reason)
    .bind(&p.evidence)
    .bind(&p.expected_effect)
    .bind(&p.risk)
    .bind(json!(related))
    .bind(key)
    .bind(revises)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    // `created` and `origin` let the gateway hand the task to the operations agent.
    acom_changeset::notify(
        tx,
        "task.updated",
        json!({"task_id": task, "state": "needs_you", "created": true, "origin": origin,
               "watcher": "ailism_bot"}),
    )
    .await?;
    Ok((decision, task))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proposal_keeps_what_it_can_use() {
        let p = proposal(
            &json!({"area": "pricing", "title": " Lift 2 prices ", "request": "Raise them.",
            "risk": "extreme", "evidence": ["a", 3, ""],
            "about": [{"entity": "product", "id": "prd_a"}, {"entity": "Product!", "id": "x"}]}),
        )
        .ok()
        .unwrap();
        assert_eq!(p.title, "Lift 2 prices");
        assert_eq!(p.reason, "Lift 2 prices");
        assert_eq!(p.risk, "medium");
        assert_eq!(p.evidence, json!(["a"]));
        assert_eq!(p.about, [("product".to_owned(), "prd_a".to_owned())]);
        assert!(proposal(&json!({"area": "weather", "title": "x", "request": "y"})).is_err());
        assert!(proposal(&json!({"area": "pricing", "title": "", "request": "y"})).is_err());
    }
}
