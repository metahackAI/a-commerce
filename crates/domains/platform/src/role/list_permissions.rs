//! `list_permissions` (`platform.role.list_permissions`): the scopes people can choose from when
//! they design a role, a menu entry or an API key.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Domain, catalog};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListPermissionsInput {
    /// Only the scopes of this API domain.
    #[schemars(example = &"promotion")]
    pub domain: Option<Domain>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// The scope catalog.
pub struct ListPermissions;

impl Operation for ListPermissions {
    const META: OpMeta = OpMeta {
        address: "platform.role.list_permissions",
        tool: "list_permissions",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/permissions",
        summary: "The scopes a role, menu entry or API key can use, each with its domain and what it allows. Administrators only.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListPermissionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListPermissions {
    async fn read(&self, cx: &Cx, input: ListPermissionsInput) -> Result<Value, ToolError> {
        crate::require_admin(cx, None)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let all: Vec<Value> = catalog(cx)?
            .into_iter()
            .filter(|p| input.domain.is_none_or(|d| p.domain == d.as_str()))
            .map(|p| json!(p))
            .collect();
        let total = i64::try_from(all.len()).unwrap_or(i64::MAX);
        let items = all
            .into_iter()
            .skip(usize::try_from(page.offset).unwrap_or(0))
            .take(usize::try_from(page.limit).unwrap_or(20))
            .collect();
        Ok(crate::list(items, total, &page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offline() -> Cx {
        crate::testdb::offline("adm_admin", acom_auth::scopes::admin())
    }

    #[tokio::test]
    async fn every_domain_has_its_tool_scopes_and_people_keep_theirs() {
        let cx = offline();
        let out = ListPermissions
            .read(
                &cx,
                serde_json::from_value(json!({"domain": "promotion"})).unwrap(),
            )
            .await
            .unwrap();
        let scopes: Vec<&str> = out["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["scope"].as_str().unwrap())
            .collect();
        assert_eq!(scopes, ["tools:read:promotion", "tools:write:promotion"]);
        let all = ListPermissions
            .read(&cx, serde_json::from_value(json!({"limit": 100})).unwrap())
            .await
            .unwrap();
        let items = all["items"].as_array().unwrap();
        assert!(items.iter().any(|p| p["scope"] == "external:cart"));
        let approve = items
            .iter()
            .find(|p| p["scope"] == "human:approve")
            .unwrap();
        assert!(
            approve["description"]
                .as_str()
                .unwrap()
                .contains("own click")
        );
        let sj = crate::testdb::offline("adm_sj", ["tools:read:platform".to_owned()].into());
        let e = ListPermissions
            .read(&sj, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap_err();
        assert_eq!((e.code.as_ref(), e.next_tool), ("forbidden", None));
    }
}
