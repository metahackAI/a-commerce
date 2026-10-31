//! What `get_change_set`, `search_change_sets`, and `verify_change_set` return
//! (docs/api/platform.md#change_set). The platform domain wraps these as read tools.

use acom_contract::ToolError;
use acom_contract::fields;
use acom_core::enums::ChangeSetState;
use acom_store::Cx;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::change_set::undo_deadline;
use crate::{CHANGE_SET_NOT_FOUND, CHANGE_SET_NOT_RUN, db_err};

/// How many `changes` rows an item shows before `changes_total` says how many there are.
const CHANGES_SHOWN: usize = 20;

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: String,
    number: i32,
    title: String,
    change_set_state: String,
    request_text: Vec<String>,
    interpretation: Value,
    conversation_id: Option<String>,
    created_by: String,
    agent_name: Option<String>,
    submitted_at: Option<DateTime<Utc>>,
    approved_by: Option<String>,
    approved_at: Option<DateTime<Utc>>,
    rejected_at: Option<DateTime<Utc>>,
    rejection_reason: Option<String>,
    verified_at: Option<DateTime<Utc>>,
    verification: Option<Value>,
    undo_until: Option<DateTime<Utc>>,
    undone_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    version: i32,
}

const COLUMNS: &str = "id, number, title, change_set_state, request_text, interpretation,
    conversation_id, created_by, agent_name, submitted_at, approved_by, approved_at, rejected_at,
    rejection_reason, verified_at, verification, undo_until, undone_at, created_at, version";

#[derive(Debug, sqlx::FromRow)]
struct ItemRow {
    position: i32,
    preview_id: String,
    depends_on: Vec<String>,
    result_id: Option<String>,
    verification: Option<Value>,
    operation: String,
    instrument: String,
    input: Value,
    changes: Value,
    checks: Value,
    needs_approval: bool,
    approval_reason: Option<String>,
    preview_state: String,
}

fn not_found(cs: &str) -> ToolError {
    ToolError::new(
        CHANGE_SET_NOT_FOUND,
        format!("There is no change set {cs}."),
    )
    .detail(json!({"entity": "change_set", "id": cs}))
    .next("search_change_sets")
}

fn time(cx: &Cx, t: Option<DateTime<Utc>>) -> Value {
    t.map_or(Value::Null, |t| json!(cx.time.format(t)))
}

/// One change set, expanded: items with their changes and checks, what was left out, the undo
/// deadline (estimated until it runs), and the history of transitions.
pub async fn get(cx: &Cx, cs: &str, fields: &[String]) -> Result<Value, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let row = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM platform.change_set WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(cx.tenant.as_str())
    .bind(cs)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db_err)?
    .ok_or_else(|| not_found(cs))?;
    let items = sqlx::query_as::<_, ItemRow>(
        "SELECT i.position, i.preview_id, i.depends_on, i.result_id, i.verification,
                p.operation, p.instrument, p.input, p.changes, p.checks, p.needs_approval,
                p.approval_reason, p.preview_state
         FROM platform.change_set_item i
         JOIN platform.preview p ON p.tenant_id = i.tenant_id AND p.id = i.preview_id
         WHERE i.tenant_id = $1 AND i.change_set_id = $2 ORDER BY i.position",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    #[allow(clippy::type_complexity)]
    let history: Vec<(
        i32,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<Value>,
        Option<Value>,
        DateTime<Utc>,
        i32,
    )> = sqlx::query_as(
        "SELECT position, action_type, action_status, agent_id, agent_name, from_state, to_state,
                result, error, start_time, change_set_version
         FROM platform.change_set_event WHERE tenant_id = $1 AND change_set_id = $2
         ORDER BY position",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    tx.commit().await.map_err(ToolError::from)?;

    let state: ChangeSetState = row
        .change_set_state
        .parse()
        .unwrap_or(ChangeSetState::Failed);
    let (mut passed, mut failed) = (0usize, 0usize);
    let mut left_out = Vec::new();
    let mut out_items = Vec::new();
    for i in &items {
        let changes = i.changes.as_array().cloned().unwrap_or_default();
        let (excluded, shown): (Vec<_>, Vec<_>) =
            changes.iter().partition(|c| c["op"] == "exclude");
        for c in &excluded {
            left_out.push(
                json!({"entity": c["entity"], "id": c["id"], "reason": c["note"],
                                 "preview_id": i.preview_id}),
            );
        }
        for c in i.checks.as_array().into_iter().flatten() {
            if c["passed"] == true {
                passed += 1;
            } else {
                failed += 1;
            }
        }
        let action_status = match (i.preview_state.as_str(), state) {
            ("committed", _) => "completed",
            (_, ChangeSetState::Failed) => "failed",
            _ => "potential",
        };
        out_items.push(json!({
            "position": i.position, "preview_id": i.preview_id, "tool": i.instrument,
            "address": i.operation, "depends_on": i.depends_on, "action_status": action_status,
            "needs_approval": i.needs_approval, "approval_reason": i.approval_reason,
            "input": i.input,
            "changes": shown.iter().take(CHANGES_SHOWN).collect::<Vec<_>>(),
            "changes_total": changes.len(), "checks": i.checks,
            "result_id": i.result_id, "verification": i.verification,
        }));
    }
    let undo_until = match row.undo_until {
        Some(t) => Some(t),
        // Before it runs: the deadline it would get if it ran now.
        None if matches!(
            state,
            ChangeSetState::Drafting | ChangeSetState::WaitingForApproval
        ) =>
        {
            undo_deadline(
                cx,
                items.iter().map(|i| (i.instrument.as_str(), &i.input)),
                |_| true,
                cx.clock.now(),
            )
        }
        None => None,
    };
    let verification = row
        .verification
        .as_ref()
        .map(|v| json!({"state": v["state"], "matched": v["matched"], "total": v["total"]}));
    let history: Vec<Value> = history
        .into_iter()
        .map(|h| {
            json!({"position": h.0, "action_type": h.1, "action_status": h.2, "agent_id": h.3,
                   "agent_name": h.4, "from": h.5, "to": h.6, "result": h.7, "error": h.8,
                   "time": cx.time.format(h.9), "version": h.10})
        })
        .collect();

    let full = json!({
        "id": row.id, "number": row.number, "version": row.version, "state": row.change_set_state,
        "title": row.title,
        "request": {"text": row.request_text, "conversation_id": row.conversation_id,
                    "requested_by": row.created_by},
        "interpretation": row.interpretation,
        "items": out_items,
        "left_out": left_out,
        "checks_summary": {"passed": passed, "failed": failed, "total": passed + failed},
        "needs_approval": items.iter().any(|i| i.needs_approval),
        "undo_until": time(cx, undo_until),
        "created_by": row.created_by,
        "agent": row.agent_name.map(|n| json!({"@type": "SoftwareApplication", "name": n})),
        "created_at": cx.time.format(row.created_at),
        "submitted_at": time(cx, row.submitted_at),
        "approved_by": row.approved_by, "approved_at": time(cx, row.approved_at),
        "rejected_at": time(cx, row.rejected_at), "rejection_reason": row.rejection_reason,
        "verified_at": time(cx, row.verified_at), "verification": verification,
        "undone_at": time(cx, row.undone_at),
        "history": history,
    });
    fields::keep(full, fields)
}

/// Filters for [`search`].
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub states: Vec<ChangeSetState>,
    pub created_by: Option<String>,
    pub created_from: Option<DateTime<Utc>>,
    pub created_through: Option<DateTime<Utc>>,
    pub undoable_only: bool,
    /// `-created_at` (default), `created_at`, or `-number`.
    pub sort: String,
    pub limit: i64,
    pub offset: i64,
}

/// Change sets by state, creator, and time; newest first by default.
pub async fn search(cx: &Cx, f: &Filter) -> Result<Value, ToolError> {
    let order = match f.sort.as_str() {
        "created_at" => "cs.created_at ASC, cs.number ASC",
        "-number" => "cs.number DESC",
        _ => "cs.created_at DESC, cs.number DESC",
    };
    let states: Vec<&str> = f.states.iter().map(|s| s.as_str()).collect();
    let now = cx.clock.now();
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let filter = "cs.tenant_id = $1
        AND (cardinality($2::text[]) = 0 OR cs.change_set_state = ANY($2))
        AND ($3::text IS NULL OR cs.created_by = $3)
        AND ($4::timestamptz IS NULL OR cs.created_at >= $4)
        AND ($5::timestamptz IS NULL OR cs.created_at <= $5)
        AND (NOT $6 OR (cs.change_set_state = 'live' AND cs.undo_until > $7))";
    // `filter` and `order` are fixed strings; every value is a bind parameter.
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM platform.change_set cs WHERE {filter}"
    )))
    .bind(cx.tenant.as_str())
    .bind(&states)
    .bind(f.created_by.as_deref())
    .bind(f.created_from)
    .bind(f.created_through)
    .bind(f.undoable_only)
    .bind(now)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        String,
        i32,
        String,
        String,
        i32,
        i64,
        bool,
        String,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
        Option<String>,
    )> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT cs.id, cs.number, cs.title, cs.change_set_state, cs.version,
                (SELECT count(*) FROM platform.change_set_item i
                  WHERE i.tenant_id = cs.tenant_id AND i.change_set_id = cs.id),
                coalesce((SELECT bool_or(p.needs_approval) FROM platform.change_set_item i
                  JOIN platform.preview p ON p.tenant_id = i.tenant_id AND p.id = i.preview_id
                  WHERE i.tenant_id = cs.tenant_id AND i.change_set_id = cs.id), false),
                cs.created_by, cs.created_at, cs.undo_until, cs.agent_name
         FROM platform.change_set cs WHERE {filter}
         ORDER BY {order} LIMIT $8 OFFSET $9"
    )))
    .bind(cx.tenant.as_str())
    .bind(&states)
    .bind(f.created_by.as_deref())
    .bind(f.created_from)
    .bind(f.created_through)
    .bind(f.undoable_only)
    .bind(now)
    .bind(f.limit)
    .bind(f.offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    tx.commit().await.map_err(ToolError::from)?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|r| {
            json!({"id": r.0, "number": r.1, "title": r.2, "state": r.3, "version": r.4,
                   "item_count": r.5, "needs_approval": r.6, "created_by": r.7,
                   "created_at": cx.time.format(r.8), "undo_until": time(cx, r.9),
                   "agent_name": r.10})
        })
        .collect();
    let next = f.offset + i64::try_from(items.len()).unwrap_or(0);
    Ok(json!({
        "items": items, "total": total,
        "next_cursor": (next < total).then(|| next.to_string()),
    }))
}

/// What the runner recorded when it read the changes back.
pub async fn verification(cx: &Cx, cs: &str) -> Result<Value, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    #[allow(clippy::type_complexity)]
    let row: Option<(String, i32, Option<DateTime<Utc>>, Option<Value>)> = sqlx::query_as(
        "SELECT change_set_state, version, verified_at, verification FROM platform.change_set
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    tx.commit().await.map_err(ToolError::from)?;
    let (state, version, verified_at, v) = row.ok_or_else(|| not_found(cs))?;
    let Some(v) = v else {
        return Err(ToolError::new(
            CHANGE_SET_NOT_RUN,
            format!("{cs} has not run yet, so there is nothing to verify."),
        )
        .next("get_change_set"));
    };
    let items: Vec<Value> = v["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            json!({"preview_id": i["preview_id"], "tool": i["tool"], "result_id": i["result_id"],
                   "matched": i["matched"], "mismatches": i["mismatches"]})
        })
        .collect();
    Ok(
        json!({"items": items, "total": items.len(), "change_set_id": cs, "version": version,
              "state": state, "verified_at": time(cx, verified_at)}),
    )
}
