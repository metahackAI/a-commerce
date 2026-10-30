//! `get_cart`, `add_to_cart`, `update_cart_item`, `remove_cart_item`, `clear_cart`: the caller's
//! own cart. Writes are `no_approval` (approval.yml): previewed, then run without a person.

use acom_catalog::service::for_sales::{self as catalog, VariantFacts, VariantMatch};
use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, ReadOp, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::ids::CartLineId;
use acom_customer::service::for_sales as customer;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::owner;
use crate::pricing::{self, CartTarget, Carts, DeliveryChoice};
use crate::repo::{self, LineSnapshot};
use crate::rules::{self, CartLimits};
use crate::{
    CART_LINE_NOT_FOUND, MIXED_ORDER_TYPE, NOT_SIGNED_IN, OUT_OF_STOCK, PRODUCT_NOT_FOUND,
    PURCHASE_LIMIT_EXCEEDED, VARIANT_NOT_FOUND, approval,
};

/// Top-level fields of a cart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CartField {
    /// The cart id.
    Id,
    /// The lines.
    Lines,
    /// Sum of list prices.
    Subtotal,
    /// One discount line per promotion.
    Discounts,
    /// Shipping charged.
    Shipping,
    /// What the shopper pays.
    Total,
    /// Where it goes and when it arrives.
    Delivery,
    /// How it is paid.
    Payment,
    /// Coupons the order will record.
    CouponsRecorded,
}

impl CartField {
    fn word(self) -> &'static str {
        match self {
            CartField::Id => "id",
            CartField::Lines => "lines",
            CartField::Subtotal => "subtotal",
            CartField::Discounts => "discounts",
            CartField::Shipping => "shipping",
            CartField::Total => "total",
            CartField::Delivery => "delivery",
            CartField::Payment => "payment",
            CartField::CouponsRecorded => "coupons_recorded",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCartInput {
    /// Only these top-level fields. All when left out.
    #[schemars(example = ["lines", "total"])]
    pub fields: Option<Vec<CartField>>,
}

pub(crate) struct GetCart;

impl Operation for GetCart {
    const META: OpMeta = OpMeta {
        address: "sales.cart.get",
        tool: "get_cart",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/carts/mine",
        summary: "The caller's own cart: lines, subtotal, one discount line per promotion, shipping, total, delivery estimate, payment method and the coupons the order will record, all from calculate_benefits. An empty cart when there is none.",
        consumes: &[],
        produces: &[Slot::one("cart_id"), Slot::many("cart_line_id")],
        checks: &[
            "discount_layers",
            "member_benefits_per_order",
            "free_shipping_min_subtotal",
        ],
        errors: &[],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCartInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCart {
    async fn read(&self, cx: &Cx, input: GetCartInput) -> Result<Value, ToolError> {
        let owners = owner::owners(&cx.principal)?;
        let limits = rules::cart_limits(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let carts = pricing::load_carts(&mut tx, &owners, limits).await?;
        let view = pricing::view(&mut tx, &carts, &DeliveryChoice::CartDefault, None).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let view = view.map_err(|id| crate::address_not_found(&id))?;
        let fields: Vec<String> = input
            .fields
            .unwrap_or_default()
            .iter()
            .map(|f| f.word().to_owned())
            .collect();
        Ok(pricing::pick(pricing::cart_json(&view), &fields))
    }
}

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

fn limit(key: &str, max: u32, actual: u32, next: &'static str) -> ToolError {
    ToolError::new(
        PURCHASE_LIMIT_EXCEEDED,
        format!("A cart takes at most {max} here (`{key}`)."),
    )
    .detail(json!({"limit": max, "actual": actual, "policy_key": key}))
    .next(next)
}

fn variant_title(v: &VariantFacts) -> String {
    let mut t = v.product_name.clone();
    for part in [&v.size, &v.color].into_iter().flatten() {
        t.push(' ');
        t.push_str(part);
    }
    t
}

fn out_of_stock(v: &VariantFacts, quantity: u32) -> Option<ToolError> {
    let reason = if !v.sellable() {
        "not_sellable"
    } else if !v.has_stock(i64::from(quantity)) {
        "not_enough"
    } else {
        return None;
    };
    Some(
        ToolError::new(
            OUT_OF_STOCK,
            format!("{} can't be bought in that quantity now.", variant_title(v)),
        )
        .detail(json!({"variant_id": v.variant_id, "reason": reason,
                       "inventory_level": v.inventory_level, "availability": v.availability}))
        .next("get_product"),
    )
}

/// Subscriptions and gift cards are their own kind of order (`cart.separate_by`).
fn order_kind(v: &VariantFacts) -> &str {
    match v.sale_type.as_str() {
        "subscription" => "subscription",
        _ => "one_time",
    }
}

/// What a cart write starts from: the caller's carts and the catalog facts of their lines.
struct Start {
    limits: CartLimits,
    carts: Carts,
    facts: Vec<VariantFacts>,
    country: Option<String>,
}

async fn start(cx: &Cx, tx: &mut TxCx) -> Result<Start, ToolError> {
    let owners = owner::owners(&cx.principal)?;
    let limits = rules::cart_limits(&cx.policy)?;
    let carts = pricing::load_carts(tx, &owners, limits).await?;
    let ids: Vec<String> = carts.merged.iter().map(|m| m.variant_id.clone()).collect();
    let facts = catalog::variants(tx, &ids).await?;
    let country = match &owners.owner.customer_id {
        Some(c) => customer::default_shipping_address(tx, c)
            .await?
            .map(|a| a.address_country),
        None => None,
    }
    .or_else(|| rules::ships_to(&cx.policy).into_iter().next());
    Ok(Start {
        limits,
        carts,
        facts,
        country,
    })
}

fn change(op: ChangeOp, entity: &str, id: &str, from: Value, to: Value) -> Change {
    Change {
        op,
        entity: entity.into(),
        id: id.into(),
        from,
        to,
        note: None,
    }
}

/// The change rows a cart write adds before its own: creating or taking over a cart.
fn target_changes(t: &CartTarget) -> Vec<Change> {
    let mut out = Vec::new();
    if t.create {
        out.push(change(
            ChangeOp::Create,
            "cart",
            "",
            Value::Null,
            json!({"customer_id": t.owner.customer_id, "session_id": t.owner.session_id}),
        ));
    }
    if t.adopt {
        out.push(Change {
            note: Some("The cart from before signing in becomes the shopper's cart".into()),
            ..change(
                ChangeOp::Update,
                "cart",
                &t.cart_id,
                json!({"customer_id": null}),
                json!({"customer_id": t.owner.customer_id}),
            )
        });
    }
    if let Some(a) = &t.absorb {
        out.push(Change {
            note: Some(format!(
                "Takes over {} line(s) added before signing in",
                a.moves.len() + a.folds.len()
            )),
            ..change(
                ChangeOp::Update,
                "cart",
                &t.cart_id,
                json!({"cart_id": a.guest_cart_id}),
                json!({"cart_id": t.cart_id}),
            )
        });
    }
    out
}

fn stock_check(v: &VariantFacts) -> Check {
    Check {
        rule: "checkout_validation.stock_available".into(),
        passed: true,
        detail: Some(format!("{} is in stock", variant_title(v))),
        fix_tool: None,
    }
}

fn produced(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), json!(v)))
        .collect()
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveCartItemInput {
    /// The line to take out (`get_cart`).
    #[schemars(regex(pattern = r"^cln_[a-z0-9_]+$"), example = &"cln_1")]
    pub cart_line_id: String,
}

/// What removing lines will do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemovePlan {
    /// The cart the result names; empty when the caller has no cart.
    cart_id: String,
    removed: Vec<LineSnapshot>,
}

fn remove_changes(removed: &[LineSnapshot]) -> Vec<Change> {
    removed
        .iter()
        .map(|l| {
            change(
                ChangeOp::Delete,
                "cart_line",
                &l.id,
                json!({"variant_id": l.variant_id, "quantity": l.quantity}),
                Value::Null,
            )
        })
        .collect()
}

async fn remove_lines(tx: &mut TxCx, plan: &RemovePlan) -> Result<Committed, ToolError> {
    let ids: Vec<String> = plan.removed.iter().map(|l| l.id.clone()).collect();
    repo::delete_lines(tx, &ids).await?;
    let mut carts: Vec<&str> = plan.removed.iter().map(|l| l.cart_id.as_str()).collect();
    carts.sort_unstable();
    carts.dedup();
    for c in carts {
        repo::touch_cart(tx, c).await?;
    }
    Ok(Committed {
        id: plan.cart_id.clone(),
        produced: produced(&[("cart_id", &plan.cart_id)]),
    })
}

async fn removed_gone(cx: &Cx, plan: &RemovePlan) -> Result<Verification, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let mut mismatches = Vec::new();
    for l in &plan.removed {
        if repo::line_quantity(&mut tx, &l.id).await?.is_some() {
            mismatches.push(format!("cart line {} is still there", l.id));
        }
    }
    tx.commit().await.map_err(ToolError::from)?;
    Ok(Verification {
        matched: mismatches.is_empty(),
        mismatches,
    })
}

async fn put_back(tx: &mut TxCx, plan: &RemovePlan) -> Result<(), ToolError> {
    for l in &plan.removed {
        repo::restore_line(tx, l).await?;
    }
    Ok(())
}

pub(crate) struct RemoveCartItem;

impl Operation for RemoveCartItem {
    const META: OpMeta = OpMeta {
        address: "sales.cart.remove",
        tool: "remove_cart_item",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Delete,
        path: "/v1/sales/carts/mine/lines/{cart_line_id}",
        summary: "Take one line out of the caller's own cart. A line that is already gone counts as removed. Runs right after its preview; no approval.",
        consumes: &[Slot::one("cart_line_id")],
        produces: &[Slot::one("cart_id")],
        checks: &[],
        errors: &[NOT_SIGNED_IN],
        audience: Audience::Shopper,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = RemoveCartItemInput;
    type Output = Value;
}

impl WriteOp<Cx> for RemoveCartItem {
    type Plan = RemovePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RemoveCartItemInput,
    ) -> Result<Planned<RemovePlan>, ToolError> {
        let owners = owner::owners(&cx.principal)?;
        let limits = rules::cart_limits(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let carts = pricing::load_carts(&mut tx, &owners, limits).await?;
        tx.commit().await.map_err(ToolError::from)?;
        // The merged line, and a guest line folded into it, go together.
        let removed: Vec<LineSnapshot> = match carts.line(&input.cart_line_id) {
            Some(m) => [Some(m.id.as_str()), m.absorbs.as_deref()]
                .into_iter()
                .flatten()
                .filter_map(|id| carts.row(id))
                .map(LineSnapshot::from)
                .collect(),
            None => Vec::new(),
        };
        let plan = RemovePlan {
            cart_id: carts.cart().map(|c| c.id.clone()).unwrap_or_default(),
            removed,
        };
        Ok(Planned {
            changes: remove_changes(&plan.removed),
            checks: vec![],
            basis: carts.basis(),
            approval: approval(cx, Self::META.tool, &json!({})),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &RemovePlan) -> Result<Committed, ToolError> {
        remove_lines(tx, plan).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RemovePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        removed_gone(cx, plan).await
    }

    async fn undo(&self, tx: &mut TxCx, plan: &RemovePlan, _: &Committed) -> Result<(), ToolError> {
        put_back(tx, plan).await
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClearCartInput {}

pub(crate) struct ClearCart;

impl Operation for ClearCart {
    const META: OpMeta = OpMeta {
        address: "sales.cart.clear",
        tool: "clear_cart",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Delete,
        path: "/v1/sales/carts/mine/lines",
        summary: "Empty the caller's own cart. Runs right after its preview; no approval.",
        consumes: &[],
        produces: &[Slot::one("cart_id")],
        checks: &[],
        errors: &[NOT_SIGNED_IN],
        audience: Audience::Shopper,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = ClearCartInput;
    type Output = Value;
}

impl WriteOp<Cx> for ClearCart {
    type Plan = RemovePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(&self, cx: &Cx, _: ClearCartInput) -> Result<Planned<RemovePlan>, ToolError> {
        let owners = owner::owners(&cx.principal)?;
        let limits = rules::cart_limits(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let carts = pricing::load_carts(&mut tx, &owners, limits).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let plan = RemovePlan {
            cart_id: carts.cart().map(|c| c.id.clone()).unwrap_or_default(),
            removed: carts.rows.iter().map(LineSnapshot::from).collect(),
        };
        Ok(Planned {
            changes: remove_changes(&plan.removed),
            checks: vec![],
            basis: carts.basis(),
            approval: approval(cx, Self::META.tool, &json!({})),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &RemovePlan) -> Result<Committed, ToolError> {
        remove_lines(tx, plan).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RemovePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        removed_gone(cx, plan).await
    }

    async fn undo(&self, tx: &mut TxCx, plan: &RemovePlan, _: &Committed) -> Result<(), ToolError> {
        put_back(tx, plan).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::defaults;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schemas_are_closed_with_modes_as_words() {
        let add = closed_schema_for::<AddToCartInput>();
        assert!(violations(&add).is_empty(), "{add:#}");
        assert_eq!(
            add["properties"]["mode"]["enum"],
            json!(["product_options", "variant"])
        );
        assert_eq!(add["required"], json!(["mode"]));
        for s in [
            closed_schema_for::<GetCartInput>(),
            closed_schema_for::<UpdateCartItemInput>(),
            closed_schema_for::<RemoveCartItemInput>(),
            closed_schema_for::<ClearCartInput>(),
        ] {
            assert!(violations(&s).is_empty(), "{s:#}");
        }
    }

    #[test]
    fn cart_writes_need_no_approval() {
        let p = defaults();
        for tool in [
            "add_to_cart",
            "update_cart_item",
            "remove_cart_item",
            "clear_cart",
        ] {
            assert!(
                !p.approval_for(tool, &json!({"amount": 1e6})).needed,
                "{tool}"
            );
        }
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

    /// Integration: update, remove and clear previews on Jamie's cart (`DATABASE_URL`).
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn previews_line_changes_without_changing_anything() {
        let cx = crate::testdb::cx("cus_jamie").await;
        let e = UpdateCartItem
            .preview(
                &cx,
                UpdateCartItemInput {
                    cart_line_id: "cln_missing".into(),
                    order_quantity: Some(2),
                    size: None,
                    color: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "cart_line_not_found");
        let gone = RemoveCartItem
            .preview(
                &cx,
                RemoveCartItemInput {
                    cart_line_id: "cln_missing".into(),
                },
            )
            .await
            .unwrap();
        assert!(gone.changes.is_empty());
        ClearCart.preview(&cx, ClearCartInput {}).await.unwrap();
    }
}
