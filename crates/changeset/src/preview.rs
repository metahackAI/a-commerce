//! The preview store: `POST {path}?preview=true` saves here, `POST {path} {preview_id}` runs from
//! here (docs/api/conventions.md §7).

use acom_contract::{
    ApprovalNeed, Check, CommitResult, Committed, ErrorCode, Kind, Preview, Registry, ToolError,
    Undo,
};
use acom_core::enums::{ActionStatus, PreviewState};
use acom_core::ids::PreviewId;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{LogEntry, change_set, db_err, irreversible, log_action, op};

pub(crate) fn input_hash(input: &Value) -> String {
    let digest = Sha256::digest(input.to_string().as_bytes());
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// Previews a write and stores the preview. With `change_set_id`, the preview joins that set
/// (its `depends_on` comes from the `$item[n]` placeholders in the input).
pub async fn preview(
    registry: &Registry<Cx>,
    cx: &Cx,
    tool: &str,
    input: Value,
    change_set_id: Option<&str>,
) -> Result<Preview, ToolError> {
    let op = op(registry, tool)?;
    let meta = op.meta();
    if !matches!(meta.kind, Kind::Write | Kind::WriteApproval) {
        return Err(ToolError::invalid_input(format!(
            "`{tool}` is not a write; call it without preview."
        )));
    }
    if change_set_id.is_some() && person_endpoint(cx, meta.tool).is_some() {
        return Err(ToolError::invalid_input(format!(
            "`{tool}` is run by the person, not by a change set."
        )));
    }
    let mut planned = op.preview(cx, input.clone()).await?;
    let (needs_approval, reason) = match (&planned.inner.approval, meta.kind) {
        (ApprovalNeed::Needed { reason }, _) => (true, Some(reason.clone())),
        (_, Kind::WriteApproval) => (
            true,
            Some(format!("{} reaches customers or moves money.", meta.tool)),
        ),
        _ => (false, None),
    };
    if change_set_id.is_some() && irreversible(cx, tool) {
        planned.inner.checks.push(Check {
            rule: "change_set.irreversible".into(),
            passed: true,
            detail: Some("Cannot be undone once it runs; the change set will have no undo.".into()),
            fix_tool: None,
        });
    }
    let now = cx.clock.now();
    let ttl = cx
        .policy
        .raw("approval")
        .and_then(|p| p.get("preview_ttl_minutes"))
        .and_then(Value::as_i64)
        .unwrap_or(30);
    let expires_at = change_set_id
        .is_none()
        .then(|| now + Duration::minutes(ttl));
    let id = PreviewId::generate();

    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    sqlx::query(
        "INSERT INTO platform.preview (tenant_id, id, operation, instrument, input, input_hash,
           plan, changes, checks, needs_approval, approval_reason, basis, policy_version,
           actor_kind, agent_id, agent_name, request_id, preview_state, expires_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,'open',$18)",
    )
    .bind(cx.tenant.as_str())
    .bind(id.as_str())
    .bind(meta.address)
    .bind(meta.tool)
    .bind(&input)
    .bind(input_hash(&input))
    .bind(&planned.plan)
    .bind(json!(planned.inner.changes))
    .bind(json!(planned.inner.checks))
    .bind(needs_approval)
    .bind(reason.as_deref())
    .bind(json!(planned.inner.basis))
    .bind(cx.policy.version())
    .bind(cx.principal.kind.as_str())
    .bind(&cx.principal.subject)
    .bind(cx.principal.name.as_deref())
    .bind(&cx.request_id)
    .bind(expires_at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    if let Some(cs) = change_set_id {
        change_set::attach(&mut tx, cs, id.as_str(), &input, None).await?;
    }
    // Preparing again (an order, a subscription) replaces the caller's earlier preparation:
    // only the newest one can be run.
    if person_endpoint(cx, meta.tool).is_some() {
        sqlx::query(
            "UPDATE platform.preview SET preview_state = 'superseded', superseded_by_id = $5,
               updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND instrument = $2 AND agent_id = $3 AND preview_state = 'open'
               AND id <> $4",
        )
        .bind(cx.tenant.as_str())
        .bind(meta.tool)
        .bind(&cx.principal.subject)
        .bind(id.as_str())
        .bind(id.as_str())
        .execute(&mut *tx.tx)
        .await
        .map_err(db_err)?;
    }
    let object = planned
        .inner
        .changes
        .first()
        .map(|c| (c.entity.as_str(), c.id.as_str()));
    log_action(
        &mut tx,
        LogEntry {
            object,
            result: Some(json!({
                "changes": planned.inner.changes.len(),
                "checks_failed": planned.inner.checks.iter().filter(|c| !c.passed).count(),
                "needs_approval": needs_approval,
            })),
            change_set_id,
            preview_id: Some(id.as_str()),
            ..LogEntry::new(meta.action_type.0, ActionStatus::Potential, meta.tool)
        },
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;

    Ok(Preview {
        preview_id: id.to_string(),
        expires_at: expires_at.map(|t| cx.time.format(t)),
        changes: planned.inner.changes,
        checks: planned.inner.checks,
        needs_approval,
        approval_reason: reason,
        change_set_id: change_set_id.map(str::to_owned),
    })
}

/// A stored preview, as `FOR UPDATE` read inside a transaction.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct Stored {
    pub id: String,
    pub instrument: String,
    pub input: Value,
    pub plan: Value,
    pub changes: Value,
    pub checks: Value,
    pub basis: Value,
    pub policy_version: String,
    pub needs_approval: bool,
    pub agent_id: Option<String>,
    pub change_set_id: Option<String>,
    pub preview_state: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub superseded_by_id: Option<String>,
    pub result: Option<Value>,
}

pub(crate) async fn load(tx: &mut TxCx, preview_id: &str) -> Result<Stored, ToolError> {
    sqlx::query_as::<_, Stored>(
        "SELECT id, instrument, input, plan, changes, checks, basis, policy_version, needs_approval,
                agent_id, change_set_id, preview_state, expires_at, superseded_by_id, result
         FROM platform.preview WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tx.tenant.as_str())
    .bind(preview_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db_err)?
    .ok_or_else(|| {
        ToolError::new(
            ErrorCode::PREVIEW_NOT_FOUND,
            format!("There is no preview {preview_id}."),
        )
        .detail(json!({"preview_id": preview_id}))
    })
}

/// Refuses a preview that can no longer run: superseded, expired, or past its deadline.
pub(crate) fn ensure_open(p: &Stored, now: DateTime<Utc>) -> Result<(), ToolError> {
    match p.preview_state.as_str() {
        s if s == PreviewState::Superseded.as_str() => Err(ToolError::new(
            ErrorCode::PREVIEW_SUPERSEDED,
            "A newer preview replaced this one; run that one instead.",
        )
        .detail(json!({"preview_id": p.id, "superseded_by": p.superseded_by_id}))),
        s if s == PreviewState::Open.as_str() => {
            if p.expires_at.is_some_and(|e| e < now) {
                Err(ToolError::new(
                    ErrorCode::PREVIEW_EXPIRED,
                    "The preview expired; preview the change again.",
                )
                .detail(json!({"preview_id": p.id})))
            } else {
                Ok(())
            }
        }
        other => Err(ToolError::new(
            ErrorCode::PREVIEW_EXPIRED,
            format!("Preview {} is {other}.", p.id),
        )
        .detail(json!({"preview_id": p.id}))),
    }
}

/// Whether every row the preview read still has the version it saw.
pub(crate) async fn basis_unchanged(tx: &mut TxCx, basis: &Value) -> Result<bool, ToolError> {
    for b in basis.as_array().into_iter().flatten() {
        let (Some(entity), Some(id), Some(version)) = (
            b["entity"].as_str(),
            b["id"].as_str(),
            b["version"].as_i64(),
        ) else {
            continue;
        };
        let well_formed = entity.split('.').count() == 2
            && entity
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c == b'_' || c == b'.');
        if !well_formed {
            return Err(ToolError::new(
                ErrorCode::INTERNAL,
                format!("bad basis entity `{entity}`"),
            ));
        }
        // `entity` is checked above to be `schema.table` in lowercase words only.
        let current: Option<i32> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT version FROM {entity} WHERE tenant_id = $1 AND id = $2"
        )))
        .bind(tx.tenant.as_str())
        .bind(id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db_err)?;
        if current.map(i64::from) != Some(version) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Records that a preview ran, with what it produced and the plan that actually ran.
pub async fn mark_committed(
    tx: &mut TxCx,
    preview_id: &str,
    plan: &Value,
    done: &Committed,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE platform.preview SET preview_state = 'committed', committed_at = $3, plan = $4,
           result = $5, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(preview_id)
    .bind(tx.clock.now())
    .bind(plan)
    .bind(json!(done))
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    Ok(())
}

pub(crate) fn first_result(p: &Stored) -> Result<Committed, ToolError> {
    p.result
        .clone()
        .and_then(|r| serde_json::from_value(r).ok())
        .ok_or_else(|| ToolError::new(ErrorCode::INTERNAL, "committed preview has no result"))
}

fn first_failed(checks: &Value) -> Option<(String, Option<String>)> {
    checks.as_array()?.iter().find_map(|c| {
        (c["passed"] == Value::Bool(false)).then(|| {
            (
                c["rule"].as_str().unwrap_or("").to_owned(),
                c["detail"].as_str().map(str::to_owned),
            )
        })
    })
}

/// Fails when the preview's checks failed, or when the store or the rules changed since the
/// preview and a fresh preview differs.
pub(crate) async fn still_right(
    registry: &Registry<Cx>,
    cx: &Cx,
    tx: &mut TxCx,
    p: &Stored,
) -> Result<(), ToolError> {
    if let Some((rule, detail)) = first_failed(&p.checks) {
        return Err(ToolError::new(
            ErrorCode::POLICY_VIOLATION,
            detail.unwrap_or_else(|| format!("The `{rule}` check failed in this preview.")),
        )
        .detail(json!({"rule": rule})));
    }
    let fresh = basis_unchanged(tx, &p.basis).await? && p.policy_version == cx.policy.version();
    if fresh {
        return Ok(());
    }
    let op = op(registry, &p.instrument)?;
    let again = op.preview(cx, p.input.clone()).await?;
    if !crate::same_changes(&json!(again.inner.changes), &p.changes) {
        return Err(ToolError::new(
            ErrorCode::ESTIMATE_CHANGED,
            "The store changed since this preview. Preview the change again to see the new result.",
        )
        .detail(
            json!({"new_preview": {"changes": again.inner.changes, "checks": again.inner.checks}}),
        )
        .next(op.meta().tool));
    }
    if let Some(failed) = again.inner.checks.iter().find(|c| !c.passed) {
        return Err(ToolError::new(
            ErrorCode::POLICY_CHANGED,
            format!(
                "A rule changed since this preview and `{}` now fails.",
                failed.rule
            ),
        )
        .detail(json!({"new_preview": {"checks": again.inner.checks}}))
        .next(op.meta().tool));
    }
    Ok(())
}

/// The person-only endpoint that runs a "preview only" tool's preview
/// (`approval.preview_only`: `prepare_order` → `POST /v1/sales/orders`), if `tool` is one.
pub fn person_endpoint(cx: &Cx, tool: &str) -> Option<String> {
    cx.policy
        .raw("approval")
        .and_then(|p| p.get("preview_only"))
        .and_then(|m| m.get(tool))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Runs a stored preview on its own (`POST {path} {preview_id}`): only a write that needs no
/// approval, is not part of a change set, and is not "preview only" (those run from a
/// person-only endpoint through [`claim_preview`]).
pub async fn commit(
    registry: &Registry<Cx>,
    cx: &Cx,
    tool: &str,
    preview_id: &str,
) -> Result<CommitResult, ToolError> {
    let op = op(registry, tool)?;
    let meta = op.meta();
    if let Some(endpoint) = person_endpoint(cx, meta.tool) {
        return Err(ToolError::invalid_input(format!(
            "`{tool}` only prepares; the person runs it with `{endpoint}` from the confirmation card."
        ))
        .detail(json!({"field": "preview_id", "reason": "preview_only", "endpoint": endpoint})));
    }
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let p = load(&mut tx, preview_id).await?;
    if p.instrument != meta.tool {
        return Err(ToolError::invalid_input(format!(
            "Preview {preview_id} is for `{}`, not `{tool}`.",
            p.instrument
        )));
    }
    if p.agent_id.as_deref() != Some(cx.principal.subject.as_str()) {
        return Err(ToolError::new(
            ErrorCode::FORBIDDEN,
            "This preview was made by someone else.",
        )
        .detail(json!({"reason": "owner"})));
    }
    if p.preview_state == PreviewState::Committed.as_str() {
        return Ok(run_result(first_result(&p)?));
    }
    if let Some(cs) = &p.change_set_id {
        return Err(ToolError::new(
            ErrorCode::APPROVAL_REQUIRED,
            format!(
                "This preview is part of change set {cs}; it runs when a person approves the set."
            ),
        )
        .detail(json!({"change_set_id": cs})));
    }
    let now = cx.clock.now();
    if let Err(e) = ensure_open(&p, now) {
        if p.preview_state == PreviewState::Open.as_str() {
            sqlx::query(
                "UPDATE platform.preview SET preview_state = 'expired', updated_at = now(),
                   version = version + 1 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(cx.tenant.as_str())
            .bind(preview_id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db_err)?;
            tx.commit().await.map_err(ToolError::from)?;
        }
        return Err(e.next(meta.tool));
    }
    if p.needs_approval {
        return Err(ToolError::new(
            ErrorCode::APPROVAL_REQUIRED,
            "A person must approve this change. Preview it into a change set and submit the set.",
        )
        .detail(json!({"change_set_id": null, "state": p.preview_state})));
    }
    still_right(registry, cx, &mut tx, &p).await?;
    tx.preview_id = Some(p.id.clone());
    let done = op.commit(&mut tx, &p.plan).await?;
    mark_committed(&mut tx, preview_id, &p.plan, &done).await?;
    log_action(
        &mut tx,
        LogEntry {
            object: crate::main_object(&p.changes, &done.id),
            result: Some(json!({"id": done.id, "produced": done.produced})),
            preview_id: Some(preview_id),
            ..LogEntry::new(meta.action_type.0, ActionStatus::Completed, meta.tool)
        },
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(run_result(done))
}

/// The answer to a run: the main id, an `undo` the write offers (a remembered preference names
/// its memory), and every other slot it produced (`cart`, `memory`…).
pub(crate) fn run_result(done: Committed) -> CommitResult {
    let mut produced = done.produced;
    let undo = produced
        .remove("undo")
        .and_then(|u| serde_json::from_value::<Undo>(u).ok());
    produced.remove("ok");
    produced.remove("id");
    CommitResult {
        ok: true,
        id: done.id,
        undo,
        produced,
    }
}

/// A preview a person-only endpoint runs (`POST /v1/sales/orders {preview_id}` runs a
/// `prepare_order` preview).
#[derive(Debug, Clone)]
pub struct ClaimedPreview {
    pub id: String,
    pub input: Value,
    pub plan: Value,
    pub changes: Value,
    /// `Some` when the same preview already ran: the endpoint returns the first result.
    pub already: Option<Committed>,
}

/// Locks the caller's own open preview of `tool` inside the endpoint's transaction. The endpoint
/// runs the plan and then calls [`mark_committed`] in the same transaction.
pub async fn claim_preview(
    tx: &mut TxCx,
    preview_id: &str,
    tool: &str,
) -> Result<ClaimedPreview, ToolError> {
    let p = load(tx, preview_id).await?;
    if p.instrument != tool {
        return Err(ToolError::invalid_input(format!(
            "Preview {preview_id} is for `{}`, not `{tool}`.",
            p.instrument
        )));
    }
    if p.agent_id.as_deref() != Some(tx.principal.subject.as_str()) {
        return Err(ToolError::new(
            ErrorCode::FORBIDDEN,
            "This preview was made for someone else.",
        )
        .detail(json!({"reason": "owner"})));
    }
    let already = if p.preview_state == PreviewState::Committed.as_str() {
        Some(first_result(&p)?)
    } else {
        ensure_open(&p, tx.clock.now())?;
        None
    };
    Ok(ClaimedPreview {
        id: p.id,
        input: p.input,
        plan: p.plan,
        changes: p.changes,
        already,
    })
}

/// `GET /v1/platform/previews/{preview_id}`.
pub async fn get(cx: &Cx, preview_id: &str) -> Result<Value, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    #[allow(clippy::type_complexity)]
    let row: Option<(
        String,
        String,
        Value,
        Value,
        Value,
        bool,
        Option<String>,
        Option<DateTime<Utc>>,
        String,
        Option<String>,
        String,
        DateTime<Utc>,
    )> = sqlx::query_as(
        "SELECT operation, instrument, input, changes, checks, needs_approval, approval_reason,
                expires_at, preview_state, change_set_id, policy_version, created_at
         FROM platform.preview WHERE tenant_id = $1 AND id = $2",
    )
    .bind(cx.tenant.as_str())
    .bind(preview_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    tx.commit().await.map_err(ToolError::from)?;
    let r = row.ok_or_else(|| {
        ToolError::new(
            ErrorCode::PREVIEW_NOT_FOUND,
            format!("There is no preview {preview_id}."),
        )
    })?;
    Ok(json!({
        "preview_id": preview_id, "operation": r.0, "tool": r.1, "input": r.2, "changes": r.3,
        "checks": r.4, "needs_approval": r.5, "approval_reason": r.6,
        "expires_at": r.7.map(|t| cx.time.format(t)), "state": r.8, "change_set_id": r.9,
        "policy_version": r.10, "created_at": cx.time.format(r.11),
    }))
}
