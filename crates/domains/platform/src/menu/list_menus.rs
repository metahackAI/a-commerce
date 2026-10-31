//! `list_menus` (`platform.menu.list`): the back-office menu the caller's scopes open, with their
//! favorites.

use std::collections::BTreeSet;

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::enums::ActorKind;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{COLUMNS, MenuRow};
use crate::repo::db;

/// No input: the menu follows the caller's own scopes.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListMenusInput {}

/// The menu for the caller.
pub struct ListMenus;

impl Operation for ListMenus {
    const META: OpMeta = OpMeta {
        address: "platform.menu.list",
        tool: "list_menus",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/menus",
        summary: "The back-office menu tree the caller's roles open, with the caller's favorites marked.",
        consumes: &[],
        produces: &[Slot::many("menu_id")],
        checks: &[],
        errors: &[ErrorCode::UNAUTHENTICATED],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListMenusInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListMenus {
    async fn read(&self, cx: &Cx, _input: ListMenusInput) -> Result<Value, ToolError> {
        let p = &cx.principal;
        let everything = p.kind == ActorKind::System;
        if !everything && !p.subject.starts_with("adm_") {
            return Err(ToolError::new(
                ErrorCode::UNAUTHENTICATED,
                "The back-office menu is for signed-in operators.",
            )
            .detail(json!({"reason": "not_signed_in"})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let rows: Vec<MenuRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM platform.site_navigation_element
             WHERE tenant_id = $1 AND is_visible ORDER BY position, name, id"
        )))
        .bind(cx.tenant.as_str())
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        let favorites: BTreeSet<String> = sqlx::query_scalar::<_, String>(
            "SELECT site_navigation_element_id FROM platform.menu_favorite
             WHERE tenant_id = $1 AND admin_user_id = $2 AND is_favorite",
        )
        .bind(cx.tenant.as_str())
        .bind(&p.subject)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?
        .into_iter()
        .collect();
        tx.commit().await.map_err(ToolError::from)?;
        let opens = |m: &MenuRow| everything || p.has_scope(&m.required_scope);
        let entry = |m: &MenuRow| {
            json!({"id": m.id, "name": m.name, "path": m.url, "required_scope": m.required_scope,
                   "favorited": favorites.contains(&m.id)})
        };
        let items: Vec<Value> = rows
            .iter()
            .filter(|m| m.is_part_of_id.is_none() && opens(m))
            .map(|top| {
                let mut e = entry(top);
                e["children"] = rows
                    .iter()
                    .filter(|c| c.is_part_of_id.as_deref() == Some(top.id.as_str()) && opens(c))
                    .map(entry)
                    .collect();
                e
            })
            .collect();
        let total = items.len();
        Ok(json!({"items": items, "total": total}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn each_operator_sees_what_their_roles_open_and_their_favorites() {
        let catalog_only = crate::testdb::cx_with(
            "adm_sj",
            ActorKind::Operator,
            [
                "tools:read:catalog".to_owned(),
                "tools:read:promotion".to_owned(),
            ]
            .into(),
        )
        .await;
        let out = ListMenus
            .read(&catalog_only, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        let items = out["items"].as_array().unwrap();
        assert!(items.iter().all(|m| {
            let s = m["required_scope"].as_str().unwrap();
            s == "tools:read:catalog" || s == "tools:read:promotion"
        }));
        let promotions = items.iter().find(|m| m["id"] == "mnu_promotions").unwrap();
        assert_eq!(promotions["favorited"], true);
        let products = items.iter().find(|m| m["id"] == "mnu_products").unwrap();
        assert!(!products["children"].as_array().unwrap().is_empty());
        let admin = crate::testdb::admin().await;
        let all = ListMenus
            .read(&admin, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        assert!(all["total"].as_u64().unwrap() > out["total"].as_u64().unwrap());
    }
}
