//! The `popup` resource (docs/api/merchandising.md#popup): store popups (`acom:Popup`, a kind of
//! `WebPageElement`, `pop_`) that open over pages for a period. What its operations share: the
//! row, its JSON, and the SQL.
//!
//! Like bookings, whether a popup opens is computed from its period when a page asks; nothing
//! switches it on or off.

pub mod list_popups;
pub mod set_popup;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_store::{Cx, Page, TxCx};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::display_status;
use crate::repo::db;

/// More popups would open at once on a page than `popup.max_concurrent_per_page` allows.
pub const TOO_MANY_POPUPS: ErrorCode = ErrorCode::new("too_many_popups", ErrorClass::LimitExceeded);
/// Another popup of the page opens at the same time with the same priority
/// (`display.priority_unique`).
pub const PRIORITY_CONFLICT: ErrorCode =
    ErrorCode::new("priority_conflict", ErrorClass::PolicyViolation);

/// A popup as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PopupRow {
    pub id: String,
    pub name: String,
    pub popup_type: String,
    pub text: Option<String>,
    pub image: Option<String>,
    pub url: Option<String>,
    pub page: Vec<String>,
    pub priority: i32,
    pub suppress_days: Option<i32>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
    #[serde(skip)]
    pub version: i32,
}

const COLUMNS: &str = "id, name, popup_type, text, image, url, page, priority, suppress_days,
    valid_from, valid_through, version";

/// A popup as people and pages see it.
pub(crate) fn popup_json(cx: &Cx, p: &PopupRow, at: DateTime<Utc>) -> Value {
    json!({
        "id": p.id,
        "name": p.name,
        "popup_type": p.popup_type,
        "pages": p.page,
        "priority": p.priority,
        "suppress_days": p.suppress_days,
        "valid_from": cx.time.format(p.valid_from),
        "valid_through": cx.time.format(p.valid_through),
        "status": display_status("scheduled", p.valid_from, Some(p.valid_through), at),
        "content": {"text": p.text, "image_url": p.image, "url": p.url},
    })
}

pub(crate) async fn popup(tx: &mut TxCx, id: &str) -> Result<Option<PopupRow>, ToolError> {
    sqlx::query_as::<_, PopupRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM merchandising.popup WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Popups of a kind, on a page, open at a moment, or all; by start, then priority.
pub(crate) async fn popups(
    tx: &mut TxCx,
    popup_type: Option<&str>,
    page_word: Option<&str>,
    open_at: Option<DateTime<Utc>>,
    page: Page,
) -> Result<(i64, Vec<PopupRow>), ToolError> {
    let filter = "tenant_id = $1 AND ($2::text IS NULL OR popup_type = $2)
         AND ($3::text IS NULL OR $3 = ANY(page))
         AND ($4::timestamptz IS NULL
              OR (valid_from <= $4 AND valid_through + interval '1 minute' > $4))";
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM merchandising.popup WHERE {filter}"
    )))
    .bind(tx.tenant.as_str())
    .bind(popup_type)
    .bind(page_word)
    .bind(open_at)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, PopupRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM merchandising.popup WHERE {filter}
         ORDER BY valid_from, priority, id LIMIT $5 OFFSET $6"
    )))
    .bind(tx.tenant.as_str())
    .bind(popup_type)
    .bind(page_word)
    .bind(open_at)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

/// Other popups that open on any of the pages during `from`–`through`.
pub(crate) async fn popups_meeting(
    tx: &mut TxCx,
    except: &str,
    pages: &[String],
    from: DateTime<Utc>,
    through: DateTime<Utc>,
) -> Result<Vec<PopupRow>, ToolError> {
    sqlx::query_as::<_, PopupRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM merchandising.popup
         WHERE tenant_id = $1 AND id <> $2 AND page && $3
           AND valid_from <= $5 AND valid_through >= $4
         ORDER BY valid_from, priority, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(except)
    .bind(pages)
    .bind(from)
    .bind(through)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The ids among `ids` that are popups already.
pub(crate) async fn taken_ids(tx: &mut TxCx, ids: &[String]) -> Result<Vec<String>, ToolError> {
    sqlx::query_scalar("SELECT id FROM merchandising.popup WHERE tenant_id = $1 AND id = ANY($2)")
        .bind(tx.tenant.as_str())
        .bind(ids)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)
}

/// Writes a popup, new or changed; only touches an existing row (and its version) when
/// something differs.
pub(crate) async fn save(tx: &mut TxCx, p: &PopupRow) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO merchandising.popup (tenant_id, id, name, popup_type, text, image, url, page,
           priority, suppress_days, valid_from, valid_through)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (tenant_id, id) DO UPDATE SET name = EXCLUDED.name,
           popup_type = EXCLUDED.popup_type, text = EXCLUDED.text, image = EXCLUDED.image,
           url = EXCLUDED.url, page = EXCLUDED.page, priority = EXCLUDED.priority,
           suppress_days = EXCLUDED.suppress_days, valid_from = EXCLUDED.valid_from,
           valid_through = EXCLUDED.valid_through, updated_at = now(),
           version = popup.version + 1
         WHERE (popup.name, popup.popup_type, popup.text, popup.image, popup.url, popup.page,
                popup.priority, popup.suppress_days, popup.valid_from, popup.valid_through)
           IS DISTINCT FROM
               (EXCLUDED.name, EXCLUDED.popup_type, EXCLUDED.text, EXCLUDED.image, EXCLUDED.url,
                EXCLUDED.page, EXCLUDED.priority, EXCLUDED.suppress_days, EXCLUDED.valid_from,
                EXCLUDED.valid_through)",
    )
    .bind(tx.tenant.as_str())
    .bind(&p.id)
    .bind(&p.name)
    .bind(&p.popup_type)
    .bind(&p.text)
    .bind(&p.image)
    .bind(&p.url)
    .bind(&p.page)
    .bind(p.priority)
    .bind(p.suppress_days)
    .bind(p.valid_from)
    .bind(p.valid_through)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Removes a popup (undoing one that never opened).
pub(crate) async fn delete(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query("DELETE FROM merchandising.popup WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}
