//! `remove_from_wishlist`.

use super::{WishlistItemInput, WishlistPlan, person_basis, produced};
use crate::access::{approval, customer_for};
use crate::{CUSTOMER_NOT_FOUND, repo};
use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, Group, Kind, Method, OpMeta, Operation,
    Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use serde_json::{Value, json};

pub struct RemoveFromWishlist;

impl Operation for RemoveFromWishlist {
    const META: OpMeta = OpMeta {
        address: "customer.wishlist.remove",
        tool: "remove_from_wishlist",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Delete,
        path: "/v1/customer/people/{customer_id}/wishlist/{product_id}",
        summary: "Take a product off the shopper's wishlist. Not there is fine.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("product_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = WishlistItemInput;
    type Output = Value;
}

impl WriteOp<Cx> for RemoveFromWishlist {
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
        let existing = repo::wishlist_item(&mut tx, &customer_id, &input.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let changes = match &existing {
            Some(_) => vec![Change {
                op: ChangeOp::Delete,
                entity: "wishlist_item".into(),
                id: input.product_id.clone(),
                from: json!({"product_id": input.product_id}),
                to: Value::Null,
                note: None,
            }],
            None => vec![],
        };
        let facts = json!({"customer_id": customer_id});
        Ok(Planned {
            plan: WishlistPlan {
                customer_id,
                product_id: input.product_id,
                added_at: existing
                    .as_ref()
                    .map_or_else(|| cx.clock.now(), |e| e.created_at),
                changes_anything: existing.is_some(),
            },
            changes,
            checks: vec![],
            basis: vec![basis],
            approval: approval(cx, Self::META.tool, &facts),
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &WishlistPlan) -> Result<Committed, ToolError> {
        if plan.changes_anything {
            repo::delete_wishlist_item(tx, &plan.customer_id, &plan.product_id).await?;
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
            Some(_) => vec![format!("{} is still on the wishlist", plan.product_id)],
            None => vec![],
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
            repo::insert_wishlist_item(tx, &plan.customer_id, &plan.product_id, plan.added_at)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_names_the_product() {
        assert_eq!(
            RemoveFromWishlist::META.path,
            "/v1/customer/people/{customer_id}/wishlist/{product_id}"
        );
    }
}
