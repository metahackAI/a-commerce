//! `remove_cart_item` (`sales.cart.remove`): one line out of the caller's own cart.
//! `no_approval`: runs right after its preview.

use acom_contract::{
    ActionType, Audience, Committed, Group, Kind, Method, OpMeta, Operation, Planned, Slot,
    ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{RemovePlan, put_back, remove_changes, remove_lines, removed_gone};
use crate::owner;
use crate::pricing;
use crate::repo::LineSnapshot;
use crate::rules;
use crate::{NOT_SIGNED_IN, approval};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveCartItemInput {
    /// The line to take out (`get_cart`).
    #[schemars(regex(pattern = r"^cln_[a-z0-9_]+$"), example = &"cln_1")]
    pub cart_line_id: String,
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

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<RemoveCartItemInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
    }
}
