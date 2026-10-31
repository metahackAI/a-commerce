//! The `menu` resource: the back-office menu (`platform.site_navigation_element`, API `menu`) and
//! each operator's favorites (`platform.menu_favorite`).

pub mod list_menus;
pub mod set_favorite;
pub mod set_menu;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_store::TxCx;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::repo::db;

/// No menu entry with that id.
pub const MENU_NOT_FOUND: ErrorCode = ErrorCode::new("menu_not_found", ErrorClass::NotFound);

/// One menu entry as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub(crate) struct MenuRow {
    pub id: String,
    pub name: String,
    pub url: String,
    pub required_scope: String,
    pub position: i32,
    pub is_visible: bool,
    pub is_part_of_id: Option<String>,
    #[serde(skip)]
    pub version: i32,
}

pub(crate) const COLUMNS: &str =
    "id, name, url, required_scope, position, is_visible, is_part_of_id, version";

pub(crate) async fn menu(tx: &mut TxCx, id: &str) -> Result<Option<MenuRow>, ToolError> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM platform.site_navigation_element WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) fn not_found(id: &str) -> ToolError {
    ToolError::new(MENU_NOT_FOUND, format!("There is no menu entry {id}."))
        .detail(json!({"entity": "menu", "id": id}))
        .next("list_menus")
}
