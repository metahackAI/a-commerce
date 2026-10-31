//! `list_tenants` (`platform.tenant.list`): the stores, for the platform administrator.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::StoreStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::repo::db;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTenantsInput {
    /// Only stores in this state (active: open for business; suspended: kept but serving no one).
    #[schemars(example = &"active")]
    pub status: Option<StoreStatus>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// The stores the caller can see.
pub struct ListTenants;

impl Operation for ListTenants {
    const META: OpMeta = OpMeta {
        address: "platform.tenant.list",
        tool: "list_tenants",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/tenants",
        summary: "The stores, with name and status. Platform administrators only.",
        consumes: &[],
        produces: &[Slot::many("tenant_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListTenantsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListTenants {
    async fn read(&self, cx: &Cx, input: ListTenantsInput) -> Result<Value, ToolError> {
        crate::require_admin(cx, Some("list_roles"))?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let status = input.status.map(StoreStatus::as_str);
        // Row-level security shows a request only its own store, so this lists that one store
        // (or none when the status filter leaves it out).
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.online_store
             WHERE ($1::text IS NULL OR store_status = $1)",
        )
        .bind(status)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT id, name, store_status FROM platform.online_store
             WHERE ($1::text IS NULL OR store_status = $1)
             ORDER BY name, id LIMIT $2 OFFSET $3",
        )
        .bind(status)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items = rows
            .into_iter()
            .map(|(id, name, s)| json!({"id": id, "name": name, "acom:status": s}))
            .collect();
        Ok(crate::list(items, total, &page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_core::enums::ActorKind;

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn the_administrator_sees_the_store_and_others_are_refused() {
        let cx = crate::testdb::admin().await;
        let out = ListTenants
            .read(&cx, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        assert_eq!(out["total"], 1);
        assert_eq!(out["items"][0]["id"], "store_northwind");
        let none = ListTenants
            .read(
                &cx,
                serde_json::from_value(json!({"status": "suspended"})).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(none["total"], 0);
        let sj = crate::testdb::cx_with(
            "adm_sj",
            ActorKind::Operator,
            ["tools:read:platform".to_owned()].into(),
        )
        .await;
        let e = ListTenants
            .read(&sj, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("forbidden", Some("list_roles"))
        );
    }
}
