//! `set_favorite` (`platform.menu.set_favorite`): keep a menu entry among my favorites, or not.
//! A setting of the person's own screen: it runs right after its preview, outside change sets.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, ErrorCode, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{MENU_NOT_FOUND, menu, not_found};
use crate::repo::db;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetFavoriteInput {
    /// The menu entry.
    #[schemars(regex(pattern = r"^mnu_[a-z0-9_]+$"), example = &"mnu_promotions")]
    pub menu_id: String,
    /// true keeps it among my favorites; false takes it out.
    #[schemars(example = true)]
    pub favorited: bool,
}

/// What `set_favorite` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetFavoritePlan {
    pub admin_user_id: String,
    pub menu_id: String,
    pub favorited: bool,
    pub before: bool,
}

/// Mark or unmark a favorite.
pub struct SetFavorite;

impl Operation for SetFavorite {
    const META: OpMeta = OpMeta {
        address: "platform.menu.set_favorite",
        tool: "set_favorite",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Put,
        path: "/v1/platform/menus/{menu_id}/favorite",
        summary: "Keep a back-office menu entry among my favorites, or take it out. My own screen setting; calling it again with the other value turns it back.",
        consumes: &[Slot::one("menu_id")],
        produces: &[Slot::one("menu_id")],
        checks: &[],
        errors: &[MENU_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetFavoriteInput;
    type Output = Value;
}

async fn favorited(tx: &mut TxCx, admin_user_id: &str, menu_id: &str) -> Result<bool, ToolError> {
    let v: Option<bool> = sqlx::query_scalar(
        "SELECT is_favorite FROM platform.menu_favorite
         WHERE tenant_id = $1 AND admin_user_id = $2 AND site_navigation_element_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(admin_user_id)
    .bind(menu_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(v.unwrap_or(false))
}

async fn write(tx: &mut TxCx, plan: &SetFavoritePlan, value: bool) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO platform.menu_favorite (tenant_id, admin_user_id, site_navigation_element_id,
           is_favorite) VALUES ($1, $2, $3, $4)
         ON CONFLICT (tenant_id, admin_user_id, site_navigation_element_id) DO UPDATE
           SET is_favorite = EXCLUDED.is_favorite, updated_at = now(),
               version = platform.menu_favorite.version + 1",
    )
    .bind(tx.tenant.as_str())
    .bind(&plan.admin_user_id)
    .bind(&plan.menu_id)
    .bind(value)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

impl WriteOp<Cx> for SetFavorite {
    type Plan = SetFavoritePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetFavoriteInput,
    ) -> Result<Planned<SetFavoritePlan>, ToolError> {
        let me = cx.principal.subject.clone();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let known: Option<String> = if me.starts_with("adm_") {
            sqlx::query_scalar(
                "SELECT id FROM platform.admin_user WHERE tenant_id = $1 AND id = $2",
            )
            .bind(cx.tenant.as_str())
            .bind(&me)
            .fetch_optional(&mut *tx.tx)
            .await
            .map_err(db)?
        } else {
            None
        };
        if known.is_none() {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                "Favorites belong to an operator account; sign in as one.",
            )
            .detail(json!({"reason": "owner"})));
        }
        let m = menu(&mut tx, &input.menu_id)
            .await?
            .ok_or_else(|| not_found(&input.menu_id))?;
        let before = favorited(&mut tx, &me, &m.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let plan = SetFavoritePlan {
            admin_user_id: me,
            menu_id: m.id.clone(),
            favorited: input.favorited,
            before,
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "menu_favorite".into(),
                id: m.id.clone(),
                from: json!({"favorited": before}),
                to: json!({"favorited": input.favorited, "name": m.name}),
                note: None,
            }],
            checks: Vec::new(),
            basis: Vec::new(),
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SetFavoritePlan) -> Result<Committed, ToolError> {
        write(tx, plan, plan.favorited).await?;
        let mut produced = Map::new();
        produced.insert("menu_id".into(), json!(plan.menu_id));
        produced.insert("favorited".into(), json!(plan.favorited));
        Ok(Committed {
            id: plan.menu_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SetFavoritePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let now = favorited(&mut tx, &plan.admin_user_id, &plan.menu_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: now == plan.favorited,
            mismatches: if now == plan.favorited {
                Vec::new()
            } else {
                vec![format!("{} favorite is {now}", plan.menu_id)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SetFavoritePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        write(tx, plan, plan.before).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_core::enums::ActorKind;

    fn input(v: Value) -> SetFavoriteInput {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn my_favorites_are_mine_and_turn_back() {
        let sj = crate::testdb::cx_with(
            "adm_sj",
            ActorKind::AgentOnBehalf,
            ["tools:read:platform".to_owned()].into(),
        )
        .await;
        let p = SetFavorite
            .preview(
                &sj,
                input(json!({"menu_id": "mnu_orders", "favorited": true})),
            )
            .await
            .unwrap();
        assert_eq!(p.plan.admin_user_id, "adm_sj");
        assert_eq!(p.approval, acom_contract::ApprovalNeed::NotNeeded);
        let mut tx = sj.begin().await.unwrap();
        let done = SetFavorite.commit(&mut tx, &p.plan).await.unwrap();
        tx.commit().await.unwrap();
        assert!(
            SetFavorite
                .verify(&sj, &p.plan, &done)
                .await
                .unwrap()
                .matched
        );
        let mut tx = sj.begin().await.unwrap();
        SetFavorite.undo(&mut tx, &p.plan, &done).await.unwrap();
        tx.commit().await.unwrap();
        assert!(
            !SetFavorite
                .verify(&sj, &p.plan, &done)
                .await
                .unwrap()
                .matched
        );
        crate::testdb::exec(
            &sj,
            "DELETE FROM platform.menu_favorite WHERE tenant_id = 'store_northwind'
               AND admin_user_id = $1 AND site_navigation_element_id = $2",
            &["adm_sj", "mnu_orders"],
        )
        .await;
        let e = SetFavorite
            .preview(
                &sj,
                input(json!({"menu_id": "mnu_nowhere", "favorited": true})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "menu_not_found");
        let shopper = crate::testdb::cx_with(
            "cus_jamie",
            ActorKind::Shopper,
            acom_auth::scopes::shopper(),
        )
        .await;
        let e = SetFavorite
            .preview(
                &shopper,
                input(json!({"menu_id": "mnu_orders", "favorited": true})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }
}
