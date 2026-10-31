//! `search_admin_users` (`platform.admin_user.search`): operator accounts by role, status,
//! organization or a word of their name, title or email. Emails are masked.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::AdminStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::masked_email;
use crate::repo::db;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchAdminUsersInput {
    /// Only accounts with this role.
    #[schemars(regex(pattern = r"^rol_[a-z0-9_]+$"), example = &"rol_merchandiser")]
    pub role_id: Option<String>,
    /// Only accounts in this state (active: can sign in; suspended: stopped by an administrator;
    /// locked: stopped after too many wrong passwords).
    #[schemars(example = &"active")]
    pub status: Option<AdminStatus>,
    /// Only accounts in this organization.
    #[schemars(regex(pattern = r"^org_[a-z0-9_]+$"), example = &"org_merchandising")]
    pub org_id: Option<String>,
    /// A word of the name, short name, job title, email or login id.
    #[schemars(length(min = 1, max = 100), example = &"Son")]
    pub query: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// Find operator accounts.
pub struct SearchAdminUsers;

impl Operation for SearchAdminUsers {
    const META: OpMeta = OpMeta {
        address: "platform.admin_user.search",
        tool: "search_admin_users",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/admin-users",
        summary: "Find operator accounts by role, status, organization, or a word of the name, title or email; with their roles. Emails are masked. Administrators only.",
        consumes: &[],
        produces: &[Slot::many("admin_user_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchAdminUsersInput;
    type Output = Value;
}

type Row = (
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Vec<String>,
);

impl ReadOp<Cx> for SearchAdminUsers {
    async fn read(&self, cx: &Cx, input: SearchAdminUsersInput) -> Result<Value, ToolError> {
        crate::require_admin(cx, Some("list_roles"))?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let status = input.status.map(AdminStatus::as_str);
        let query = input
            .query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(|q| {
                format!(
                    "%{}%",
                    q.replace('\\', "\\\\")
                        .replace('%', "\\%")
                        .replace('_', "\\_")
                )
            });
        let filter = "a.tenant_id = $1
            AND ($2::text IS NULL OR EXISTS (SELECT 1 FROM platform.admin_user_role x
                 WHERE x.tenant_id = a.tenant_id AND x.admin_user_id = a.id AND x.role_id = $2))
            AND ($3::text IS NULL OR a.admin_status = $3)
            AND ($4::text IS NULL OR a.organization_id = $4)
            AND ($5::text IS NULL OR a.name ILIKE $5 OR a.alternate_name ILIKE $5
                 OR a.job_title ILIKE $5 OR a.email ILIKE $5 OR a.login_id ILIKE $5)";
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM platform.admin_user a WHERE {filter}"
        )))
        .bind(cx.tenant.as_str())
        .bind(input.role_id.as_deref())
        .bind(status)
        .bind(input.org_id.as_deref())
        .bind(query.as_deref())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT a.id, a.name, a.job_title, a.email, a.admin_status, a.organization_id,
                    coalesce((SELECT array_agg(r.role_id ORDER BY r.role_id)
                              FROM platform.admin_user_role r
                              WHERE r.tenant_id = a.tenant_id AND r.admin_user_id = a.id), '{{}}')
             FROM platform.admin_user a WHERE {filter}
             ORDER BY a.name, a.id LIMIT $6 OFFSET $7"
        )))
        .bind(cx.tenant.as_str())
        .bind(input.role_id.as_deref())
        .bind(status)
        .bind(input.org_id.as_deref())
        .bind(query.as_deref())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items = rows
            .into_iter()
            .map(|(id, name, title, email, status, org, roles)| {
                json!({"id": id, "name": name, "job_title": title, "roles": roles,
                       "status": status, "org_id": org, "email": masked_email(&email)})
            })
            .collect();
        Ok(crate::list(items, total, &page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn search(v: Value) -> Value {
        let cx = crate::testdb::admin().await;
        SearchAdminUsers
            .read(&cx, serde_json::from_value(v).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn finds_accounts_by_role_status_and_word_with_masked_emails() {
        let all = search(json!({})).await;
        assert!(all["total"].as_i64().unwrap() >= 4, "{all}");
        let sj = search(json!({"query": "seongjun"})).await;
        assert_eq!(sj["total"], 1);
        assert_eq!(sj["items"][0]["id"], "adm_sj");
        assert_eq!(sj["items"][0]["email"], "s***@northwind.example");
        assert!(
            sj["items"][0]["roles"]
                .as_array()
                .unwrap()
                .contains(&json!("rol_merchandiser"))
        );
        let merch = search(json!({"role_id": "rol_merchandiser", "status": "active"})).await;
        assert!(
            merch["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|a| a["status"] == "active")
        );
        let suspended = search(json!({"status": "suspended"})).await;
        assert!(suspended["total"].as_i64().unwrap() >= 1, "{suspended}");
        let none = search(json!({"query": "100%_nobody"})).await;
        assert_eq!(none["total"], 0);
    }
}
