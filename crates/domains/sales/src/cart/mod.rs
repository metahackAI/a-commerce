//! The `cart` resource (`docs/api/sales.md` §cart): `get_cart`, `add_to_cart`,
//! `update_cart_item`, `remove_cart_item`, `clear_cart` on the caller's own cart. Writes are
//! `no_approval` (approval.yml): previewed, then run without a person. What the writes share
//! lives here: where a write starts from, the change rows, and removing lines.

pub mod add_to_cart;
pub mod clear_cart;
pub mod get_cart;
pub mod remove_cart_item;
pub mod update_cart_item;

use acom_catalog::service::for_sales::{self as catalog, VariantFacts};
use acom_contract::{Change, ChangeOp, Check, Committed, ToolError, Verification};
use acom_customer::service::for_sales as customer;
use acom_store::{Cx, TxCx};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::owner;
use crate::pricing::{self, CartTarget, Carts};
use crate::repo::{self, LineSnapshot};
use crate::rules::{self, CartLimits};
use crate::{OUT_OF_STOCK, PURCHASE_LIMIT_EXCEEDED};

pub(crate) fn limit(key: &str, max: u32, actual: u32, next: &'static str) -> ToolError {
    ToolError::new(
        PURCHASE_LIMIT_EXCEEDED,
        format!("A cart takes at most {max} here (`{key}`)."),
    )
    .detail(json!({"limit": max, "actual": actual, "policy_key": key}))
    .next(next)
}

pub(crate) fn variant_title(v: &VariantFacts) -> String {
    let mut t = v.product_name.clone();
    for part in [&v.size, &v.color].into_iter().flatten() {
        t.push(' ');
        t.push_str(part);
    }
    t
}

pub(crate) fn out_of_stock(v: &VariantFacts, quantity: u32) -> Option<ToolError> {
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

/// What a cart write starts from: the caller's carts and the catalog facts of their lines.
pub(crate) struct Start {
    pub(crate) limits: CartLimits,
    pub(crate) carts: Carts,
    pub(crate) facts: Vec<VariantFacts>,
    pub(crate) country: Option<String>,
}

pub(crate) async fn start(cx: &Cx, tx: &mut TxCx) -> Result<Start, ToolError> {
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

pub(crate) fn change(op: ChangeOp, entity: &str, id: &str, from: Value, to: Value) -> Change {
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
pub(crate) fn target_changes(t: &CartTarget) -> Vec<Change> {
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

pub(crate) fn stock_check(v: &VariantFacts) -> Check {
    Check {
        rule: "checkout_validation.stock_available".into(),
        passed: true,
        detail: Some(format!("{} is in stock", variant_title(v))),
        fix_tool: None,
    }
}

pub(crate) fn produced(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), json!(v)))
        .collect()
}

/// What removing lines will do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemovePlan {
    /// The cart the result names; empty when the caller has no cart.
    pub(crate) cart_id: String,
    pub(crate) removed: Vec<LineSnapshot>,
}

pub(crate) fn remove_changes(removed: &[LineSnapshot]) -> Vec<Change> {
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

pub(crate) async fn remove_lines(tx: &mut TxCx, plan: &RemovePlan) -> Result<Committed, ToolError> {
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

pub(crate) async fn removed_gone(cx: &Cx, plan: &RemovePlan) -> Result<Verification, ToolError> {
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

pub(crate) async fn put_back(tx: &mut TxCx, plan: &RemovePlan) -> Result<(), ToolError> {
    for l in &plan.removed {
        repo::restore_line(tx, l).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::clear_cart::{ClearCart, ClearCartInput};
    use super::remove_cart_item::{RemoveCartItem, RemoveCartItemInput};
    use super::update_cart_item::{UpdateCartItem, UpdateCartItemInput};
    use crate::rules::tests::defaults;
    use acom_contract::WriteOp;
    use serde_json::json;

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
