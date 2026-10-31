//! `list_api_keys` (`platform.api_key.list`): the store's API keys, without their secrets.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::ApiKeyStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{COLUMNS, KeyRow};
use crate::repo::db;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListApiKeysInput {
    /// Only keys in this state (active: can sign in; revoked: stopped for good).
    #[schemars(example = &"active")]
    pub status: Option<ApiKeyStatus>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// The API keys.
pub struct ListApiKeys;

impl Operation for ListApiKeys {
    const META: OpMeta = OpMeta {
        address: "platform.api_key.list",
        tool: "list_api_keys",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/api-keys",
        summary: "The store's API keys: name, scopes, state, the first characters of the secret, last use. Secrets are never shown. Administrators only.",
        consumes: &[],
        produces: &[Slot::many("api_key_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListApiKeysInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListApiKeys {
    async fn read(&self, cx: &Cx, input: ListApiKeysInput) -> Result<Value, ToolError> {
        crate::require_admin(cx, Some("list_roles"))?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let status = input.status.map(ApiKeyStatus::as_str);
        let filter = "tenant_id = $1 AND ($2::text IS NULL OR api_key_status = $2)";
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM platform.api_key WHERE {filter}"
        )))
        .bind(cx.tenant.as_str())
        .bind(status)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows: Vec<KeyRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM platform.api_key WHERE {filter}
             ORDER BY api_key_status, name, id LIMIT $3 OFFSET $4"
        )))
        .bind(cx.tenant.as_str())
        .bind(status)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let at = |t: Option<chrono::DateTime<chrono::Utc>>| t.map(|t| cx.time.format(t));
        let items = rows
            .iter()
            .map(|k| {
                json!({"id": k.id, "name": k.name, "scopes": k.scopes,
                       "status": k.api_key_status, "key_prefix": k.key_prefix,
                       "last_used_at": at(k.last_used_at), "expires": at(k.expires),
                       "revoked_at": at(k.revoked_at), "created_by": k.creator_id})
            })
            .collect();
        Ok(crate::list(items, total, &page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn lists_keys_by_their_prefix_never_their_secret() {
        let cx = crate::testdb::admin().await;
        let out = ListApiKeys
            .read(
                &cx,
                serde_json::from_value(json!({"status": "active"})).unwrap(),
            )
            .await
            .unwrap();
        let k = out["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|k| k["id"] == "key_31")
            .unwrap();
        assert_eq!(k["key_prefix"], "ak_live_3f");
        assert_eq!(k["last_used_at"], "2026-11-06T09:14:00-05:00");
        assert!(k.get("secret_hash").is_none() && k.get("secret").is_none());
        let revoked = ListApiKeys
            .read(
                &cx,
                serde_json::from_value(json!({"status": "revoked"})).unwrap(),
            )
            .await
            .unwrap();
        assert!(revoked["total"].as_i64().unwrap() >= 1);
    }
}
