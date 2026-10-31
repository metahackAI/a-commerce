//! `get_import_status` (`integration.product_import.get_import_status`): how an import went,
//! row by row: what was taken, what was rejected and why, and the tool that fixes it. The basis
//! for sending rows again.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{IMPORT_NOT_FOUND, check_id, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetImportStatusInput {
    /// The import (`import_products` returns it).
    #[schemars(regex(pattern = r"^imp_[a-z0-9_]+$"), example = &"imp_1107")]
    pub import_id: String,
    /// Top-level fields to return (`id` always comes back); all of them when left out.
    #[schemars(example = ["action_status", "rejected", "rows"])]
    pub fields: Option<Vec<String>>,
}

/// Read how an import went.
#[derive(Debug)]
pub struct GetImportStatus;

impl Operation for GetImportStatus {
    const META: OpMeta = OpMeta {
        address: "integration.product_import.get_import_status",
        tool: "get_import_status",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/integration/product-imports/{import_id}",
        summary: "How a product import went, row by row: accepted or rejected, the product each row made or changed, and for a rejected row the reason and the tool that fixes it.",
        consumes: &[Slot::one("import_id")],
        produces: &[Slot::many("product_id")],
        checks: &[],
        errors: &[IMPORT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetImportStatusInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetImportStatus {
    async fn read(&self, cx: &Cx, input: GetImportStatusInput) -> Result<Value, ToolError> {
        check_id("import_id", "imp", &input.import_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let import = repo::import(&mut tx, &input.import_id).await?;
        let Some(import) = import else {
            return Err(ToolError::new(
                IMPORT_NOT_FOUND,
                format!("There is no import {} in this store.", input.import_id),
            )
            .detail(json!({"entity": "product_import", "id": input.import_id}))
            .next("import_products"));
        };
        let items = repo::import_items(&mut tx, &import.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let accepted = items.iter().filter(|i| i.item_status == "accepted").count();
        let rows: Vec<Value> = items
            .iter()
            .map(|i| {
                json!({"position": i.position, "external_id": i.external_id,
                       "product_id": i.product_group_id, "acom:item_status": i.item_status,
                       "error": i.error})
            })
            .collect();
        let out = json!({
            "id": import.id,
            "seller_id": import.seller_id,
            "acom:import_mode": import.import_mode,
            "action_status": import.action_status,
            "accepted": accepted,
            "rejected": items.len() - accepted,
            "start_time": cx.time.format(import.start_time),
            "end_time": import.end_time.map(|t| cx.time.format(t)),
            "rows": rows,
        });
        acom_contract::fields::keep(out, input.fields.as_deref().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn the_input_is_closed() {
        let s = closed_schema_for::<GetImportStatusInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["import_id"]));
    }

    #[tokio::test]
    async fn a_malformed_id_is_refused_before_any_query() {
        let e = GetImportStatus
            .read(
                &crate::testing::offline("adm_sj"),
                serde_json::from_value(json!({"import_id": "1107"})).unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[tokio::test]
    #[ignore = "integration: needs the demo database (DATABASE_URL)"]
    async fn shows_the_seeded_import_row_by_row() {
        let cx = crate::testing::db("adm_sj").await;
        let out = GetImportStatus
            .read(
                &cx,
                serde_json::from_value(json!({"import_id": "imp_1107"})).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(out["action_status"], "completed");
        assert_eq!(
            (out["accepted"].as_u64(), out["rejected"].as_u64()),
            (Some(1), Some(1))
        );
        assert_eq!(out["rows"][0]["product_id"], "prd_merino_tee");
        assert_eq!(out["rows"][1]["error"]["code"], "category_unmapped");
        let e = GetImportStatus
            .read(
                &cx,
                serde_json::from_value(json!({"import_id": "imp_nothing"})).unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("import_not_found", Some("import_products"))
        );
    }
}
