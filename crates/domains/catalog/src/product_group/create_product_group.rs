//! `create_product_group` (docs/api/catalog.md#product_group): a new product starts as a draft.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{
    CreateGroupPlan, INVALID_CATEGORY, MISSING_REQUIRED_FIELD, OptionAxis, first_image,
    free_product_id, known_brand, max_axes_check, primary_category, standard_category, trimmed,
};
use crate::neighbors::{self};
use crate::rules::ProductRules;
use crate::write::{self, NewGroup};
use crate::{BRAND_NOT_FOUND, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProductGroupInput {
    /// The product's name as shoppers will see it.
    #[schemars(length(min = 1, max = 200), example = &"Packable windbreaker")]
    pub name: String,
    /// The description in the store's tone (`content.copy`). Can come later.
    #[schemars(length(max = 4000), example = &"A wind layer that packs into its own pocket. Light enough to forget until the weather turns. Machine wash cold.")]
    pub description: Option<String>,
    /// The brand (`list_brands`).
    #[schemars(regex(pattern = r"^brn_[a-z0-9_]+$"), example = &"brn_northwind")]
    pub brand_id: Option<String>,
    /// The standard category (`list_categories` with term_set standard).
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub category_id: String,
    /// Image URLs; the first is the main image.
    #[schemars(example = ["https://cdn.northwind.example/p/windbreaker-slate.jpg"])]
    pub image: Option<Vec<String>>,
    /// Option axes of its variants. color: variants differ in color; size: in size.
    #[schemars(example = ["color", "size"])]
    pub varies_by: Option<Vec<OptionAxis>>,
    /// Main material shown to shoppers.
    #[schemars(length(max = 200), example = &"Recycled nylon")]
    pub material: Option<String>,
}

/// Register a new product as a draft.
#[derive(Debug)]
pub struct CreateProductGroup;

impl Operation for CreateProductGroup {
    const META: OpMeta = OpMeta {
        address: "catalog.product_group.create",
        tool: "create_product_group",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/product-groups",
        summary: "Register a new product as a draft in a standard category. Shoppers do not see it; add variants with set_variants and publish with approve_product.",
        consumes: &[Slot::one("brand_id"), Slot::one("category_id")],
        produces: &[Slot::one("product_id")],
        checks: &["product.required_on_create", "variant.max_option_axes"],
        errors: &[MISSING_REQUIRED_FIELD, INVALID_CATEGORY, BRAND_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateProductGroupInput;
    type Output = Value;
}

impl WriteOp<Cx> for CreateProductGroup {
    type Plan = CreateGroupPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateProductGroupInput,
    ) -> Result<Planned<CreateGroupPlan>, ToolError> {
        let rules = ProductRules::from_policy(&cx.policy)?;
        let Some(name) = trimmed(Some(&input.name)) else {
            return Err(
                ToolError::new(MISSING_REQUIRED_FIELD, "A product needs a name.")
                    .detail(json!({"field": "name", "reason": "missing"}))
                    .next("create_product_group"),
            );
        };
        let mut varies_by: Vec<String> = Vec::new();
        for a in input.varies_by.unwrap_or_default() {
            if !varies_by.iter().any(|x| x == a.as_str()) {
                varies_by.push(a.as_str().to_owned());
            }
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let category = standard_category(&mut tx, &input.category_id).await?;
        if let Some(b) = &input.brand_id {
            known_brand(&mut tx, b).await?;
        }
        let product_id = free_product_id(&mut tx, &name).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let plan = CreateGroupPlan {
            product_id,
            name,
            description: trimmed(input.description.as_deref()),
            brand_id: input.brand_id,
            category_id: category.id.clone(),
            image: first_image(input.image.as_ref()),
            varies_by,
            material: trimmed(input.material.as_deref()),
        };
        let mut checks = vec![Check {
            rule: "product.required_on_create".into(),
            passed: true,
            detail: Some(format!(
                "It has {}; the store is the seller",
                rules
                    .required_on_create
                    .iter()
                    .filter(|r| *r != "seller")
                    .map(|r| match r.as_str() {
                        "category" => format!("the category {}", category.name),
                        "name" => "a name".to_owned(),
                        other => other.replace('_', " "),
                    })
                    .collect::<Vec<_>>()
                    .join(" and ")
            )),
            fix_tool: None,
        }];
        checks.extend(max_axes_check(&cx.policy, plan.varies_by.len()));
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "product".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"product_id": plan.product_id, "name": plan.name,
                           "description": plan.description, "brand_id": plan.brand_id,
                           "category_id": plan.category_id, "image": plan.image,
                           "varies_by": plan.varies_by, "material": plan.material,
                           "approval_status": "draft"}),
                note: Some("A draft: shoppers do not see it until it is approved.".into()),
            }],
            checks,
            basis: Vec::new(),
            approval: crate::policy_approval(
                cx,
                Self::META.tool,
                &json!({"change_type": "content"}),
            ),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &CreateGroupPlan) -> Result<Committed, ToolError> {
        if write::existing_groups(tx, std::slice::from_ref(&plan.product_id))
            .await?
            .is_empty()
        {
            write::insert_group(
                tx,
                &NewGroup {
                    id: &plan.product_id,
                    name: &plan.name,
                    description: plan.description.as_deref(),
                    brand_id: plan.brand_id.as_deref(),
                    varies_by: &plan.varies_by,
                    material: plan.material.as_deref(),
                    image: plan.image.as_deref(),
                },
            )
            .await?;
            neighbors::place_product(tx, &plan.product_id, &plan.category_id).await?;
        }
        let mut produced = Map::new();
        produced.insert("product_id".into(), json!(plan.product_id));
        Ok(Committed {
            id: plan.product_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CreateGroupPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let g = repo::group(&mut tx, &plan.product_id).await?;
        let category = primary_category(&mut tx, &plan.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match g {
            None => mismatches.push(format!("{} was not created", plan.product_id)),
            Some(g) if g.name != plan.name => {
                mismatches.push(format!("{} is named {}", g.id, g.name))
            }
            Some(_) => {}
        }
        if category.as_deref() != Some(plan.category_id.as_str()) {
            mismatches.push(format!(
                "{} is not in {}",
                plan.product_id, plan.category_id
            ));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &CreateGroupPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        let variants = repo::variants(tx, std::slice::from_ref(&plan.product_id)).await?;
        if !variants.is_empty() {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} has variants now; take those back first.",
                    plan.product_id
                ),
            )
            .detail(json!({"entity": "product", "id": plan.product_id}))
            .next("get_product_group"));
        }
        if !write::delete_draft(tx, &plan.product_id).await? {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("{} is no longer a draft; it stays.", plan.product_id),
            )
            .detail(json!({"entity": "product", "id": plan.product_id}))
            .next("get_change_history"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product_group::OptionAxis;
    use crate::product_group::update_product_group::UpdateProductGroupInput;
    use crate::testing;

    #[test]
    fn inputs_are_closed_and_axes_are_words() {
        for schema in [
            acom_contract::schema::closed_schema_for::<CreateProductGroupInput>(),
            acom_contract::schema::closed_schema_for::<UpdateProductGroupInput>(),
        ] {
            assert!(
                acom_contract::schema::violations(&schema).is_empty(),
                "{schema:#}"
            );
        }
        let i: CreateProductGroupInput = serde_json::from_value(json!({
            "name": "Packable windbreaker", "category_id": "cat_summer_outerwear",
            "varies_by": ["color", "size"]}))
        .unwrap();
        assert_eq!(i.varies_by.unwrap()[1], OptionAxis::Size);
        assert!(
            serde_json::from_value::<CreateProductGroupInput>(
                json!({"name": "x", "category_id": "cat_x", "varies_by": ["fit"]})
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn a_blank_name_is_refused_before_any_lookup() {
        let e = CreateProductGroup
            .preview(
                &testing::cx(acom_core::enums::ActorKind::Operator, "adm_sj"),
                serde_json::from_value(json!({"name": "  ", "category_id": "cat_outerwear"}))
                    .unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("missing_required_field", Some("create_product_group"))
        );
    }
}
