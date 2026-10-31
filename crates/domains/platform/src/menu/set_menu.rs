//! `set_menu` (`platform.menu.set`): define a back-office menu entry: path, name, the scope it
//! needs, its place, and whether it shows.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{MENU_NOT_FOUND, MenuRow, menu, not_found};
use crate::repo::db;
use crate::role::{INVALID_SCOPE, known_scopes};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetMenuInput {
    /// The entry. A new id makes a new entry, which needs `name`, `path` and `required_scope`.
    #[schemars(regex(pattern = r"^mnu_[a-z0-9_]+$"), example = &"mnu_promotions")]
    pub menu_id: String,
    /// The label operators see.
    #[schemars(length(min = 1, max = 60), example = &"Promotions")]
    pub name: Option<String>,
    /// The back-office path it opens.
    #[schemars(regex(pattern = r"^/[a-z0-9/_\-]*$"), example = &"/bo/promotions")]
    pub path: Option<String>,
    /// The scope an operator needs to see it, from list_permissions.
    #[schemars(example = &"tools:read:promotion")]
    pub required_scope: Option<String>,
    /// Order among the entries beside it, smallest first.
    #[schemars(range(min = 0, max = 1000), example = 40)]
    pub position: Option<i32>,
    /// false hides it from every operator without removing it.
    #[schemars(example = true)]
    pub visible: Option<bool>,
}

/// What `set_menu` runs; `before` is `None` for a new entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetMenuPlan {
    pub to: MenuRow,
    pub before: Option<MenuRow>,
}

/// Define a menu entry.
pub struct SetMenu;

impl Operation for SetMenu {
    const META: OpMeta = OpMeta {
        address: "platform.menu.set",
        tool: "set_menu",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Put,
        path: "/v1/platform/menus/{menu_id}",
        summary: "Define a back-office menu entry: name, path, the scope it needs, its position and whether it shows. Administrators only.",
        consumes: &[],
        produces: &[Slot::one("menu_id")],
        checks: &["roles.known_scopes"],
        errors: &[INVALID_SCOPE, MENU_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetMenuInput;
    type Output = Value;
}

fn shown(m: &MenuRow) -> Value {
    json!({"name": m.name, "path": m.url, "required_scope": m.required_scope,
           "position": m.position, "visible": m.is_visible})
}

async fn write(tx: &mut TxCx, m: &MenuRow) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO platform.site_navigation_element (tenant_id, id, name, url, required_scope,
           position, is_visible, is_part_of_id) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (tenant_id, id) DO UPDATE SET name = EXCLUDED.name, url = EXCLUDED.url,
           required_scope = EXCLUDED.required_scope, position = EXCLUDED.position,
           is_visible = EXCLUDED.is_visible, updated_at = now(),
           version = platform.site_navigation_element.version + 1",
    )
    .bind(tx.tenant.as_str())
    .bind(&m.id)
    .bind(&m.name)
    .bind(&m.url)
    .bind(&m.required_scope)
    .bind(m.position)
    .bind(m.is_visible)
    .bind(m.is_part_of_id.as_deref())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

fn same(a: &MenuRow, b: &MenuRow) -> bool {
    a.name == b.name
        && a.url == b.url
        && a.required_scope == b.required_scope
        && a.position == b.position
        && a.is_visible == b.is_visible
}

impl WriteOp<Cx> for SetMenu {
    type Plan = SetMenuPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetMenuInput,
    ) -> Result<Planned<SetMenuPlan>, ToolError> {
        crate::require_admin(cx, Some("list_menus"))?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let current = menu(&mut tx, &input.menu_id).await?;
        let next_position: i32 = if current.is_none() {
            sqlx::query_scalar(
                "SELECT coalesce(max(position), 0) + 10 FROM platform.site_navigation_element
                 WHERE tenant_id = $1 AND is_part_of_id IS NULL",
            )
            .bind(cx.tenant.as_str())
            .fetch_one(&mut *tx.tx)
            .await
            .map_err(db)?
        } else {
            0
        };
        tx.commit().await.map_err(ToolError::from)?;
        let text = |v: &Option<String>| {
            v.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let to = match &current {
            Some(m) => MenuRow {
                name: text(&input.name).unwrap_or_else(|| m.name.clone()),
                url: text(&input.path).unwrap_or_else(|| m.url.clone()),
                required_scope: text(&input.required_scope)
                    .unwrap_or_else(|| m.required_scope.clone()),
                position: input.position.unwrap_or(m.position),
                is_visible: input.visible.unwrap_or(m.is_visible),
                ..m.clone()
            },
            None => {
                let (Some(name), Some(url), Some(scope)) = (
                    text(&input.name),
                    text(&input.path),
                    text(&input.required_scope),
                ) else {
                    return Err(not_found(&input.menu_id).detail(json!({
                        "entity": "menu", "id": input.menu_id,
                        "hint": "A new entry needs name, path and required_scope."})));
                };
                MenuRow {
                    id: input.menu_id.clone(),
                    name,
                    url,
                    required_scope: scope,
                    position: input.position.unwrap_or(next_position),
                    is_visible: input.visible.unwrap_or(true),
                    is_part_of_id: None,
                    version: 0,
                }
            }
        };
        if !to.url.starts_with('/') {
            return Err(ToolError::invalid_input("A menu path starts with `/`.")
                .detail(json!({"field": "path", "reason": "format"})));
        }
        let check = known_scopes(cx, std::slice::from_ref(&to.required_scope))?;
        if current.as_ref().is_some_and(|m| same(m, &to)) {
            return Err(ToolError::invalid_input(format!(
                "Nothing to change: {} already has these values.",
                to.id
            ))
            .detail(json!({"field": "menu_id", "reason": "unchanged"})));
        }
        Ok(Planned {
            changes: vec![Change {
                op: if current.is_some() {
                    ChangeOp::Update
                } else {
                    ChangeOp::Create
                },
                entity: "menu".into(),
                id: if current.is_some() {
                    to.id.clone()
                } else {
                    String::new()
                },
                from: current.as_ref().map_or(Value::Null, shown),
                to: {
                    let mut v = shown(&to);
                    v["menu_id"] = json!(to.id);
                    v
                },
                note: None,
            }],
            checks: vec![check],
            basis: current
                .iter()
                .map(|m| Basis {
                    entity: "platform.site_navigation_element".into(),
                    id: m.id.clone(),
                    version: i64::from(m.version),
                })
                .collect(),
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan: SetMenuPlan {
                to,
                before: current,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SetMenuPlan) -> Result<Committed, ToolError> {
        write(tx, &plan.to).await?;
        let mut produced = Map::new();
        produced.insert("menu_id".into(), json!(plan.to.id));
        Ok(Committed {
            id: plan.to.id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SetMenuPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let m = menu(&mut tx, &plan.to.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = m.as_ref().is_some_and(|m| same(m, &plan.to));
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not as previewed", plan.to.id)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SetMenuPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        if let Some(before) = &plan.before {
            return write(tx, before).await;
        }
        let children: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.site_navigation_element
             WHERE tenant_id = $1 AND is_part_of_id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.to.id)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        if children > 0 {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("{} has entries under it now; it stays.", plan.to.id),
            )
            .detail(json!({"entity": "menu", "id": plan.to.id}))
            .next("list_menus"));
        }
        for sql in [
            "DELETE FROM platform.menu_favorite WHERE tenant_id = $1 AND site_navigation_element_id = $2",
            "DELETE FROM platform.site_navigation_element WHERE tenant_id = $1 AND id = $2",
        ] {
            sqlx::query(sql)
                .bind(tx.tenant.as_str())
                .bind(&plan.to.id)
                .execute(&mut *tx.tx)
                .await
                .map_err(db)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn input(v: Value) -> SetMenuInput {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_input_is_closed() {
        let s = closed_schema_for::<SetMenuInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["menu_id"]));
    }

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn adds_an_entry_hides_one_and_takes_both_back() {
        let cx = crate::testdb::admin().await;
        let e = SetMenu
            .preview(
                &cx,
                input(json!({"menu_id": "mnu_test_reports", "name": "Reports"})),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("menu_not_found", Some("list_menus"))
        );
        let e = SetMenu
            .preview(
                &cx,
                input(json!({"menu_id": "mnu_test_reports", "name": "Reports",
                             "path": "/bo/reports", "required_scope": "tools:read:everything"})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_scope");
        let made = SetMenu
            .preview(
                &cx,
                input(json!({"menu_id": "mnu_test_reports", "name": "Reports",
                             "path": "/bo/reports", "required_scope": "tools:read:analytics"})),
            )
            .await
            .unwrap();
        assert_eq!(made.changes[0].op, ChangeOp::Create);
        let hidden = SetMenu
            .preview(&cx, input(json!({"menu_id": "mnu_data", "visible": false})))
            .await
            .unwrap();
        assert_eq!(hidden.changes[0].from["visible"], true);
        for p in [&made, &hidden] {
            let mut tx = cx.begin().await.unwrap();
            let done = SetMenu.commit(&mut tx, &p.plan).await.unwrap();
            tx.commit().await.unwrap();
            assert!(SetMenu.verify(&cx, &p.plan, &done).await.unwrap().matched);
            let mut tx = cx.begin().await.unwrap();
            SetMenu.undo(&mut tx, &p.plan, &done).await.unwrap();
            tx.commit().await.unwrap();
        }
        let mut tx = cx.read().await.unwrap();
        assert!(menu(&mut tx, "mnu_test_reports").await.unwrap().is_none());
        assert!(menu(&mut tx, "mnu_data").await.unwrap().unwrap().is_visible);
        tx.commit().await.unwrap();
    }
}
