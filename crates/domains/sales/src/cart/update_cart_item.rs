//! `update_cart_item` (`sales.cart.update`): another quantity, size or color for a line of the
//! caller's own cart. `no_approval`: runs right after its preview.

use acom_catalog::service::for_sales::{self as catalog, VariantMatch};
use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, Group, Kind, Method, OpMeta, Operation,
    Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{change, limit, out_of_stock, produced, start, stock_check, target_changes};
use crate::pricing::{self, CartTarget};
use crate::repo::{self, LineSnapshot};
use crate::{
    CART_LINE_NOT_FOUND, NOT_SIGNED_IN, OUT_OF_STOCK, PURCHASE_LIMIT_EXCEEDED, VARIANT_NOT_FOUND,
    approval,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCartItemInput {
    /// The line to change (`get_cart`).
    #[schemars(regex(pattern = r"^cln_[a-z0-9_]+$"), example = &"cln_1")]
    pub cart_line_id: String,
    /// New quantity, at least 1.
    #[schemars(range(min = 1), example = 2)]
    pub order_quantity: Option<u32>,
    /// Another size of the same product.
    #[schemars(length(min = 1, max = 20), example = &"L")]
    pub size: Option<String>,
    /// Another color of the same product.
    #[schemars(length(min = 1, max = 40), example = &"Black")]
    pub color: Option<String>,
}

/// What `update_cart_item` will do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UpdatePlan {
    target: CartTarget,
    line_id: String,
    from_variant: String,
    from_quantity: u32,
    variant_id: String,
    quantity: u32,
    /// Another line already holds the new variant: this line folds into it.
    fold_into: Option<(String, u32)>,
}

pub(crate) struct UpdateCartItem;

impl Operation for UpdateCartItem {
    const META: OpMeta = OpMeta {
        address: "sales.cart.update",
        tool: "update_cart_item",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/sales/carts/mine/lines/{cart_line_id}",
        summary: "Change the quantity, size or color of a line in the caller's own cart (only to a variant on sale). Runs right after its preview; no approval.",
        consumes: &[Slot::one("cart_line_id")],
        produces: &[Slot::one("cart_line_id")],
        checks: &[
            "cart.max_quantity_per_line",
            "checkout_validation.stock_available",
        ],
        errors: &[
            CART_LINE_NOT_FOUND,
            VARIANT_NOT_FOUND,
            OUT_OF_STOCK,
            PURCHASE_LIMIT_EXCEEDED,
            NOT_SIGNED_IN,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateCartItemInput;
    type Output = Value;
}

fn line_not_found(id: &str) -> ToolError {
    ToolError::new(CART_LINE_NOT_FOUND, format!("Your cart has no line {id}."))
        .detail(json!({"entity": "cart_line", "id": id}))
        .next("get_cart")
}

impl WriteOp<Cx> for UpdateCartItem {
    type Plan = UpdatePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateCartItemInput,
    ) -> Result<Planned<UpdatePlan>, ToolError> {
        if input.order_quantity.is_none() && input.size.is_none() && input.color.is_none() {
            return Err(ToolError::invalid_input(
                "Give a new `order_quantity`, `size` or `color`.",
            )
            .detail(json!({"field": "order_quantity", "reason": "missing"})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let s = start(cx, &mut tx).await?;
        let line = s
            .carts
            .line(&input.cart_line_id)
            .cloned()
            .ok_or_else(|| line_not_found(&input.cart_line_id))?;
        let current = s
            .facts
            .iter()
            .find(|f| f.variant_id == line.variant_id)
            .cloned()
            .ok_or_else(|| line_not_found(&input.cart_line_id))?;
        let variant = if input.size.is_some() || input.color.is_some() {
            let size = input
                .size
                .clone()
                .or(current.size.clone())
                .unwrap_or_default();
            let color = input
                .color
                .clone()
                .or(current.color.clone())
                .unwrap_or_default();
            match catalog::find_variant(&mut tx, &current.product_id, &size, &color).await? {
                VariantMatch::Found(v) => *v,
                VariantMatch::NoSuchOption { sizes, colors } => {
                    return Err(ToolError::new(
                        VARIANT_NOT_FOUND,
                        format!(
                            "{} does not come in {size} / {color}.",
                            current.product_name
                        ),
                    )
                    .detail(
                        json!({"entity": "variant", "product_id": current.product_id,
                                   "sizes": sizes, "colors": colors}),
                    )
                    .next("get_cart"));
                }
                VariantMatch::NoProduct => return Err(line_not_found(&input.cart_line_id)),
            }
        } else {
            current.clone()
        };
        tx.commit().await.map_err(ToolError::from)?;

        let quantity = input.order_quantity.unwrap_or(line.quantity);
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
        let fold_into = s
            .carts
            .merged
            .iter()
            .find(|m| m.variant_id == variant.variant_id && m.id != line.id)
            .map(|m| (m.id.clone(), m.quantity));
        let target = s.carts.target();
        let mut changes = target_changes(&target);
        changes.push(change(
            ChangeOp::Update,
            "cart_line",
            &line.id,
            json!({"variant_id": line.variant_id, "quantity": line.quantity}),
            json!({"variant_id": variant.variant_id, "quantity": quantity}),
        ));
        if let Some((other, _)) = &fold_into {
            changes.push(Change {
                note: Some("The cart already had this variant; the two lines become one".into()),
                ..change(
                    ChangeOp::Delete,
                    "cart_line",
                    other,
                    Value::Null,
                    Value::Null,
                )
            });
        }
        Ok(Planned {
            plan: UpdatePlan {
                target,
                line_id: line.id.clone(),
                from_variant: line.variant_id.clone(),
                from_quantity: line.quantity,
                variant_id: variant.variant_id.clone(),
                quantity,
                fold_into,
            },
            changes,
            checks: vec![stock_check(&variant)],
            basis: s.carts.basis(),
            approval: approval(cx, Self::META.tool, &json!({"quantity": quantity})),
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &UpdatePlan) -> Result<Committed, ToolError> {
        let cart_id = pricing::apply_target(tx, &plan.target).await?;
        if let Some((other, _)) = &plan.fold_into {
            repo::delete_lines(tx, std::slice::from_ref(other)).await?;
        }
        repo::update_line(tx, &plan.line_id, &plan.variant_id, plan.quantity).await?;
        repo::touch_cart(tx, &cart_id).await?;
        Ok(Committed {
            id: plan.line_id.clone(),
            produced: produced(&[("cart_line_id", &plan.line_id), ("cart_id", &cart_id)]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &UpdatePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let now = repo::line_quantity(&mut tx, &plan.line_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let ok = now.as_ref().is_some_and(|(_, v, q)| {
            v == &plan.variant_id && i64::from(*q) == i64::from(plan.quantity)
        });
        Ok(Verification {
            matched: ok,
            mismatches: if ok {
                vec![]
            } else {
                vec![format!("cart line {}: found {now:?}", plan.line_id)]
            },
        })
    }

    async fn undo(&self, tx: &mut TxCx, plan: &UpdatePlan, _: &Committed) -> Result<(), ToolError> {
        repo::update_line(tx, &plan.line_id, &plan.from_variant, plan.from_quantity).await?;
        if let Some((other, quantity)) = &plan.fold_into {
            repo::restore_line(
                tx,
                &LineSnapshot {
                    id: other.clone(),
                    cart_id: plan.target.cart_id.clone(),
                    variant_id: plan.variant_id.clone(),
                    quantity: *quantity,
                    position: 1,
                    note: None,
                },
            )
            .await?;
        }
        repo::touch_cart(tx, &plan.target.cart_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<UpdateCartItemInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
    }
}
