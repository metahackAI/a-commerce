//! `add_to_wishlist`.

use super::{WishlistItemInput, WishlistPlan, person_basis, produced};
use crate::access::{approval, customer_for};
use crate::service::providers;
use crate::{CUSTOMER_NOT_FOUND, PRODUCT_NOT_FOUND, repo};
use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, Group, Kind, Method, OpMeta, Operation,
    Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use serde_json::{Value, json};

pub struct AddToWishlist;

impl Operation for AddToWishlist {
    const META: OpMeta = OpMeta {
        address: "customer.wishlist.add",
        tool: "add_to_wishlist",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/people/{customer_id}/wishlist",
        summary: "Keep a product on the shopper's wishlist. Already there is fine.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("product_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND, PRODUCT_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::ADD,
        person_scope: "",
    };
    type Input = WishlistItemInput;
    type Output = Value;
}

impl WriteOp<Cx> for AddToWishlist {
    type Plan = WishlistPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: WishlistItemInput,
    ) -> Result<Planned<WishlistPlan>, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let basis = person_basis(&mut tx, &customer_id).await?;
        // Without the catalog reader the foreign key still refuses unknown products at commit.
        if let Some(read) = providers::product_cards() {
            let found = read(&mut tx, std::slice::from_ref(&input.product_id)).await?;
            if found.is_empty() {
                return Err(ToolError::new(
                    PRODUCT_NOT_FOUND,
                    format!("There is no product {}.", input.product_id),
                )
                .detail(json!({"entity": "product", "id": input.product_id}))
                .next("search_products"));
            }
        }
        let existing = repo::wishlist_item(&mut tx, &customer_id, &input.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let changes = match &existing {
            Some(_) => vec![],
            None => vec![Change {
                op: ChangeOp::Create,
                entity: "wishlist_item".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"product_id": input.product_id}),
                note: None,
            }],
        };
        let facts = json!({"customer_id": customer_id});
        Ok(Planned {
            plan: WishlistPlan {
                customer_id,
                product_id: input.product_id,
                added_at: existing
                    .as_ref()
                    .map_or_else(|| cx.clock.now(), |e| e.created_at),
                changes_anything: existing.is_none(),
            },
            changes,
            checks: vec![],
            basis: vec![basis],
            approval: approval(cx, Self::META.tool, &facts),
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &WishlistPlan) -> Result<Committed, ToolError> {
        if plan.changes_anything {
            repo::insert_wishlist_item(tx, &plan.customer_id, &plan.product_id, plan.added_at)
                .await?;
        }
        Ok(produced(&plan.product_id))
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &WishlistPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::wishlist_item(&mut tx, &plan.customer_id, &plan.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            Some(_) => vec![],
            None => vec![format!("{} is not on the wishlist", plan.product_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &WishlistPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        if plan.changes_anything {
            repo::delete_wishlist_item(tx, &plan.customer_id, &plan.product_id).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<WishlistItemInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(AddToWishlist::META.kind, Kind::Write);
    }
}
