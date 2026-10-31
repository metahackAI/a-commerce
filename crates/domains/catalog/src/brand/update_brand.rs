//! `update_brand` (docs/api/catalog.md#brand).

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{DUPLICATE_BRAND, duplicate, logo_of};
use crate::BRAND_NOT_FOUND;
use crate::write::{self, BrandRow};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateBrandInput {
    /// The brand to change.
    #[schemars(regex(pattern = r"^brn_[a-z0-9_]+$"), example = &"brn_northwind")]
    pub brand_id: String,
    /// New name, unique in the store.
    #[schemars(length(min = 1, max = 100), example = &"Northwind")]
    pub name: Option<String>,
    /// New logo URL; empty text removes the logo.
    #[schemars(example = &"https://cdn.northwind.example/brand/northwind-2026.svg")]
    pub logo: Option<String>,
}

/// What `update_brand` runs, with what it replaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateBrandPlan {
    pub brand_id: String,
    pub name: String,
    pub logo: Option<String>,
    pub from_name: String,
    pub from_logo: Option<String>,
}

/// Rename a brand or change its logo.
#[derive(Debug)]
pub struct UpdateBrand;

impl Operation for UpdateBrand {
    const META: OpMeta = OpMeta {
        address: "catalog.brand.update",
        tool: "update_brand",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/catalog/brands/{brand_id}",
        summary: "Rename a brand or change its logo.",
        consumes: &[Slot::one("brand_id")],
        produces: &[Slot::one("brand_id")],
        checks: &[],
        errors: &[BRAND_NOT_FOUND, DUPLICATE_BRAND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateBrandInput;
    type Output = Value;
}

fn not_found(id: &str) -> ToolError {
    ToolError::new(BRAND_NOT_FOUND, format!("There is no brand {id}."))
        .detail(json!({"entity": "brand", "id": id}))
        .next("list_brands")
}

impl WriteOp<Cx> for UpdateBrand {
    type Plan = UpdateBrandPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateBrandInput,
    ) -> Result<Planned<UpdateBrandPlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let b: BrandRow = write::brand_row(&mut tx, &input.brand_id)
            .await?
            .ok_or_else(|| not_found(&input.brand_id))?;
        let name = input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map_or_else(|| b.name.clone(), str::to_owned);
        if !name.eq_ignore_ascii_case(&b.name)
            && let Some(existing) = write::brand_named(&mut tx, &name).await?
        {
            return Err(duplicate(&name, &existing));
        }
        tx.commit().await.map_err(ToolError::from)?;
        let logo = match &input.logo {
            Some(l) => logo_of(Some(l)),
            None => b.logo.clone(),
        };
        if name == b.name && logo == b.logo {
            return Err(ToolError::invalid_input(format!(
                "Nothing to change: {} already has these values.",
                b.id
            ))
            .detail(json!({"field": "brand_id", "reason": "missing"})));
        }
        let plan = UpdateBrandPlan {
            brand_id: b.id.clone(),
            name,
            logo,
            from_name: b.name.clone(),
            from_logo: b.logo.clone(),
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "brand".into(),
                id: b.id.clone(),
                from: json!({"name": plan.from_name, "logo": plan.from_logo}),
                to: json!({"name": plan.name, "logo": plan.logo}),
                note: None,
            }],
            checks: Vec::new(),
            basis: vec![Basis {
                entity: "catalog.brand".into(),
                id: b.id,
                version: i64::from(b.version),
            }],
            approval: crate::policy_approval(
                cx,
                Self::META.tool,
                &json!({"change_type": "content"}),
            ),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &UpdateBrandPlan) -> Result<Committed, ToolError> {
        if write::update_brand(tx, &plan.brand_id, &plan.name, plan.logo.as_deref()).await? == 0 {
            return Err(not_found(&plan.brand_id));
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
        plan: &UpdateBrandPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let b = write::brand_row(&mut tx, &plan.brand_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = b.is_some_and(|b| b.name == plan.name && b.logo == plan.logo);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} does not read back as {}",
                    plan.brand_id, plan.name
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &UpdateBrandPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        write::update_brand(
            tx,
            &plan.brand_id,
            &plan.from_name,
            plan.from_logo.as_deref(),
        )
        .await
        .map(|_| ())
    }
}
