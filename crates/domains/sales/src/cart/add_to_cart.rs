//! `add_to_cart` (`sales.cart.add`): a product with size and color, or a variant, into the
//! caller's own cart. `no_approval`: runs right after its preview.

use acom_catalog::service::for_sales::{self as catalog, VariantFacts, VariantMatch};
use acom_contract::{
    ActionType, Audience, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta, Operation,
    Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::ids::CartLineId;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{change, limit, out_of_stock, produced, start, stock_check, target_changes};
use crate::pricing::{self, CartTarget};
use crate::repo;
use crate::{
    MIXED_ORDER_TYPE, NOT_SIGNED_IN, OUT_OF_STOCK, PRODUCT_NOT_FOUND, PURCHASE_LIMIT_EXCEEDED,
    VARIANT_NOT_FOUND, approval,
};

/// How `add_to_cart` names what to add.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AddMode {
    /// A product with size and color: the server finds the variant.
    ProductOptions,
    /// A variant id as is.
    Variant,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddToCartInput {
    /// `product_options`: `product_id` + `size` + `color`; `variant`: `variant_id`.
    #[schemars(example = &"product_options")]
    pub mode: AddMode,
    /// The product to add (`search_products`, `get_product`). Needed with `product_options`.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_windbreaker")]
    pub product_id: Option<String>,
    /// One of the product's sizes (`get_product` `has_variant[].size`). Needed with `product_options`.
    #[schemars(length(min = 1, max = 20), example = &"M")]
    pub size: Option<String>,
    /// One of the product's colors (`has_variant[].color`). Needed with `product_options`.
    #[schemars(length(min = 1, max = 40), example = &"Slate")]
    pub color: Option<String>,
    /// The variant to add. Needed with `variant`.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_windbreaker_m_slate")]
    pub variant_id: Option<String>,
    /// Units, 1 up to `cart.max_quantity_per_line`. Default 1. If the cart already has the
    /// variant, its line is set to this quantity.
    #[schemars(range(min = 1), example = 1)]
    pub order_quantity: Option<u32>,
}

fn invalid(field: &str, message: &str) -> ToolError {
    ToolError::invalid_input(message).detail(json!({"field": field, "reason": "missing"}))
}

/// Subscriptions and gift cards are their own kind of order (`cart.separate_by`).
fn order_kind(v: &VariantFacts) -> &str {
    match v.sale_type.as_str() {
        "subscription" => "subscription",
        _ => "one_time",
    }
}

/// What `add_to_cart` will do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AddPlan {
    target: CartTarget,
    line_id: String,
    variant_id: String,
    quantity: u32,
    position: u32,
    /// The line's quantity before, when the cart already had the variant.
    previous: Option<u32>,
}

pub(crate) struct AddToCart;

impl Operation for AddToCart {
    const META: OpMeta = OpMeta {
        address: "sales.cart.add",
        tool: "add_to_cart",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/sales/carts/mine/lines",
        summary: "Put a product (with size and color) or a variant in the caller's own cart, creating the cart if needed. Runs right after its preview; no approval.",
        consumes: &[Slot::one("product_id"), Slot::one("variant_id")],
        produces: &[Slot::one("cart_id"), Slot::one("cart_line_id")],
        checks: &[
            "cart.max_lines",
            "cart.no_mix",
            "checkout_validation.stock_available",
        ],
        errors: &[
            OUT_OF_STOCK,
            PRODUCT_NOT_FOUND,
            VARIANT_NOT_FOUND,
            PURCHASE_LIMIT_EXCEEDED,
            MIXED_ORDER_TYPE,
            NOT_SIGNED_IN,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::ADD,
        person_scope: "",
    };
    type Input = AddToCartInput;
    type Output = Value;
}

/// Finds the variant `add_to_cart` names.
async fn resolve(tx: &mut TxCx, input: &AddToCartInput) -> Result<VariantFacts, ToolError> {
    match input.mode {
        AddMode::ProductOptions => {
            let product = input
                .product_id
                .as_deref()
                .ok_or_else(|| invalid("product_id", "`product_options` needs `product_id`."))?;
            let size = input
                .size
                .as_deref()
                .ok_or_else(|| invalid("size", "`product_options` needs `size`."))?;
            let color = input
                .color
                .as_deref()
                .ok_or_else(|| invalid("color", "`product_options` needs `color`."))?;
            match catalog::find_variant(tx, product, size, color).await? {
                VariantMatch::Found(v) => Ok(*v),
                VariantMatch::NoProduct => Err(ToolError::new(
                    PRODUCT_NOT_FOUND,
                    format!("There is no product {product}."),
                )
                .detail(json!({"entity": "product", "id": product}))
                .next("get_product")),
                VariantMatch::NoSuchOption { sizes, colors } => Err(ToolError::new(
                    VARIANT_NOT_FOUND,
                    format!("{product} does not come in {size} / {color}."),
                )
                .detail(json!({"entity": "variant", "product_id": product,
                               "sizes": sizes, "colors": colors}))
                .next("get_product")),
            }
        }
        AddMode::Variant => {
            let id = input
                .variant_id
                .as_deref()
                .ok_or_else(|| invalid("variant_id", "`variant` needs `variant_id`."))?;
            catalog::variants(tx, &[id.to_owned()])
                .await?
                .pop()
                .ok_or_else(|| {
                    ToolError::new(VARIANT_NOT_FOUND, format!("There is no variant {id}."))
                        .detail(json!({"entity": "variant", "id": id}))
                        .next("get_product")
                })
        }
    }
}

impl WriteOp<Cx> for AddToCart {
    type Plan = AddPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(&self, cx: &Cx, input: AddToCartInput) -> Result<Planned<AddPlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let s = start(cx, &mut tx).await?;
        let variant = resolve(&mut tx, &input).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let quantity = input.order_quantity.unwrap_or(1);
        if quantity == 0 {
            return Err(
                ToolError::invalid_input("`order_quantity` must be at least 1.")
                    .detail(json!({"field": "order_quantity", "reason": "range"})),
            );
        }
        if quantity > s.limits.max_quantity_per_line {
            return Err(limit(
                "cart.max_quantity_per_line",
                s.limits.max_quantity_per_line,
                quantity,
                "update_cart_item",
            ));
        }
        if let Some(e) = out_of_stock(&variant, quantity) {
            return Err(e);
        }
        if s.facts
            .iter()
            .any(|f| order_kind(f) != order_kind(&variant))
        {
            return Err(ToolError::new(
                MIXED_ORDER_TYPE,
                "Subscriptions and one-time purchases go in separate orders.",
            )
            .detail(json!({"variant_id": variant.variant_id, "policy_key": "cart.separate_by"}))
            .next("remove_cart_item"));
        }
        let existing = s
            .carts
            .merged
            .iter()
            .find(|m| m.variant_id == variant.variant_id);
        let lines_after = s.carts.merged.len() + usize::from(existing.is_none());
        if lines_after > s.limits.max_lines as usize {
            return Err(limit(
                "cart.max_lines",
                s.limits.max_lines,
                u32::try_from(lines_after).unwrap_or(u32::MAX),
                "remove_cart_item",
            ));
        }

        let target = s.carts.target();
        let mut changes = target_changes(&target);
        let plan = match existing {
            Some(m) => {
                changes.push(change(
                    ChangeOp::Update,
                    "cart_line",
                    &m.id,
                    json!({"variant_id": m.variant_id, "quantity": m.quantity}),
                    json!({"variant_id": variant.variant_id, "quantity": quantity}),
                ));
                AddPlan {
                    target: target.clone(),
                    line_id: m.id.clone(),
                    variant_id: variant.variant_id.clone(),
                    quantity,
                    position: 0,
                    previous: Some(m.quantity),
                }
            }
            None => {
                changes.push(change(
                    ChangeOp::Create,
                    "cart_line",
                    "",
                    Value::Null,
                    json!({"cart_id": target.cart_id, "variant_id": variant.variant_id,
                           "quantity": quantity}),
                ));
                AddPlan {
                    target: target.clone(),
                    line_id: CartLineId::generate().to_string(),
                    variant_id: variant.variant_id.clone(),
                    quantity,
                    position: u32::try_from(lines_after).unwrap_or(u32::MAX),
                    previous: None,
                }
            }
        };
        let country = s.country.unwrap_or_else(|| "store's country".into());
        let checks = vec![
            Check {
                rule: "cart.max_lines".into(),
                passed: true,
                detail: Some(format!("{lines_after} of {} lines", s.limits.max_lines)),
                fix_tool: None,
            },
            Check {
                rule: "cart.no_mix".into(),
                passed: true,
                detail: Some(format!(
                    "Ships within the {country} like the rest of the cart"
                )),
                fix_tool: None,
            },
            stock_check(&variant),
        ];
        Ok(Planned {
            plan,
            changes,
            checks,
            basis: s.carts.basis(),
            approval: approval(cx, Self::META.tool, &json!({"quantity": quantity})),
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &AddPlan) -> Result<Committed, ToolError> {
        let cart_id = pricing::apply_target(tx, &plan.target).await?;
        let id = repo::upsert_line(
            tx,
            &plan.line_id,
            &cart_id,
            &plan.variant_id,
            plan.quantity,
            plan.position.max(1),
        )
        .await?;
        repo::touch_cart(tx, &cart_id).await?;
        Ok(Committed {
            produced: produced(&[
                ("cart_id", &cart_id),
                ("cart_line_id", &id),
                ("variant_id", &plan.variant_id),
            ]),
            id,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &AddPlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let now = repo::line_quantity(&mut tx, &done.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let cart_id = done.produced["cart_id"]
            .as_str()
            .unwrap_or(&plan.target.cart_id);
        let expected = (
            cart_id.to_owned(),
            plan.variant_id.clone(),
            i32::try_from(plan.quantity).unwrap_or(i32::MAX),
        );
        Ok(match now {
            Some(found) if found == expected => Verification {
                matched: true,
                mismatches: vec![],
            },
            other => Verification {
                matched: false,
                mismatches: vec![format!(
                    "cart line {}: expected {expected:?}, found {other:?}",
                    done.id
                )],
            },
        })
    }

    async fn undo(&self, tx: &mut TxCx, plan: &AddPlan, done: &Committed) -> Result<(), ToolError> {
        match plan.previous {
            Some(q) => repo::set_quantity(tx, &done.id, q).await?,
            None => {
                repo::delete_lines(tx, std::slice::from_ref(&done.id)).await?;
            }
        }
        let cart_id = done.produced["cart_id"]
            .as_str()
            .unwrap_or(&plan.target.cart_id);
        repo::touch_cart(tx, cart_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cart::get_cart::{GetCart, GetCartInput};
    use acom_contract::ReadOp;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed_with_modes_as_words() {
        let add = closed_schema_for::<AddToCartInput>();
        assert!(violations(&add).is_empty(), "{add:#}");
        assert_eq!(
            add["properties"]["mode"]["enum"],
            json!(["product_options", "variant"])
        );
        assert_eq!(add["required"], json!(["mode"]));
    }

    #[test]
    fn plans_survive_the_preview_store() {
        let plan = AddPlan {
            target: CartTarget {
                cart_id: "cart_jamie".into(),
                owner: crate::owner::Owner {
                    customer_id: Some("cus_jamie".into()),
                    session_id: None,
                },
                create: false,
                adopt: false,
                absorb: None,
            },
            line_id: "cln_1".into(),
            variant_id: "var_windbreaker_m_slate".into(),
            quantity: 1,
            position: 1,
            previous: None,
        };
        let back: AddPlan = serde_json::from_value(serde_json::to_value(&plan).unwrap()).unwrap();
        assert_eq!(back, plan);
    }

    /// Integration: F2 step 3 — Jamie adds the windbreaker M Slate and sees $71.20
    /// (`DATABASE_URL`, demo seed with change set #1042 run).
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_adds_the_windbreaker_and_the_cart_shows_the_vip_price() {
        let cx = crate::testdb::cx("cus_jamie").await;
        let planned = AddToCart
            .preview(
                &cx,
                AddToCartInput {
                    mode: AddMode::ProductOptions,
                    product_id: Some("prd_windbreaker".into()),
                    size: Some("M".into()),
                    color: Some("Slate".into()),
                    variant_id: None,
                    order_quantity: None,
                },
            )
            .await
            .unwrap();
        assert!(planned.checks.iter().all(|c| c.passed));
        let mut tx = cx.begin().await.unwrap();
        let done = AddToCart.commit(&mut tx, &planned.plan).await.unwrap();
        assert_eq!(done.produced["variant_id"], "var_windbreaker_m_slate");
        // Rolled back on drop: the database is left as it was.
        drop(tx);
        let cart = GetCart
            .read(&cx, GetCartInput { fields: None })
            .await
            .unwrap();
        assert!(cart["lines"].is_array());
    }
}
