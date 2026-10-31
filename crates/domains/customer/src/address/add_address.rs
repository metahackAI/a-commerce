//! `add_address`.

use super::{basis, one_line, person, person_basis, postal_code_check, read_back, usage_words};
use crate::access::{approval, customer_for, policy_number};
use crate::repo::{self, AddressFields};
use crate::{ADDRESS_LIMIT_EXCEEDED, CUSTOMER_NOT_FOUND, INVALID_POSTAL_CODE};
use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::AddressUsage;
use acom_core::ids::AddressId;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddAddressInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
    /// The shopper's name for the address.
    #[schemars(length(max = 40), example = &"Work")]
    pub label: Option<String>,
    /// Recipient name printed on parcels.
    #[schemars(length(max = 100), example = &"Jamie Rivera")]
    pub name: Option<String>,
    /// Street, number and unit.
    #[schemars(length(min = 1, max = 200), example = &"200 Water St")]
    pub street_address: String,
    /// City or town.
    #[schemars(length(min = 1, max = 100), example = &"Brooklyn")]
    pub address_locality: String,
    /// State or region.
    #[schemars(length(max = 100), example = &"NY")]
    pub address_region: Option<String>,
    /// Postal code, checked against the country's format.
    #[schemars(length(min = 1, max = 12), example = &"11201")]
    pub postal_code: String,
    /// Country, ISO 3166-1 alpha-2.
    #[schemars(regex(pattern = r"^[A-Z]{2}$"), example = &"US")]
    pub address_country: String,
    /// Contact number for the carrier.
    #[schemars(length(max = 30), example = &"+1 718 555 0142")]
    pub telephone: Option<String>,
    /// What the address is for.
    #[schemars(length(min = 1), example = ["shipping"])]
    pub usage: Vec<AddressUsage>,
    /// Make it the primary address; the old primary stops being one. The first address is always
    /// primary.
    #[schemars(example = false)]
    pub is_primary: Option<bool>,
}

/// Everything `commit` needs to add the address.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddAddressPlan {
    pub customer_id: String,
    pub address_id: String,
    pub address: AddressFields,
    pub is_primary: bool,
    /// The old primary address that stops being primary.
    pub unset_primary: Option<String>,
}

pub struct AddAddress;

impl Operation for AddAddress {
    const META: OpMeta = OpMeta {
        address: "customer.address.add",
        tool: "add_address",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/customer/people/{customer_id}/addresses",
        summary: "Save a new address for a member. With is_primary the old primary address stops being primary.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::one("address_id")],
        checks: &["address.postal_code_validation", "address.max_per_member"],
        errors: &[
            CUSTOMER_NOT_FOUND,
            ADDRESS_LIMIT_EXCEEDED,
            INVALID_POSTAL_CODE,
        ],
        audience: Audience::Both,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = AddAddressInput;
    type Output = Value;
}

impl WriteOp<Cx> for AddAddress {
    type Plan = AddAddressPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: AddAddressInput,
    ) -> Result<Planned<AddAddressPlan>, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let limit = policy_number(cx, "customer.address.max_per_member")?;
        let address = AddressFields {
            label: input.label,
            name: input.name,
            street_address: input.street_address.trim().to_owned(),
            address_locality: input.address_locality.trim().to_owned(),
            address_region: input.address_region,
            postal_code: input.postal_code.trim().to_owned(),
            address_country: input.address_country,
            telephone: input.telephone,
            usage: usage_words(&input.usage),
        };
        let postal = postal_code_check(
            &address.address_country,
            &address.postal_code,
            "add_address",
        )?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let who = person(&mut tx, &customer_id).await?;
        let (count, _) = repo::addresses(&mut tx, &customer_id, 1, 0).await?;
        let primary = repo::primary_address(&mut tx, &customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if count >= limit {
            return Err(ToolError::new(
                ADDRESS_LIMIT_EXCEEDED,
                format!("A member can save {limit} addresses; remove one first."),
            )
            .detail(json!({"limit": limit, "actual": count,
                           "policy_key": "address.max_per_member"}))
            .next("list_addresses"));
        }
        let is_primary = input.is_primary.unwrap_or(false) || primary.is_none();
        let address_id = AddressId::generate().to_string();
        let mut changes = vec![Change {
            op: ChangeOp::Create,
            entity: "postal_address".into(),
            id: String::new(),
            from: Value::Null,
            to: json!(one_line(&address)),
            note: is_primary.then(|| "Becomes the primary address".to_owned()),
        }];
        let mut basis_rows = vec![person_basis(&who)];
        let unset_primary = match (&primary, is_primary) {
            (Some(old), true) => {
                changes.push(Change {
                    op: ChangeOp::Update,
                    entity: "postal_address".into(),
                    id: old.id.clone(),
                    from: json!({"is_primary": true}),
                    to: json!({"is_primary": false}),
                    note: None,
                });
                basis_rows.push(basis(old));
                Some(old.id.clone())
            }
            _ => None,
        };
        let checks = vec![
            postal,
            Check {
                rule: "address.max_per_member".into(),
                passed: true,
                detail: Some(format!("{} of {limit} addresses", count + 1)),
                fix_tool: None,
            },
        ];
        let facts = json!({"customer_id": customer_id});
        Ok(Planned {
            plan: AddAddressPlan {
                customer_id,
                address_id,
                address,
                is_primary,
                unset_primary,
            },
            changes,
            checks,
            basis: basis_rows,
            approval: approval(cx, Self::META.tool, &facts),
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &AddAddressPlan) -> Result<Committed, ToolError> {
        if let Some(old) = &plan.unset_primary {
            repo::set_primary(tx, &plan.customer_id, old, false).await?;
        }
        repo::insert_address(
            tx,
            &plan.address_id,
            &plan.customer_id,
            &plan.address,
            plan.is_primary,
        )
        .await?;
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
        plan: &AddAddressPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut mismatches = Vec::new();
        match read_back(cx, &plan.customer_id, &plan.address_id).await? {
            None => mismatches.push(format!("{} was not saved", plan.address_id)),
            Some(a) => {
                if AddressFields::from(&a) != plan.address {
                    mismatches.push(format!("{} differs from the plan", plan.address_id));
                }
                if a.is_primary != plan.is_primary {
                    mismatches.push(format!("{} primary is {}", a.id, a.is_primary));
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
        plan: &AddAddressPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        repo::delete_address(tx, &plan.customer_id, &plan.address_id).await?;
        if let Some(old) = &plan.unset_primary {
            repo::set_primary(tx, &plan.customer_id, old, true).await?;
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
        let s = closed_schema_for::<AddAddressInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }
}
