//! `get_tenant` (`platform.tenant.get`): the store's name, language, currency, time zone, feature
//! flags and how many policy values it overrides.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{COLUMNS, StoreRow, TENANT_NOT_FOUND, overridden_keys, store};
use crate::repo::db;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetTenantInput {
    /// The store; it must be the store this request is for.
    #[schemars(regex(pattern = r"^store_[a-z0-9_]+$"), example = &"store_northwind")]
    pub tenant_id: String,
    /// Top-level fields to return, for example only the time zone and currency.
    #[schemars(example = ["acom:time_zone", "currency"])]
    pub fields: Option<Vec<String>>,
}

/// The store this request is for.
pub struct GetTenant;

impl Operation for GetTenant {
    const META: OpMeta = OpMeta {
        address: "platform.tenant.get",
        tool: "get_tenant",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/tenants/{tenant_id}",
        summary: "The store: name, language, currency, time zone, status, feature flags, and how many policy values it overrides.",
        consumes: &[Slot::one("tenant_id")],
        produces: &[],
        checks: &[],
        errors: &[TENANT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetTenantInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetTenant {
    async fn read(&self, cx: &Cx, input: GetTenantInput) -> Result<Value, ToolError> {
        // Another store is out of reach for everyone: the token's tenant decides the store, and
        // row-level security hides every other store's rows.
        if input.tenant_id != cx.tenant.as_str() {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                format!(
                    "This request is for {}; another store's details are not available here.",
                    cx.tenant.as_str()
                ),
            )
            .detail(json!({"reason": "tenant", "id": input.tenant_id})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = sqlx::query_as::<_, StoreRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM platform.online_store WHERE id = $1"
        )))
        .bind(&input.tenant_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?
        .ok_or_else(|| {
            ToolError::new(
                TENANT_NOT_FOUND,
                format!("There is no store {}.", input.tenant_id),
            )
            .detail(json!({"entity": "tenant", "id": input.tenant_id}))
            .next("list_tenants")
        })?;
        let overrides = overridden_keys(&mut tx).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(crate::project(
            store(&row, overrides),
            &input.fields.unwrap_or_default(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn the_input_is_closed_and_names_the_store() {
        let s = closed_schema_for::<GetTenantInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["tenant_id"]));
        assert_eq!(
            s["properties"]["tenant_id"]["pattern"],
            "^store_[a-z0-9_]+$"
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn reads_the_store_of_the_request_only() {
        let cx = crate::testdb::admin().await;
        let t = GetTenant
            .read(
                &cx,
                serde_json::from_value(json!({"tenant_id": "store_northwind"})).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(t["name"], "Northwind Apparel");
        assert_eq!(t["acom:time_zone"], "America/New_York");
        assert_eq!(t["currency"], "USD");
        assert_eq!(t["acom:status"], "active");
        assert!(t["acom:policy_overrides"].as_u64().unwrap() >= 2, "{t}");
        let e = GetTenant
            .read(
                &cx,
                serde_json::from_value(json!({"tenant_id": "store_elsewhere"})).unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        assert_eq!(e.detail.unwrap()["reason"], "tenant");
        let only = GetTenant
            .read(
                &cx,
                serde_json::from_value(
                    json!({"tenant_id": "store_northwind", "fields": ["currency"]}),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(only, json!({"currency": "USD"}));
    }
}
