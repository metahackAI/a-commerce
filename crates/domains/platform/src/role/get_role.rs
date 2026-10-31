//! `get_role` (`platform.role.get`): one role with its scopes, approver roles and how many people
//! hold it.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ROLE_NOT_FOUND, not_found, role, scopes_of};
use crate::repo::db;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetRoleInput {
    /// The role.
    #[schemars(regex(pattern = r"^rol_[a-z0-9_]+$"), example = &"rol_merchandiser")]
    pub role_id: String,
}

/// One role.
pub struct GetRole;

impl Operation for GetRole {
    const META: OpMeta = OpMeta {
        address: "platform.role.get",
        tool: "get_role",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/roles/{role_id}",
        summary: "One role: its scopes, the approver roles it carries (approval.yml approver_role), and who holds it. Administrators only.",
        consumes: &[Slot::one("role_id")],
        produces: &[],
        checks: &[],
        errors: &[ROLE_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetRoleInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetRole {
    async fn read(&self, cx: &Cx, input: GetRoleInput) -> Result<Value, ToolError> {
        crate::require_admin(cx, None)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = role(&mut tx, &input.role_id)
            .await?
            .ok_or_else(|| not_found(&input.role_id))?;
        let scopes = scopes_of(&mut tx, &r.id).await?;
        let holders: Vec<(String, String)> = sqlx::query_as(
            "SELECT a.id, a.name FROM platform.admin_user_role ur
             JOIN platform.admin_user a ON a.tenant_id = ur.tenant_id AND a.id = ur.admin_user_id
             WHERE ur.tenant_id = $1 AND ur.role_id = $2 ORDER BY a.name, a.id",
        )
        .bind(cx.tenant.as_str())
        .bind(&r.id)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(json!({
            "id": r.id, "name": r.name, "description": r.description, "scopes": scopes,
            "approver_roles": r.approver_roles, "acom:is_system": r.is_system,
            "acom:assigned_count": holders.len(),
            "assigned_to": holders.iter().map(|(id, name)| json!({"admin_user_id": id, "name": name}))
                .collect::<Vec<_>>(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn shows_scopes_approver_roles_and_holders() {
        let cx = crate::testdb::admin().await;
        let r = GetRole
            .read(
                &cx,
                serde_json::from_value(json!({"role_id": "rol_merchandiser"})).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r["name"], "Merchandiser");
        assert_eq!(r["approver_roles"], json!(["merchandiser", "operator"]));
        assert!(r["acom:assigned_count"].as_u64().unwrap() >= 2, "{r}");
        let e = GetRole
            .read(
                &cx,
                serde_json::from_value(json!({"role_id": "rol_nobody"})).unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("role_not_found", Some("list_roles"))
        );
    }
}
