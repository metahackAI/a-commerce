//! `update_product` (docs/api/catalog.md#product).

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::SaleType;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{VariantOptions, approval_from, customer_facing, gift_check};
use crate::write::{self, VariantFields};
use crate::{BELOW_PRICE_FLOOR, PRODUCT_NOT_FOUND, product_not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProductInput {
    /// The variant to change.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_windbreaker_m_slate")]
    pub variant_id: String,
    /// New variant name.
    #[schemars(length(min = 1, max = 200), example = &"Packable windbreaker, Slate, M")]
    pub name: Option<String>,
    /// Image URLs of the variant; the first is used.
    #[schemars(example = ["https://cdn.northwind.example/p/windbreaker-slate.jpg"])]
    pub image: Option<Vec<String>>,
    /// New color and size. A structural change on a product shoppers can see.
    #[schemars(example = &serde_json::json!({"color": "Slate", "size": "M"}))]
    pub additional_property: Option<VariantOptions>,
    /// Whether stock is tracked.
    #[serde(rename = "acom:managed", alias = "managed")]
    #[schemars(example = true)]
    pub managed: Option<bool>,
    /// How it is sold. always: from stock; pre_order: before stock arrives; subscription: on a
    /// schedule; gift_card: a digital gift card. A structural change.
    #[serde(rename = "acom:sale_type", alias = "sale_type")]
    #[schemars(example = &"pre_order")]
    pub sale_type: Option<SaleType>,
}

/// What `update_product` runs, with what it replaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateVariantPlan {
    pub variant_id: String,
    pub to: VariantFields,
    pub from: VariantFields,
}

/// Change one variant.
#[derive(Debug)]
pub struct UpdateProduct;

impl Operation for UpdateProduct {
    const META: OpMeta = OpMeta {
        address: "catalog.product.update",
        tool: "update_product",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/catalog/variants/{variant_id}",
        summary: "Change a variant's name, image, color and size, stock tracking or sale type. Content needs no approval; size, color or sale type on a product shoppers can see does. Prices are set_price.",
        consumes: &[Slot::one("variant_id")],
        produces: &[Slot::one("variant_id")],
        checks: &["product.approval", "gift_card.disallow"],
        errors: &[
            ErrorCode::INVALID_STATE,
            BELOW_PRICE_FLOOR,
            PRODUCT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateProductInput;
    type Output = Value;
}

#[derive(Debug, sqlx::FromRow)]
struct VariantRow {
    id: String,
    name: String,
    color: Option<String>,
    size: Option<String>,
    image: Option<String>,
    sale_type: String,
    is_managed: bool,
    version: i32,
    approval_status: String,
}

async fn variant_row(tx: &mut TxCx, id: &str) -> Result<Option<VariantRow>, ToolError> {
    sqlx::query_as::<_, VariantRow>(
        "SELECT p.id, p.name, p.color, p.size, p.image, p.sale_type,
                p.is_managed, p.version, g.approval_status
         FROM catalog.product p
         JOIN catalog.product_group g ON g.tenant_id = p.tenant_id AND g.id = p.product_group_id
         WHERE p.tenant_id = $1 AND p.id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(crate::db)
}

impl WriteOp<Cx> for UpdateProduct {
    type Plan = UpdateVariantPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateProductInput,
    ) -> Result<Planned<UpdateVariantPlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let v = variant_row(&mut tx, &input.variant_id)
            .await?
            .ok_or_else(|| product_not_found("variant", &input.variant_id, "search_products"))?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut to = VariantFields::default();
        let mut from = VariantFields::default();
        let (mut cf, mut ct) = (Map::new(), Map::new());
        let mut structural = false;
        if let Some(n) = input
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty() && *n != v.name)
        {
            cf.insert("name".into(), json!(v.name));
            ct.insert("name".into(), json!(n));
            from.name = Some(v.name.clone());
            to.name = Some(n.to_owned());
        }
        if let Some(i) = input
            .image
            .as_ref()
            .and_then(|l| l.iter().map(|s| s.trim()).find(|s| !s.is_empty()))
            .filter(|i| Some(*i) != v.image.as_deref())
        {
            cf.insert("image".into(), json!(v.image));
            ct.insert("image".into(), json!(i));
            from.image = Some(v.image.clone());
            to.image = Some(Some(i.to_owned()));
        }
        if let Some(o) = &input.additional_property {
            if o.color.is_some() && o.color != v.color {
                structural = true;
                cf.insert("color".into(), json!(v.color));
                ct.insert("color".into(), json!(o.color));
                from.color = Some(v.color.clone());
                to.color = Some(o.color.clone());
            }
            if o.size.is_some() && o.size != v.size {
                structural = true;
                cf.insert("size".into(), json!(v.size));
                ct.insert("size".into(), json!(o.size));
                from.size = Some(v.size.clone());
                to.size = Some(o.size.clone());
            }
        }
        if let Some(m) = input.managed.filter(|m| *m != v.is_managed) {
            cf.insert("acom:managed".into(), json!(v.is_managed));
            ct.insert("acom:managed".into(), json!(m));
            from.is_managed = Some(v.is_managed);
            to.is_managed = Some(m);
        }
        let mut sale_types = vec![v.sale_type.clone()];
        if let Some(s) = input
            .sale_type
            .map(|s| s.as_str().to_owned())
            .filter(|s| *s != v.sale_type)
        {
            structural = true;
            cf.insert("acom:sale_type".into(), json!(v.sale_type));
            ct.insert("acom:sale_type".into(), json!(s));
            from.sale_type = Some(v.sale_type.clone());
            to.sale_type = Some(s.clone());
            sale_types = vec![s];
        }
        if ct.is_empty() {
            return Err(ToolError::invalid_input(format!(
                "Nothing to change: {} already has these values.",
                v.id
            ))
            .detail(json!({"field": "variant_id", "reason": "missing"})));
        }
        let live = customer_facing(&v.approval_status);
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "variant".into(),
                id: v.id.clone(),
                from: Value::Object(cf),
                to: Value::Object(ct),
                note: None,
            }],
            checks: vec![
                Check {
                    rule: "product.approval".into(),
                    passed: true,
                    detail: Some(if structural && live {
                        "Changes the structure of a variant shoppers can see; a person approves it"
                            .into()
                    } else {
                        "No approval needed for this change".into()
                    }),
                    fix_tool: None,
                },
                gift_check(&cx.policy, &sale_types),
            ],
            basis: vec![Basis {
                entity: "catalog.product".into(),
                id: v.id.clone(),
                version: i64::from(v.version),
            }],
            approval: approval_from(
                cx,
                Self::META.tool,
                structural && live,
                "Changes a variant shoppers can see".into(),
            ),
            plan: UpdateVariantPlan {
                variant_id: v.id,
                to,
                from,
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &UpdateVariantPlan,
    ) -> Result<Committed, ToolError> {
        if write::update_variant(tx, &plan.variant_id, &plan.to).await? == 0 {
            return Err(product_not_found(
                "variant",
                &plan.variant_id,
                "search_products",
            ));
        }
        let mut produced = Map::new();
        produced.insert("variant_id".into(), json!(plan.variant_id));
        Ok(Committed {
            id: plan.variant_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &UpdateVariantPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let v = variant_row(&mut tx, &plan.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match v {
            None => mismatches.push(format!("{} is missing", plan.variant_id)),
            Some(v) => {
                if plan.to.name.as_ref().is_some_and(|n| *n != v.name) {
                    mismatches.push(format!("{} is named {}", v.id, v.name));
                }
                if plan
                    .to
                    .sale_type
                    .as_ref()
                    .is_some_and(|s| *s != v.sale_type)
                {
                    mismatches.push(format!("{} is sold as {}", v.id, v.sale_type));
                }
            }
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &UpdateVariantPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        write::update_variant(tx, &plan.variant_id, &plan.from)
            .await
            .map(|_| ())
    }
}
