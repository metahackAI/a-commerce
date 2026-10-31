//! `clear_cart` (`sales.cart.clear`): empties the caller's own cart. `no_approval`: runs right
//! after its preview.

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
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<ClearCartInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
    }
}
