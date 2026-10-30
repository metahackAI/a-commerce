//! The change-set state machine (docs/api/platform.md#change_set):
//!
//! ```text
//! drafting ─submit─▶ waiting_for_approval ─approve─▶ running ─▶ verifying ─▶ live ─undo─▶ undone
//!    ▲                    │        └─reject─▶ rejected   └──────────▶ failed ◀──┘
//!    └── items changed, or approval found a different estimate (version + 1)
//! ```
//!
//! The runtime drafts and submits (scope `changeset:draft`); only a person approves, rejects,
//! and undoes (`human:*`); the runner (`sys_runner`) runs and verifies what was approved.

use std::collections::{BTreeMap, BTreeSet};

use acom_contract::{ActionType, Committed, ErrorCode, Registry, ToolError, UndoSupport};
use acom_core::StoreTime;
use acom_core::enums::{ActionStatus, ChangeSetState, PreviewState};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Days, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::preview::{self, ensure_open, first_result, mark_committed};
use crate::{
    ALREADY_EXISTS, CHANGE_SET_NOT_FOUND, IRREVERSIBLE_CHANGE, LogEntry, RUNNER, bind, db_err,
    irreversible, log_action, op,
};

/// `POST /v1/platform/change-sets`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewChangeSet {
    pub title: String,
    #[serde(default)]
    pub request_text: Vec<String>,
    #[serde(default)]
    pub interpretation: Vec<Value>,
    #[serde(default)]
    pub conversation_id: Option<String>,
}

/// One item in `POST …/items` and `PUT …/items`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemRef {
    pub preview_id: String,
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,
}

#[derive(Debug, sqlx::FromRow)]
struct Head {
    change_set_state: String,
    version: i32,
    submitted_at: Option<DateTime<Utc>>,
    undo_until: Option<DateTime<Utc>>,
    undone_at: Option<DateTime<Utc>>,
}

impl Head {
    fn state(&self) -> ChangeSetState {
        self.change_set_state
            .parse()
            .unwrap_or(ChangeSetState::Failed)
    }
}

async fn lock(tx: &mut TxCx, cs: &str) -> Result<Head, ToolError> {
    sqlx::query_as::<_, Head>(
        "SELECT change_set_state, version, submitted_at, undo_until, undone_at
         FROM platform.change_set WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tx.tenant.as_str())
    .bind(cs)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db_err)?
    .ok_or_else(|| {
        ToolError::new(
            CHANGE_SET_NOT_FOUND,
            format!("There is no change set {cs}."),
        )
        .detail(json!({"entity": "change_set", "id": cs}))
        .next("search_change_sets")
    })
}

fn invalid_state(cs: &str, state: ChangeSetState, wanted: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INVALID_STATE,
        format!("Change set {cs} is {state}; {wanted}."),
    )
    .detail(json!({"entity": "change_set", "from": state.as_str()}))
}

/// One `platform.change_set_event` row, and a notification for the event stream.
#[allow(clippy::too_many_arguments)]
async fn event(
    tx: &mut TxCx,
    cs: &str,
    action_type: &str,
    status: ActionStatus,
    from: Option<ChangeSetState>,
    to: ChangeSetState,
    result: Option<Value>,
    error: Option<&ToolError>,
) -> Result<(), ToolError> {
    let now = tx.clock.now();
    let p = tx.principal.clone();
    let version: i32 = sqlx::query_scalar(
        "INSERT INTO platform.change_set_event (tenant_id, change_set_id, position, action_type,
           action_status, agent_id, agent_name, instrument, from_state, to_state, result, error,
           start_time, end_time, request_id, change_set_version)
         SELECT $1, $2, coalesce((SELECT max(position) FROM platform.change_set_event
                                  WHERE tenant_id = $1 AND change_set_id = $2), 0) + 1,
                $3, $4, $5, $6, $3, $7, $8, $9, $10, $11, $11, $12, cs.version
         FROM platform.change_set cs WHERE cs.tenant_id = $1 AND cs.id = $2
         RETURNING change_set_version",
    )
    .bind(tx.tenant.as_str())
    .bind(cs)
    .bind(action_type)
    .bind(status.as_str())
    .bind(&p.subject)
    .bind(p.name.as_deref())
    .bind(from.map(|f| f.as_str()))
    .bind(to.as_str())
    .bind(result)
    .bind(error.map(|e| json!({"code": e.code, "class": e.class, "message": e.message})))
    .bind(now)
    .bind(&tx.request_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    crate::notify(
        tx,
        "change_set.updated",
        json!({"change_set_id": cs, "version": version, "state": to.as_str()}),
    )
    .await?;
    if matches!(to, ChangeSetState::Live | ChangeSetState::Rejected) {
        close_tasks(tx, cs, to).await?;
    }
    Ok(())
}

/// A task is done once its change set is live or rejected (docs/api/platform.md#task); its
/// summary then says how the set ended instead of what was being drafted.
async fn close_tasks(tx: &mut TxCx, cs: &str, to: ChangeSetState) -> Result<(), ToolError> {
    let now = tx.clock.now();
    let (number, title): (i32, String) = sqlx::query_as(
        "SELECT number, title FROM platform.change_set WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(cs)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    let summary = match to {
        ChangeSetState::Live => format!("#{number} ran: {title}"),
        _ => format!("#{number} was rejected: {title}"),
    };
    let closed: Vec<String> = sqlx::query_scalar(
        "UPDATE platform.task SET task_state = 'done', end_time = $3, updated_at = $3,
           summary = $4, version = version + 1
         WHERE tenant_id = $1 AND change_set_id = $2 AND task_state IN ('needs_you', 'working')
         RETURNING id",
    )
    .bind(tx.tenant.as_str())
    .bind(cs)
    .bind(now)
    .bind(&summary)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    for id in closed {
        crate::jobs::task_event(tx, &id, "done", Some(cs)).await?;
    }
    Ok(())
}

async fn set_state(
    tx: &mut TxCx,
    cs: &str,
    to: ChangeSetState,
    bump_version: bool,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE platform.change_set SET change_set_state = $3, updated_at = now(),
           version = version + CASE WHEN $4 THEN 1 ELSE 0 END
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(cs)
    .bind(to.as_str())
    .bind(bump_version)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    Ok(())
}

async fn summary(tx: &mut TxCx, cs: &str) -> Result<Value, ToolError> {
    let (state, version, count): (String, i32, i64) = sqlx::query_as(
        "SELECT cs.change_set_state, cs.version,
                (SELECT count(*) FROM platform.change_set_item i
                  WHERE i.tenant_id = cs.tenant_id AND i.change_set_id = cs.id)
         FROM platform.change_set cs WHERE cs.tenant_id = $1 AND cs.id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(cs)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    Ok(json!({"id": cs, "version": version, "state": state, "item_count": count}))
}

// ---------------------------------------------------------------------------------------------
// Runtime endpoints (scope `changeset:draft`)

/// `POST /v1/platform/change-sets`: a new set in `drafting`, version 1.
pub async fn create(cx: &Cx, new: NewChangeSet) -> Result<Value, ToolError> {
    if new.title.trim().is_empty() {
        return Err(ToolError::invalid_input("A change set needs a title.")
            .detail(json!({"field": "title", "reason": "missing"})));
    }
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    // One numbering per store; the advisory lock keeps two drafts from taking the same number.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('change_set_number:' || $1))")
        .bind(cx.tenant.as_str())
        .execute(&mut *tx.tx)
        .await
        .map_err(db_err)?;
    let number: i32 = sqlx::query_scalar(
        "SELECT coalesce(max(number), 1000) + 1 FROM platform.change_set WHERE tenant_id = $1",
    )
    .bind(cx.tenant.as_str())
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    let id = format!("cs_{number}");
    sqlx::query(
        "INSERT INTO platform.change_set (tenant_id, id, number, title, change_set_state,
           request_text, interpretation, conversation_id, created_by, agent_name, created_at)
         VALUES ($1,$2,$3,$4,'drafting',$5,$6,$7,$8,$9,$10)",
    )
    .bind(cx.tenant.as_str())
    .bind(&id)
    .bind(number)
    .bind(new.title.trim())
    .bind(&new.request_text)
    .bind(Value::Array(new.interpretation))
    .bind(new.conversation_id.as_deref())
    .bind(&cx.principal.subject)
    .bind(cx.principal.name.as_deref())
    // On the store clock, like every other time of the change set (the demo clock in demo mode).
    .bind(cx.clock.now())
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    event(
        &mut tx,
        &id,
        ActionType::CREATE.0,
        ActionStatus::Completed,
        None,
        ChangeSetState::Drafting,
        None,
        None,
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(json!({"id": id, "number": number, "version": 1, "state": "drafting"}))
}

/// Puts a stored preview at the end of the set. A set that was submitted before goes back to
/// drafting with version + 1. `depends_on` defaults to the items the input's placeholders name.
pub(crate) async fn attach(
    tx: &mut TxCx,
    cs: &str,
    preview_id: &str,
    input: &Value,
    depends_on: Option<Vec<String>>,
) -> Result<(), ToolError> {
    let head = lock(tx, cs).await?;
    let state = head.state();
    if !matches!(
        state,
        ChangeSetState::Drafting | ChangeSetState::WaitingForApproval
    ) {
        return Err(invalid_state(cs, state, "it no longer takes items"));
    }
    let items: Vec<(i32, String)> = sqlx::query_as(
        "SELECT position, preview_id FROM platform.change_set_item
         WHERE tenant_id = $1 AND change_set_id = $2 ORDER BY position",
    )
    .bind(tx.tenant.as_str())
    .bind(cs)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    if items.iter().any(|(_, p)| p == preview_id) {
        return Err(ToolError::new(
            ALREADY_EXISTS,
            format!("Preview {preview_id} is already in {cs}."),
        )
        .detail(json!({"entity": "change_set_item", "existing_id": preview_id})));
    }
    let position = items.last().map_or(1, |(p, _)| p + 1);
    let depends_on = match depends_on {
        Some(d) => d,
        None => bind::positions(input)
            .into_iter()
            .map(|n| {
                items
                    .iter()
                    .find(|(p, _)| *p == n)
                    .map(|(_, id)| id.clone())
                    .ok_or_else(|| {
                        ToolError::invalid_input(format!(
                            "The input refers to item {n}, which {cs} does not have."
                        ))
                    })
            })
            .collect::<Result<_, _>>()?,
    };
    sqlx::query(
        "UPDATE platform.preview SET change_set_id = $3, expires_at = NULL, updated_at = now()
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(preview_id)
    .bind(cs)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    sqlx::query(
        "INSERT INTO platform.change_set_item (tenant_id, change_set_id, position, preview_id,
           depends_on) VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(tx.tenant.as_str())
    .bind(cs)
    .bind(position)
    .bind(preview_id)
    .bind(&depends_on)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    let was_submitted = head.submitted_at.is_some();
    set_state(tx, cs, ChangeSetState::Drafting, was_submitted).await?;
    event(
        tx,
        cs,
        ActionType::ADD.0,
        ActionStatus::Completed,
        Some(state),
        ChangeSetState::Drafting,
        Some(json!({"preview_id": preview_id, "position": position})),
        None,
    )
    .await
}

/// `POST /v1/platform/change-sets/{id}/items`.
pub async fn add_item(cx: &Cx, cs: &str, item: ItemRef) -> Result<Value, ToolError> {
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let p = preview::load(&mut tx, &item.preview_id).await?;
    ensure_open(&p, cx.clock.now())?;
    if preview::person_endpoint(cx, &p.instrument).is_some() {
        return Err(ToolError::invalid_input(format!(
            "`{}` is run by the person, not by a change set.",
            p.instrument
        )));
    }
    if let Some(other) = p.change_set_id.as_deref().filter(|o| *o != cs) {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("Preview {} already belongs to {other}.", p.id),
        ));
    }
    attach(&mut tx, cs, &item.preview_id, &p.input, item.depends_on).await?;
    let out = summary(&mut tx, cs).await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(out)
}

/// `DELETE /v1/platform/change-sets/{id}/items/{preview_id}`. The preview becomes `superseded`.
pub async fn remove_item(cx: &Cx, cs: &str, preview_id: &str) -> Result<Value, ToolError> {
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let head = lock(&mut tx, cs).await?;
    let state = head.state();
    if !matches!(
        state,
        ChangeSetState::Drafting | ChangeSetState::WaitingForApproval
    ) {
        return Err(invalid_state(cs, state, "its items can no longer change"));
    }
    let removed = sqlx::query(
        "DELETE FROM platform.change_set_item
         WHERE tenant_id = $1 AND change_set_id = $2 AND preview_id = $3",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .bind(preview_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?
    .rows_affected();
    if removed == 0 {
        return Err(ToolError::new(
            ErrorCode::PREVIEW_NOT_FOUND,
            format!("{cs} has no item {preview_id}."),
        ));
    }
    sqlx::query(
        "UPDATE platform.preview SET preview_state = 'superseded', updated_at = now()
         WHERE tenant_id = $1 AND id = $2 AND preview_state = 'open'",
    )
    .bind(cx.tenant.as_str())
    .bind(preview_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    set_state(
        &mut tx,
        cs,
        ChangeSetState::Drafting,
        head.submitted_at.is_some(),
    )
    .await?;
    event(
        &mut tx,
        cs,
        ActionType::DELETE.0,
        ActionStatus::Completed,
        Some(state),
        ChangeSetState::Drafting,
        Some(json!({"preview_id": preview_id})),
        None,
    )
    .await?;
    let out = summary(&mut tx, cs).await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(out)
}

/// `PUT /v1/platform/change-sets/{id}/items`: the items as edited by talking. The previews left
/// out become `superseded`; the set goes to drafting with version + 1 (unless nothing changed).
pub async fn replace_items(cx: &Cx, cs: &str, items: Vec<ItemRef>) -> Result<Value, ToolError> {
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let head = lock(&mut tx, cs).await?;
    let state = head.state();
    if !matches!(
        state,
        ChangeSetState::Drafting | ChangeSetState::WaitingForApproval
    ) {
        return Err(invalid_state(cs, state, "its items can no longer change"));
    }
    let current: Vec<(String, Vec<String>)> = sqlx::query_as(
        "SELECT preview_id, depends_on FROM platform.change_set_item
         WHERE tenant_id = $1 AND change_set_id = $2 ORDER BY position",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    let same = current.len() == items.len()
        && current
            .iter()
            .zip(&items)
            .all(|((p, d), i)| *p == i.preview_id && i.depends_on.as_ref().is_none_or(|x| x == d));
    if same {
        let out = summary(&mut tx, cs).await?;
        tx.commit().await.map_err(ToolError::from)?;
        return Ok(
            json!({"id": cs, "version": out["version"], "state": out["state"], "superseded": []}),
        );
    }
    // Check the new previews before touching anything.
    let mut inputs = Vec::with_capacity(items.len());
    for i in &items {
        let p = preview::load(&mut tx, &i.preview_id).await?;
        if p.change_set_id.as_deref().is_some_and(|o| o != cs) {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("Preview {} belongs to another change set.", p.id),
            ));
        }
        ensure_open(&p, cx.clock.now())?;
        inputs.push(p.input);
    }
    let keep: BTreeSet<&str> = items.iter().map(|i| i.preview_id.as_str()).collect();
    let superseded: Vec<String> = current
        .iter()
        .map(|(p, _)| p.clone())
        .filter(|p| !keep.contains(p.as_str()))
        .collect();
    sqlx::query(
        "UPDATE platform.preview SET preview_state = 'superseded', updated_at = now()
         WHERE tenant_id = $1 AND id = ANY($2) AND preview_state = 'open'",
    )
    .bind(cx.tenant.as_str())
    .bind(&superseded)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    sqlx::query("DELETE FROM platform.change_set_item WHERE tenant_id = $1 AND change_set_id = $2")
        .bind(cx.tenant.as_str())
        .bind(cs)
        .execute(&mut *tx.tx)
        .await
        .map_err(db_err)?;
    for (n, (i, input)) in items.iter().zip(&inputs).enumerate() {
        let position = i32::try_from(n + 1).unwrap_or(i32::MAX);
        let depends_on = match &i.depends_on {
            Some(d) => d.clone(),
            None => bind::positions(input)
                .into_iter()
                .filter_map(|k| items.get(usize::try_from(k - 1).ok()?))
                .map(|r| r.preview_id.clone())
                .collect(),
        };
        sqlx::query(
            "UPDATE platform.preview SET change_set_id = $3, expires_at = NULL, updated_at = now()
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(cx.tenant.as_str())
        .bind(&i.preview_id)
        .bind(cs)
        .execute(&mut *tx.tx)
        .await
        .map_err(db_err)?;
        sqlx::query(
            "INSERT INTO platform.change_set_item (tenant_id, change_set_id, position, preview_id,
               depends_on) VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(cx.tenant.as_str())
        .bind(cs)
        .bind(position)
        .bind(&i.preview_id)
        .bind(&depends_on)
        .execute(&mut *tx.tx)
        .await
        .map_err(db_err)?;
    }
    set_state(&mut tx, cs, ChangeSetState::Drafting, true).await?;
    event(
        &mut tx,
        cs,
        ActionType::UPDATE.0,
        ActionStatus::Completed,
        Some(state),
        ChangeSetState::Drafting,
        Some(json!({"items": keep, "superseded": superseded})),
        None,
    )
    .await?;
    let out = summary(&mut tx, cs).await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(json!({"id": cs, "version": out["version"], "state": "drafting", "superseded": superseded}))
}

/// `PATCH /v1/platform/change-sets/{id}`: what the person asked, as it grows by talking. A
/// correction ("Drop the low-stock ones.") joins the request text and the interpretation is
/// replaced; the items change through `replace_items`, which carries the version.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestUpdate {
    #[serde(default)]
    pub title: Option<String>,
    /// One more sentence from the person, appended to `request_text`.
    #[serde(default)]
    pub append_request_text: Option<String>,
    #[serde(default)]
    pub interpretation: Option<Vec<Value>>,
}

pub async fn update_request(cx: &Cx, cs: &str, update: RequestUpdate) -> Result<Value, ToolError> {
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let head = lock(&mut tx, cs).await?;
    let state = head.state();
    if !matches!(
        state,
        ChangeSetState::Drafting | ChangeSetState::WaitingForApproval
    ) {
        return Err(invalid_state(cs, state, "its request can no longer change"));
    }
    let title = update
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let sentence = update
        .append_request_text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    sqlx::query(
        "UPDATE platform.change_set SET
           title = coalesce($3, title),
           request_text = CASE WHEN $4::text IS NULL THEN request_text
                               ELSE array_append(request_text, $4) END,
           interpretation = coalesce($5, interpretation),
           updated_at = now()
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .bind(title)
    .bind(sentence)
    .bind(update.interpretation.map(Value::Array))
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    event(
        &mut tx,
        cs,
        ActionType::UPDATE.0,
        ActionStatus::Completed,
        Some(state),
        state,
        Some(json!({"request_updated": true})),
        None,
    )
    .await?;
    let out = summary(&mut tx, cs).await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(out)
}

/// `POST /v1/platform/change-sets/{id}/submit {version}`.
pub async fn submit(cx: &Cx, cs: &str, version: i32) -> Result<Value, ToolError> {
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let head = lock(&mut tx, cs).await?;
    let state = head.state();
    if state != ChangeSetState::Drafting {
        return Err(invalid_state(
            cs,
            state,
            "only a drafting set can be submitted",
        ));
    }
    if head.version != version {
        return Err(ToolError::new(
            ErrorCode::VERSION_OUT_OF_DATE,
            "A newer version of this change set exists.",
        )
        .detail(json!({"latest_version": head.version})));
    }
    let (items, closed): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE p.preview_state <> 'open')
         FROM platform.change_set_item i
         JOIN platform.preview p ON p.tenant_id = i.tenant_id AND p.id = i.preview_id
         WHERE i.tenant_id = $1 AND i.change_set_id = $2",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    if items == 0 {
        return Err(invalid_state(
            cs,
            state,
            "add at least one item before submitting",
        ));
    }
    if closed > 0 {
        return Err(ToolError::new(
            ErrorCode::PREVIEW_EXPIRED,
            "An item's preview is no longer open; preview it again.",
        ));
    }
    sqlx::query(
        "UPDATE platform.change_set SET change_set_state = 'waiting_for_approval',
           submitted_at = $3, updated_at = now() WHERE tenant_id = $1 AND id = $2",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .bind(cx.clock.now())
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    event(
        &mut tx,
        cs,
        ActionType::SEND.0,
        ActionStatus::Completed,
        Some(state),
        ChangeSetState::WaitingForApproval,
        None,
        None,
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(json!({"id": cs, "version": version, "state": "waiting_for_approval"}))
}

// ---------------------------------------------------------------------------------------------
// Person-only endpoints (`human:*`)

/// `POST /v1/platform/change-sets/{id}/reject {reason}`.
pub async fn reject(cx: &Cx, cs: &str, reason: &str) -> Result<Value, ToolError> {
    let reason = reason.trim();
    if reason.is_empty() || reason.chars().count() > 500 {
        return Err(
            ToolError::invalid_input("Give a reason of 1–500 characters.")
                .detail(json!({"field": "reason", "reason": "range"})),
        );
    }
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    reject_in(&mut tx, cs, reason, false).await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(json!({"ok": true, "id": cs, "state": "rejected"}))
}

/// Rejects inside the caller's transaction. `drafting_too` also closes a set still being
/// drafted (a dismissed task takes its draft with it). Already rejected is not an error.
pub async fn reject_in(
    tx: &mut TxCx,
    cs: &str,
    reason: &str,
    drafting_too: bool,
) -> Result<(), ToolError> {
    let head = lock(tx, cs).await?;
    let state = head.state();
    if state == ChangeSetState::Rejected {
        return Ok(());
    }
    let allowed = state == ChangeSetState::WaitingForApproval
        || (drafting_too && state == ChangeSetState::Drafting);
    if !allowed {
        return Err(invalid_state(
            cs,
            state,
            "only a set waiting for approval can be rejected",
        ));
    }
    sqlx::query(
        "UPDATE platform.change_set SET change_set_state = 'rejected', rejected_at = $3,
           rejection_reason = $4, updated_at = now() WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(cs)
    .bind(tx.clock.now())
    .bind(reason)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    sqlx::query(
        "UPDATE platform.preview SET preview_state = 'expired', updated_at = now()
         WHERE tenant_id = $1 AND change_set_id = $2 AND preview_state = 'open'",
    )
    .bind(tx.tenant.as_str())
    .bind(cs)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    event(
        tx,
        cs,
        ActionType::REJECT.0,
        ActionStatus::Completed,
        Some(state),
        ChangeSetState::Rejected,
        Some(json!({"reason": reason})),
        None,
    )
    .await?;
    log_action(
        tx,
        LogEntry {
            object: Some(("change_set", cs)),
            result: Some(json!({"reason": reason})),
            change_set_id: Some(cs),
            ..LogEntry::new(
                ActionType::REJECT.0,
                ActionStatus::Completed,
                "reject_change_set",
            )
        },
    )
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct Item {
    pub position: i32,
    pub preview_id: String,
    pub instrument: String,
    pub input: Value,
    pub plan: Value,
    pub changes: Value,
    pub needs_approval: bool,
    pub result: Option<Value>,
}

pub(crate) async fn items(tx: &mut TxCx, cs: &str) -> Result<Vec<Item>, ToolError> {
    sqlx::query_as::<_, Item>(
        "SELECT i.position, p.id AS preview_id, p.instrument, p.input, p.plan, p.changes,
                p.needs_approval, p.result
         FROM platform.change_set_item i
         JOIN platform.preview p ON p.tenant_id = i.tenant_id AND p.id = i.preview_id
         WHERE i.tenant_id = $1 AND i.change_set_id = $2 ORDER BY i.position",
    )
    .bind(tx.tenant.as_str())
    .bind(cs)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db_err)
}

/// The `approver_role`s the approval thresholds name for these tools.
fn required_roles(cx: &Cx, tools: &[&str]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let thresholds = cx
        .policy
        .raw("approval")
        .and_then(|p| p.get("thresholds"))
        .and_then(Value::as_object);
    for t in thresholds.into_iter().flat_map(|m| m.values()) {
        let applies = t["applies_to"]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| tools.contains(&x.as_str().unwrap_or(""))));
        if applies && let Some(role) = t["approver_role"].as_str() {
            out.insert(role.to_owned());
        }
    }
    out
}

async fn held_roles(tx: &mut TxCx) -> Result<BTreeSet<String>, ToolError> {
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT unnest(r.approver_roles) FROM platform.admin_user_role ur
         JOIN platform.role r ON r.tenant_id = ur.tenant_id AND r.id = ur.role_id
         WHERE ur.tenant_id = $1 AND ur.admin_user_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(&tx.principal.subject)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    Ok(roles.into_iter().collect())
}

/// Every `valid_through` in a value (the end of a sale, a coupon, a banner booking).
fn period_ends(v: &Value, time: &StoreTime, out: &mut Vec<DateTime<Utc>>) {
    match v {
        Value::Object(o) => {
            for (k, x) in o {
                if k == "valid_through"
                    && let Some(t) = x.as_str().and_then(|s| time.parse(s).ok())
                {
                    out.push(t);
                }
                period_ends(x, time, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| period_ends(x, time, out)),
        _ => {}
    }
}

/// 23:59 store time, `days` after the day of `at`.
pub(crate) fn end_of_day(time: &StoreTime, at: DateTime<Utc>, days: u64) -> DateTime<Utc> {
    let day = at.with_timezone(&time.zone).date_naive() + Days::new(days);
    time.parse(&format!("{day}T23:59:00")).unwrap_or(at)
}

fn window_days(cx: &Cx, key: &str) -> u64 {
    cx.policy
        .raw("approval")
        .and_then(|p| p.get("change_set"))
        .and_then(|c| c.get(key))
        .and_then(Value::as_u64)
        .unwrap_or(1)
}

/// The undo deadline: the latest period end + N days, else the run day + N days, at 23:59 store
/// time. `None` when an item cannot be taken back.
pub(crate) fn undo_deadline<'a>(
    cx: &Cx,
    items: impl IntoIterator<Item = (&'a str, &'a Value)>,
    reversible: impl Fn(&str) -> bool,
    ran_at: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let mut ends = Vec::new();
    for (tool, input) in items {
        if irreversible(cx, tool) || !reversible(tool) {
            return None;
        }
        period_ends(input, &cx.time, &mut ends);
    }
    Some(match ends.into_iter().max() {
        Some(end) => end_of_day(&cx.time, end, window_days(cx, "undo_window_after_end_days")),
        None => end_of_day(
            &cx.time,
            ran_at,
            window_days(cx, "undo_window_after_run_days"),
        ),
    })
}

/// Back to drafting with version + 1 because approval found something different.
async fn back_to_drafting(
    cx: &Cx,
    cs: &str,
    detail: Value,
    error: ToolError,
) -> Result<Value, ToolError> {
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    lock(&mut tx, cs).await?;
    set_state(&mut tx, cs, ChangeSetState::Drafting, true).await?;
    sqlx::query(
        "UPDATE platform.change_set SET submitted_at = NULL WHERE tenant_id = $1 AND id = $2",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    event(
        &mut tx,
        cs,
        ActionType::ASSESS.0,
        ActionStatus::Failed,
        Some(ChangeSetState::WaitingForApproval),
        ChangeSetState::Drafting,
        Some(detail.clone()),
        Some(&error),
    )
    .await?;
    let version: i32 = sqlx::query_scalar(
        "SELECT version FROM platform.change_set WHERE tenant_id = $1 AND id = $2",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    tx.commit().await.map_err(ToolError::from)?;
    let mut d = detail;
    d["new_version"] = json!(version);
    Err(error.detail(d))
}

/// `POST /v1/platform/change-sets/{id}/approve {version}`: checks, runs, verifies.
///
/// Returns the final state (`live` or `failed`): the runner works inside this request, so the
/// person sees the outcome on the same card.
pub async fn approve(
    registry: &Registry<Cx>,
    cx: &Cx,
    cs: &str,
    version: i32,
) -> Result<Value, ToolError> {
    let runner = cx.as_system(RUNNER);
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let head = lock(&mut tx, cs).await?;
    let state = head.state();
    if state != ChangeSetState::WaitingForApproval {
        return Err(invalid_state(
            cs,
            state,
            "only a set waiting for approval can be approved",
        ));
    }
    if head.version != version {
        return Err(ToolError::new(
            ErrorCode::VERSION_OUT_OF_DATE,
            "A newer version of this change set exists; review it again.",
        )
        .detail(json!({"latest_version": head.version})));
    }
    let list = items(&mut tx, cs).await?;
    let gated: Vec<&str> = list
        .iter()
        .filter(|i| i.needs_approval)
        .map(|i| i.instrument.as_str())
        .collect();
    let required = required_roles(cx, &gated);
    let held = held_roles(&mut tx).await?;
    let missing: Vec<&String> = required.difference(&held).collect();
    if !missing.is_empty() {
        return Err(ToolError::new(
            ErrorCode::FORBIDDEN,
            format!(
                "Approving this set needs the {} approver role.",
                missing
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )
        .detail(json!({"reason": "role", "required": missing})));
    }
    tx.commit().await.map_err(ToolError::from)?;

    // The person approves exactly what will run: preview every item again.
    let revalidate = cx
        .policy
        .raw("approval")
        .and_then(|p| p.pointer("/change_set/revalidate_on_approve"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if revalidate {
        for item in &list {
            let again = op(registry, &item.instrument)?
                .preview(&runner, item.input.clone())
                .await?;
            if !crate::same_changes(&json!(again.inner.changes), &item.changes) {
                return back_to_drafting(
                    cx,
                    cs,
                    json!({"preview_id": item.preview_id, "changes": again.inner.changes}),
                    ToolError::new(
                        ErrorCode::ESTIMATE_CHANGED,
                        format!(
                            "The store changed since {} was previewed. The set is back to drafting; review the new version.",
                            item.instrument
                        ),
                    ),
                )
                .await;
            }
            if let Some(failed) = again.inner.checks.iter().find(|c| !c.passed) {
                return back_to_drafting(
                    cx,
                    cs,
                    json!({"preview_id": item.preview_id, "rule": failed.rule, "checks": again.inner.checks}),
                    ToolError::new(
                        ErrorCode::POLICY_VIOLATION,
                        format!(
                            "{} fails the `{}` check: {}",
                            item.instrument,
                            failed.rule,
                            failed.detail.as_deref().unwrap_or("see the checks")
                        ),
                    ),
                )
                .await;
            }
        }
    }

    let approver = cx.principal.subject.clone();
    start(
        registry,
        cx,
        &runner,
        cs,
        version,
        &list,
        Some(&approver),
        json!({"version": version}),
    )
    .await
}

/// waiting_for_approval → running (only if nobody else got there first), then the runner.
/// `approved_by` is the person, or `None` under a standing approval.
#[allow(clippy::too_many_arguments)]
async fn start(
    registry: &Registry<Cx>,
    cx: &Cx,
    runner: &Cx,
    cs: &str,
    version: i32,
    list: &[Item],
    approved_by: Option<&str>,
    authorization: Value,
) -> Result<Value, ToolError> {
    let now = cx.clock.now();
    let undo_until = undo_deadline(
        cx,
        list.iter().map(|i| (i.instrument.as_str(), &i.input)),
        |t| {
            registry
                .get(t)
                .is_some_and(|o| o.undo_support() == UndoSupport::Supported)
        },
        now,
    );
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let moved = sqlx::query(
        "UPDATE platform.change_set SET change_set_state = 'running', approved_by = $3,
           approved_at = $4, undo_until = $5, updated_at = now()
         WHERE tenant_id = $1 AND id = $2 AND change_set_state = 'waiting_for_approval'
           AND version = $6",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .bind(approved_by)
    .bind(now)
    .bind(undo_until)
    .bind(version)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?
    .rows_affected();
    if moved == 0 {
        return Err(ToolError::new(
            ErrorCode::VERSION_OUT_OF_DATE,
            "The change set changed while it was being approved; review it again.",
        ));
    }
    event(
        &mut tx,
        cs,
        ActionType::AUTHORIZE.0,
        ActionStatus::Completed,
        Some(ChangeSetState::WaitingForApproval),
        ChangeSetState::Running,
        Some(authorization.clone()),
        None,
    )
    .await?;
    log_action(
        &mut tx,
        LogEntry {
            object: Some(("change_set", cs)),
            result: Some(authorization),
            change_set_id: Some(cs),
            ..LogEntry::new(
                ActionType::AUTHORIZE.0,
                ActionStatus::Completed,
                "approve_change_set",
            )
        },
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;

    let outcome = run(registry, cx, runner, cs, list).await?;
    let mut out = json!({
        "ok": true, "id": cs, "version": version,
        "approved_by": approved_by, "approved_at": cx.time.format(now),
        "undo_until": undo_until.map(|u| cx.time.format(u)),
    });
    if let (Value::Object(o), Value::Object(r)) = (&mut out, outcome) {
        o.extend(r);
    }
    Ok(out)
}

/// Runs a submitted set the store approved in advance: a watcher's fix under
/// `platform.yml watchers.<key>.run_fix: standing_approval` (`policy_key`, without the file
/// name). Nobody approves it here; the approval is the policy override, which itself went
/// through a person. The set records `approved_by: null` and the policy key, and keeps its undo
/// deadline.
pub async fn run_with_standing_approval(
    registry: &Registry<Cx>,
    cx: &Cx,
    cs: &str,
    policy_key: &str,
) -> Result<Value, ToolError> {
    let standing = cx
        .policy
        .lookup(&format!("platform.{policy_key}"))
        .and_then(Value::as_str)
        .is_some_and(|v| v == "standing_approval");
    if !standing {
        return Err(ToolError::new(
            ErrorCode::APPROVAL_REQUIRED,
            format!("The store's policy gives {cs} no standing approval; a person approves it."),
        )
        .detail(json!({"change_set_id": cs, "state": "waiting_for_approval"})));
    }
    let runner = cx.as_system(RUNNER);
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let head = lock(&mut tx, cs).await?;
    let state = head.state();
    if state != ChangeSetState::WaitingForApproval {
        return Err(invalid_state(
            cs,
            state,
            "only a submitted set can run under a standing approval",
        ));
    }
    let list = items(&mut tx, cs).await?;
    tx.commit().await.map_err(ToolError::from)?;
    start(
        registry,
        cx,
        &runner,
        cs,
        head.version,
        &list,
        None,
        json!({"version": head.version, "standing_approval": policy_key}),
    )
    .await
}

/// The runner: commits every item in order, binding placeholders; on a failure takes back what
/// ran, in reverse. Then reads every change back.
async fn run(
    registry: &Registry<Cx>,
    cx: &Cx,
    runner: &Cx,
    cs: &str,
    list: &[Item],
) -> Result<Value, ToolError> {
    let mut produced: BTreeMap<i32, Map<String, Value>> = BTreeMap::new();
    let mut done: Vec<(&Item, Value, Committed)> = Vec::new();
    for item in list {
        match run_item(registry, runner, cs, item, &produced).await {
            Ok((plan, c)) => {
                produced.insert(item.position, c.produced.clone());
                done.push((item, plan, c));
            }
            Err(e) => {
                let mut taken_back = Vec::new();
                for (prev, plan, c) in done.iter().rev() {
                    let o = op(registry, &prev.instrument)?;
                    if o.undo_support() != UndoSupport::Supported {
                        continue;
                    }
                    let mut tx = runner.begin().await.map_err(ToolError::from)?;
                    if o.undo(&mut tx, plan, c).await.is_ok() {
                        tx.commit().await.map_err(ToolError::from)?;
                        taken_back.push(prev.preview_id.clone());
                    }
                }
                let mut tx = runner.begin().await.map_err(ToolError::from)?;
                set_state(&mut tx, cs, ChangeSetState::Failed, false).await?;
                let detail = json!({"failed_item": item.preview_id, "taken_back": taken_back,
                                    "error": {"code": e.code, "message": e.message}});
                event(
                    &mut tx,
                    cs,
                    ActionType::UPDATE.0,
                    ActionStatus::Failed,
                    Some(ChangeSetState::Running),
                    ChangeSetState::Failed,
                    Some(detail.clone()),
                    Some(&e),
                )
                .await?;
                tx.commit().await.map_err(ToolError::from)?;
                return Ok(json!({"state": "failed", "failure": detail}));
            }
        }
    }

    let mut tx = runner.begin().await.map_err(ToolError::from)?;
    set_state(&mut tx, cs, ChangeSetState::Verifying, false).await?;
    event(
        &mut tx,
        cs,
        ActionType::UPDATE.0,
        ActionStatus::Completed,
        Some(ChangeSetState::Running),
        ChangeSetState::Verifying,
        None,
        None,
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;

    let mut results = Vec::new();
    let mut all_matched = true;
    for (item, plan, c) in &done {
        let v = op(registry, &item.instrument)?
            .verify(runner, plan, c)
            .await
            .unwrap_or_else(|e| acom_contract::Verification {
                matched: false,
                mismatches: vec![format!("could not read back: {}", e.message)],
            });
        all_matched &= v.matched;
        let entry = json!({"preview_id": item.preview_id, "tool": item.instrument,
                           "result_id": c.id, "matched": v.matched, "mismatches": v.mismatches});
        let mut tx = runner.begin().await.map_err(ToolError::from)?;
        sqlx::query(
            "UPDATE platform.change_set_item SET verification = $4, updated_at = now()
             WHERE tenant_id = $1 AND change_set_id = $2 AND preview_id = $3",
        )
        .bind(cx.tenant.as_str())
        .bind(cs)
        .bind(&item.preview_id)
        .bind(&entry)
        .execute(&mut *tx.tx)
        .await
        .map_err(db_err)?;
        tx.commit().await.map_err(ToolError::from)?;
        results.push(entry);
    }
    let end = if all_matched {
        ChangeSetState::Live
    } else {
        ChangeSetState::Failed
    };
    let matched = results.iter().filter(|r| r["matched"] == true).count();
    let verification = json!({"state": end.as_str(), "matched": matched, "total": results.len(),
                              "items": results});
    let mut tx = runner.begin().await.map_err(ToolError::from)?;
    sqlx::query(
        "UPDATE platform.change_set SET change_set_state = $3, verified_at = $4,
           verification = $5, updated_at = now() WHERE tenant_id = $1 AND id = $2",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .bind(end.as_str())
    .bind(runner.clock.now())
    .bind(&verification)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    event(
        &mut tx,
        cs,
        ActionType::ASSESS.0,
        if all_matched {
            ActionStatus::Completed
        } else {
            ActionStatus::Failed
        },
        Some(ChangeSetState::Verifying),
        end,
        Some(json!({"matched": matched, "total": results.len()})),
        None,
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(json!({"state": end.as_str(), "verification": verification}))
}

/// Commits one item in its own transaction. Running it again returns the first result.
async fn run_item(
    registry: &Registry<Cx>,
    runner: &Cx,
    cs: &str,
    item: &Item,
    produced: &BTreeMap<i32, Map<String, Value>>,
) -> Result<(Value, Committed), ToolError> {
    let o = op(registry, &item.instrument)?;
    let mut tx = runner.begin().await.map_err(ToolError::from)?;
    let p = preview::load(&mut tx, &item.preview_id).await?;
    if p.preview_state == PreviewState::Committed.as_str() {
        return Ok((p.plan.clone(), first_result(&p)?));
    }
    tx.preview_id = Some(item.preview_id.clone());
    tx.change_set_id = Some(cs.to_owned());
    let plan = bind::bind(&item.plan, produced)
        .map_err(|e| ToolError::new(ErrorCode::INTERNAL, format!("cannot bind the plan: {e}")))?;
    let done = o.commit(&mut tx, &plan).await?;
    mark_committed(&mut tx, &item.preview_id, &plan, &done).await?;
    sqlx::query(
        "UPDATE platform.change_set_item SET result_id = $4, updated_at = now()
         WHERE tenant_id = $1 AND change_set_id = $2 AND preview_id = $3",
    )
    .bind(runner.tenant.as_str())
    .bind(cs)
    .bind(&item.preview_id)
    .bind(&done.id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    log_action(
        &mut tx,
        LogEntry {
            object: crate::main_object(&item.changes, &done.id),
            result: Some(json!({"id": done.id, "produced": done.produced})),
            change_set_id: Some(cs),
            preview_id: Some(&item.preview_id),
            ..LogEntry::new(
                o.meta().action_type.0,
                ActionStatus::Completed,
                o.meta().tool,
            )
        },
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok((plan, done))
}

/// `POST /v1/platform/change-sets/{id}/undo`: every item, in reverse order, before the deadline.
pub async fn undo(registry: &Registry<Cx>, cx: &Cx, cs: &str) -> Result<Value, ToolError> {
    let runner = cx.as_system(RUNNER);
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    let head = lock(&mut tx, cs).await?;
    let state = head.state();
    if state == ChangeSetState::Undone {
        let list = items(&mut tx, cs).await?;
        return Ok(
            json!({"ok": true, "id": cs, "state": "undone", "undone_items": list.len(),
                         "undone_at": head.undone_at.map(|t| cx.time.format(t))}),
        );
    }
    if state != ChangeSetState::Live {
        return Err(invalid_state(cs, state, "only a live set can be undone"));
    }
    let list = items(&mut tx, cs).await?;
    tx.commit().await.map_err(ToolError::from)?;
    if let Some(item) = list.iter().find(|i| {
        irreversible(cx, &i.instrument)
            || registry
                .get(&i.instrument)
                .is_none_or(|o| o.undo_support() != UndoSupport::Supported)
    }) {
        return Err(ToolError::new(
            IRREVERSIBLE_CHANGE,
            format!(
                "{} cannot be taken back (a message sent or money paid), so this set has no undo.",
                item.instrument
            ),
        ));
    }
    let now = cx.clock.now();
    match head.undo_until {
        Some(until) if until < now => {
            return Err(ToolError::new(
                ErrorCode::UNDO_WINDOW_CLOSED,
                "The undo deadline for this change set has passed.",
            )
            .detail(json!({"until": cx.time.format(until)})));
        }
        None => {
            return Err(ToolError::new(
                ErrorCode::UNDO_WINDOW_CLOSED,
                "This change set has no undo deadline, so it cannot be undone.",
            ));
        }
        _ => {}
    }
    let mut undone = 0usize;
    for item in list.iter().rev() {
        let o = op(registry, &item.instrument)?;
        let done: Committed = item
            .result
            .clone()
            .and_then(|r| serde_json::from_value(r).ok())
            .ok_or_else(|| ToolError::new(ErrorCode::INTERNAL, "item has no recorded result"))?;
        let mut tx = runner.begin().await.map_err(ToolError::from)?;
        if let Err(e) = o.undo(&mut tx, &item.plan, &done).await {
            drop(tx);
            let mut tx = cx.begin().await.map_err(ToolError::from)?;
            set_state(&mut tx, cs, ChangeSetState::Failed, false).await?;
            event(
                &mut tx,
                cs,
                ActionType::UPDATE.0,
                ActionStatus::Failed,
                Some(state),
                ChangeSetState::Failed,
                Some(json!({"undone_items": undone, "failed_item": item.preview_id})),
                Some(&e),
            )
            .await?;
            tx.commit().await.map_err(ToolError::from)?;
            return Err(e);
        }
        tx.commit().await.map_err(ToolError::from)?;
        undone += 1;
    }
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    sqlx::query(
        "UPDATE platform.change_set SET change_set_state = 'undone', undone_at = $3,
           updated_at = now() WHERE tenant_id = $1 AND id = $2",
    )
    .bind(cx.tenant.as_str())
    .bind(cs)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db_err)?;
    event(
        &mut tx,
        cs,
        ActionType::UPDATE.0,
        ActionStatus::Completed,
        Some(state),
        ChangeSetState::Undone,
        Some(json!({"undone_items": undone})),
        None,
    )
    .await?;
    log_action(
        &mut tx,
        LogEntry {
            object: Some(("change_set", cs)),
            result: Some(json!({"undone_items": undone})),
            change_set_id: Some(cs),
            ..LogEntry::new(
                ActionType::UPDATE.0,
                ActionStatus::Completed,
                "undo_change_set",
            )
        },
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(
        json!({"ok": true, "id": cs, "state": "undone", "undone_items": undone,
              "undone_at": cx.time.format(now)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_deadline_is_end_of_day_in_store_time() {
        let st = StoreTime::new(chrono_tz::America::New_York);
        let end = st.parse("2026-11-08T23:59:00").unwrap();
        assert_eq!(
            st.format(end_of_day(&st, end, 1)),
            "2026-11-09T23:59:00-05:00"
        );
        let ran = st.parse("2026-11-06T11:08:30").unwrap();
        assert_eq!(
            st.format(end_of_day(&st, ran, 1)),
            "2026-11-07T23:59:00-05:00"
        );
    }

    #[test]
    fn finds_period_ends_anywhere_in_an_input() {
        let st = StoreTime::new(chrono_tz::America::New_York);
        let input = json!({"valid_through": "2026-11-08T23:59", "booking": {"valid_through": "2026-11-09T12:00"}});
        let mut ends = Vec::new();
        period_ends(&input, &st, &mut ends);
        assert_eq!(ends.len(), 2);
        assert_eq!(
            st.format(*ends.iter().max().unwrap()),
            "2026-11-09T12:00:00-05:00"
        );
    }
}
