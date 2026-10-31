//! `list_orgs` (`platform.org.list`): the organization tree with how many people each has.

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
pub struct ListOrgsInput {
    /// Only the organizations directly under this one.
    #[schemars(regex(pattern = r"^org_[a-z0-9_]+$"), example = &"org_northwind")]
    pub parent_org_id: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// The organizations.
pub struct ListOrgs;

impl Operation for ListOrgs {
    const META: OpMeta = OpMeta {
        address: "platform.org.list",
        tool: "list_orgs",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/orgs",
        summary: "The organization tree: departments, vendors and branches with their parent and how many operator accounts belong to each. Administrators only.",
        consumes: &[],
        produces: &[Slot::many("org_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListOrgsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListOrgs {
    async fn read(&self, cx: &Cx, input: ListOrgsInput) -> Result<Value, ToolError> {
        crate::require_admin(cx, Some("list_roles"))?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let filter = "o.tenant_id = $1 AND ($2::text IS NULL OR o.parent_organization_id = $2)";
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM platform.organization o WHERE {filter}"
        )))
        .bind(cx.tenant.as_str())
        .bind(input.parent_org_id.as_deref())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        // Top-level organizations first, then by name.
        let rows: Vec<(String, String, String, Option<String>, i64)> =
            sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT o.id, o.name, o.organization_type, o.parent_organization_id,
                        (SELECT count(*) FROM platform.admin_user a
                         WHERE a.tenant_id = o.tenant_id AND a.organization_id = o.id)
                 FROM platform.organization o WHERE {filter}
                 ORDER BY o.parent_organization_id IS NOT NULL, o.name, o.id
                 LIMIT $3 OFFSET $4"
            )))
            .bind(cx.tenant.as_str())
            .bind(input.parent_org_id.as_deref())
            .bind(page.limit)
            .bind(page.offset)
            .fetch_all(&mut *tx.tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items = rows
            .into_iter()
            .map(|(id, name, kind, parent, members)| {
                json!({"id": id, "name": name, "acom:org_type": kind, "parent_org_id": parent,
                       "acom:member_count": members})
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
    async fn lists_the_tree_with_member_counts() {
        let cx = crate::testdb::admin().await;
        let all = ListOrgs
            .read(&cx, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        assert!(all["total"].as_i64().unwrap() >= 4, "{all}");
        assert_eq!(all["items"][0]["parent_org_id"], Value::Null);
        let merch = all["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == "org_merchandising")
            .unwrap();
        assert_eq!(merch["acom:org_type"], "department");
        assert!(merch["acom:member_count"].as_i64().unwrap() >= 1);
        let under = ListOrgs
            .read(
                &cx,
                serde_json::from_value(json!({"parent_org_id": "org_northwind"})).unwrap(),
            )
            .await
            .unwrap();
        assert!(
            under["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|o| o["parent_org_id"] == "org_northwind")
        );
    }
}
