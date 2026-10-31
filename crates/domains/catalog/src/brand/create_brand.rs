//! `create_brand` (docs/api/catalog.md#brand).

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, ErrorCode, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{DUPLICATE_BRAND, duplicate, logo_of};
use crate::slug;
use crate::write::{self};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateBrandInput {
    /// The brand's name, unique in the store.
    #[schemars(length(min = 1, max = 100), example = &"Harbor Knit Co.")]
    pub name: String,
    /// URL of the logo image.
    #[schemars(example = &"https://cdn.northwind.example/brand/harbor-knit.svg")]
    pub logo: Option<String>,
}

/// What `create_brand` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateBrandPlan {
    pub brand_id: String,
    pub name: String,
    pub logo: Option<String>,
}

/// Add a brand.
#[derive(Debug)]
pub struct CreateBrand;

impl Operation for CreateBrand {
    const META: OpMeta = OpMeta {
        address: "catalog.brand.create",
        tool: "create_brand",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/brands",
        summary: "Add a brand products can be sold under.",
        consumes: &[],
        produces: &[Slot::one("brand_id")],
        checks: &[],
        errors: &[DUPLICATE_BRAND],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateBrandInput;
    type Output = Value;
}

impl WriteOp<Cx> for CreateBrand {
    type Plan = CreateBrandPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateBrandInput,
    ) -> Result<Planned<CreateBrandPlan>, ToolError> {
        let name = input.name.trim().to_owned();
        if name.is_empty() {
            return Err(ToolError::invalid_input("A brand needs a name.")
                .detail(json!({"field": "name", "reason": "missing"})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if let Some(existing) = write::brand_named(&mut tx, &name).await? {
            return Err(duplicate(&name, &existing));
        }
        let base = format!("brn_{}", slug(&name));
        let mut brand_id = base.clone();
        for n in 2..50 {
            if write::brand_row(&mut tx, &brand_id).await?.is_none() {
                break;
            }
            brand_id = format!("{base}_{n}");
        }
        tx.commit().await.map_err(ToolError::from)?;
        let plan = CreateBrandPlan {
            brand_id,
            name,
            logo: logo_of(input.logo.as_deref()),
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "brand".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"brand_id": plan.brand_id, "name": plan.name, "logo": plan.logo}),
                note: None,
            }],
            checks: Vec::new(),
            basis: Vec::new(),
            approval: crate::policy_approval(
                cx,
                Self::META.tool,
                &json!({"change_type": "content"}),
            ),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &CreateBrandPlan) -> Result<Committed, ToolError> {
        if let Some(existing) = write::brand_named(tx, &plan.name).await? {
            if existing != plan.brand_id {
                return Err(duplicate(&plan.name, &existing));
            }
        } else {
            write::insert_brand(tx, &plan.brand_id, &plan.name, plan.logo.as_deref()).await?;
        }
        let mut produced = Map::new();
        produced.insert("brand_id".into(), json!(plan.brand_id));
        Ok(Committed {
            id: plan.brand_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CreateBrandPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let b = write::brand_row(&mut tx, &plan.brand_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = b.is_some_and(|b| b.name == plan.name);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not {}", plan.brand_id, plan.name)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &CreateBrandPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        if write::delete_unused_brand(tx, &plan.brand_id).await? {
            Ok(())
        } else {
            Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("Products use {} now; it stays.", plan.brand_id),
            )
            .detail(json!({"entity": "brand", "id": plan.brand_id}))
            .next("search_product_groups"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brand::update_brand::UpdateBrandInput;
    use crate::brand::{duplicate, logo_of};

    #[test]
    fn inputs_are_closed_and_names_are_required() {
        for schema in [
            acom_contract::schema::closed_schema_for::<CreateBrandInput>(),
            acom_contract::schema::closed_schema_for::<UpdateBrandInput>(),
        ] {
            assert!(
                acom_contract::schema::violations(&schema).is_empty(),
                "{schema:#}"
            );
        }
        assert!(serde_json::from_value::<CreateBrandInput>(json!({"logo": "x"})).is_err());
        assert_eq!(logo_of(Some("  ")), None);
        assert_eq!(
            duplicate("Northwind", "brn_northwind").code,
            "duplicate_brand"
        );
    }

    #[tokio::test]
    async fn a_blank_name_is_refused() {
        let e = CreateBrand
            .preview(
                &crate::testing::cx(acom_core::enums::ActorKind::Operator, "adm_sj"),
                serde_json::from_value(json!({"name": " "})).unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }
}
