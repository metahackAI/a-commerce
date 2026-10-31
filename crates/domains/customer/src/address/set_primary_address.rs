//! `set_primary_address`.

use super::{address_not_found, basis, person, read_back};
use crate::access::{approval, customer_for};
use crate::repo::{self, AddressRow};
use crate::{ADDRESS_NOT_FOUND, CUSTOMER_NOT_FOUND};
use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPrimaryAddressInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
    /// The address to make primary (`list_addresses`).
    #[schemars(regex(pattern = r"^adr_[a-z0-9_]+$"), example = &"adr_work")]
    pub address_id: String,
}

/// The new primary and the one it replaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetPrimaryPlan {
    pub customer_id: String,
    pub address_id: String,
    /// `None` when there was no primary, or it already was this address.
    pub previous: Option<String>,
    /// `false` when the address already was primary (nothing to do).
    pub changes_anything: bool,
}

pub struct SetPrimaryAddress;

impl Operation for SetPrimaryAddress {
    const META: OpMeta = OpMeta {
        address: "customer.address.set_primary",
        tool: "set_primary_address",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/people/{customer_id}/addresses/{address_id}/primary",
        summary: "Make a saved address the primary one; the old primary stops being primary.",
        consumes: &[Slot::one("address_id")],
        produces: &[Slot::one("address_id")],
        checks: &["address.primary_per_member"],
        errors: &[CUSTOMER_NOT_FOUND, ADDRESS_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetPrimaryAddressInput;
    type Output = Value;
}

impl WriteOp<Cx> for SetPrimaryAddress {
    type Plan = SetPrimaryPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetPrimaryAddressInput,
    ) -> Result<Planned<SetPrimaryPlan>, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        person(&mut tx, &customer_id).await?;
        let target = repo::address(&mut tx, &customer_id, &input.address_id)
            .await?
            .ok_or_else(|| address_not_found(&input.address_id))?;
        let primary = repo::primary_address(&mut tx, &customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut changes = Vec::new();
        let mut basis_rows = vec![basis(&target)];
        let label = |a: &AddressRow| a.label.clone().unwrap_or_else(|| a.street_address.clone());
        let detail = if target.is_primary {
            format!("{} already is the primary address", label(&target))
        } else {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "postal_address".into(),
                id: target.id.clone(),
                from: json!({"is_primary": false}),
                to: json!({"is_primary": true}),
                note: None,
            });
            match &primary {
                Some(old) => {
                    changes.push(Change {
                        op: ChangeOp::Update,
                        entity: "postal_address".into(),
                        id: old.id.clone(),
                        from: json!({"is_primary": true}),
                        to: json!({"is_primary": false}),
                        note: None,
                    });
                    basis_rows.push(basis(old));
                    format!("{} stops being the primary address", label(old))
                }
                None => "No other primary address".to_owned(),
            }
        };
        let previous = primary.filter(|p| p.id != target.id).map(|p| p.id);
        let facts = json!({"customer_id": customer_id});
        Ok(Planned {
            plan: SetPrimaryPlan {
                customer_id,
                address_id: target.id.clone(),
                previous,
                changes_anything: !target.is_primary,
            },
            changes,
            checks: vec![Check {
                rule: "address.primary_per_member".into(),
                passed: true,
                detail: Some(detail),
                fix_tool: None,
            }],
            basis: basis_rows,
            approval: approval(cx, Self::META.tool, &facts),
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SetPrimaryPlan) -> Result<Committed, ToolError> {
        if plan.changes_anything {
            if let Some(old) = &plan.previous {
                repo::set_primary(tx, &plan.customer_id, old, false).await?;
            }
            repo::set_primary(tx, &plan.customer_id, &plan.address_id, true).await?;
        }
        let mut produced = Map::new();
        produced.insert("address_id".into(), json!(plan.address_id));
        Ok(Committed {
            id: plan.address_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SetPrimaryPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mismatches = match read_back(cx, &plan.customer_id, &plan.address_id).await? {
            Some(a) if a.is_primary => vec![],
            Some(_) => vec![format!("{} is not primary", plan.address_id)],
            None => vec![format!("{} is gone", plan.address_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SetPrimaryPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        if plan.changes_anything {
            repo::set_primary(tx, &plan.customer_id, &plan.address_id, false).await?;
            if let Some(old) = &plan.previous {
                repo::set_primary(tx, &plan.customer_id, old, true).await?;
            }
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
        let s = closed_schema_for::<SetPrimaryAddressInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }
}
