//! `list_roles` (`platform.role.list`): the store's roles with their scopes.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::repo::db;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListRolesInput {
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// Every role, system roles first.
pub struct ListRoles;

impl Operation for ListRoles {
    const META: OpMeta = OpMeta {
        address: "platform.role.list",
        tool: "list_roles",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/roles",
        summary: "The store's roles: name, scopes, and whether it is a built-in system role. Administrators only.",
        consumes: &[],
        produces: &[Slot::many("role_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListRolesInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListRoles {
    async fn read(&self, cx: &Cx, input: ListRolesInput) -> Result<Value, ToolError> {
        crate::require_admin(cx, None)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 =
            sqlx::query_scalar("SELECT count(*) FROM platform.role WHERE tenant_id = $1")
                .bind(cx.tenant.as_str())
                .fetch_one(&mut *tx.tx)
                .await
                .map_err(db)?;
        let rows: Vec<(String, String, bool, Vec<String>)> = sqlx::query_as(
            "SELECT r.id, r.name, r.is_system,
                    coalesce(array_agg(rs.scope ORDER BY rs.scope)
                             FILTER (WHERE rs.scope IS NOT NULL), '{}')
             FROM platform.role r
             LEFT JOIN platform.role_scope rs ON rs.tenant_id = r.tenant_id AND rs.role_id = r.id
             WHERE r.tenant_id = $1
             GROUP BY r.id, r.name, r.is_system
             ORDER BY r.is_system DESC, r.name, r.id LIMIT $2 OFFSET $3",
        )
        .bind(cx.tenant.as_str())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items = rows
            .into_iter()
            .map(|(id, name, system, scopes)| {
                json!({"id": id, "name": name, "scopes": scopes, "acom:is_system": system})
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
    async fn lists_the_system_roles_with_their_scopes() {
        let cx = crate::testdb::admin().await;
        let out = ListRoles
            .read(&cx, serde_json::from_value(json!({"limit": 50})).unwrap())
            .await
            .unwrap();
        assert!(out["total"].as_i64().unwrap() >= 5, "{out}");
        let merch = out["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "rol_merchandiser")
            .unwrap();
        assert_eq!(merch["acom:is_system"], true);
        assert!(
            merch["scopes"]
                .as_array()
                .unwrap()
                .contains(&json!("human:approve"))
        );
    }
}
