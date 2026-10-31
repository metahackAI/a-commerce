//! `update_address`.

use super::{address_not_found, basis, person, postal_code_check, read_back, usage_words};
use crate::access::{approval, customer_for};
use crate::repo::{self, AddressFields};
use crate::{ADDRESS_NOT_FOUND, CUSTOMER_NOT_FOUND, INVALID_POSTAL_CODE};
use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, Group, Kind, Method, OpMeta, Operation,
    Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::AddressUsage;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAddressInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
    /// The address to change (`list_addresses`).
    #[schemars(regex(pattern = r"^adr_[a-z0-9_]+$"), example = &"adr_home")]
    pub address_id: String,
    /// New name for the address.
    #[schemars(length(max = 40), example = &"Home")]
    pub label: Option<String>,
    /// New recipient name.
    #[schemars(length(max = 100), example = &"Jamie Rivera")]
    pub name: Option<String>,
    /// New street, number and unit.
    #[schemars(length(min = 1, max = 200), example = &"12 Harbor St, Apt 4")]
    pub street_address: Option<String>,
    /// New city or town.
    #[schemars(length(min = 1, max = 100), example = &"Brooklyn")]
    pub address_locality: Option<String>,
    /// New state or region.
    #[schemars(length(max = 100), example = &"NY")]
    pub address_region: Option<String>,
    /// New postal code.
    #[schemars(length(min = 1, max = 12), example = &"11201")]
    pub postal_code: Option<String>,
    /// New country, ISO 3166-1 alpha-2.
    #[schemars(regex(pattern = r"^[A-Z]{2}$"), example = &"US")]
    pub address_country: Option<String>,
    /// New contact number.
    #[schemars(length(max = 30), example = &"+1 718 555 0142")]
    pub telephone: Option<String>,
    /// New uses of the address.
    #[schemars(length(min = 1), example = ["shipping", "billing"])]
    pub usage: Option<Vec<AddressUsage>>,
}

/// The address before and after.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateAddressPlan {
    pub customer_id: String,
    pub address_id: String,
    pub before: AddressFields,
    pub after: AddressFields,
}

pub struct UpdateAddress;

impl Operation for UpdateAddress {
    const META: OpMeta = OpMeta {
        address: "customer.address.update",
        tool: "update_address",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/customer/people/{customer_id}/addresses/{address_id}",
        summary: "Change fields of a saved address (street, unit, city, postal code, label, uses).",
        consumes: &[Slot::one("address_id")],
        produces: &[Slot::one("address_id")],
        checks: &["address.postal_code_validation"],
        errors: &[CUSTOMER_NOT_FOUND, ADDRESS_NOT_FOUND, INVALID_POSTAL_CODE],
        audience: Audience::Both,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateAddressInput;
    type Output = Value;
}

/// The fields that differ, as `{field: value}` from each side.
fn changed_fields(before: &AddressFields, after: &AddressFields) -> (Value, Value) {
    let (b, a) = (json!(before), json!(after));
    let mut from = Map::new();
    let mut to = Map::new();
    if let (Value::Object(b), Value::Object(a)) = (b, a) {
        for (k, v) in a {
            let old = b.get(&k).cloned().unwrap_or(Value::Null);
            if old != v {
                from.insert(k.clone(), old);
                to.insert(k, v);
            }
        }
    }
    (Value::Object(from), Value::Object(to))
}

impl WriteOp<Cx> for UpdateAddress {
    type Plan = UpdateAddressPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateAddressInput,
    ) -> Result<Planned<UpdateAddressPlan>, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        person(&mut tx, &customer_id).await?;
        let row = repo::address(&mut tx, &customer_id, &input.address_id)
            .await?
            .ok_or_else(|| address_not_found(&input.address_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        let before = AddressFields::from(&row);
        let after = AddressFields {
            label: input.label.or_else(|| before.label.clone()),
            name: input.name.or_else(|| before.name.clone()),
            street_address: input
                .street_address
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|| before.street_address.clone()),
            address_locality: input
                .address_locality
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|| before.address_locality.clone()),
            address_region: input
                .address_region
                .or_else(|| before.address_region.clone()),
            postal_code: input
                .postal_code
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|| before.postal_code.clone()),
            address_country: input
                .address_country
                .unwrap_or_else(|| before.address_country.clone()),
            telephone: input.telephone.or_else(|| before.telephone.clone()),
            usage: input
                .usage
                .map(|u| usage_words(&u))
                .unwrap_or_else(|| before.usage.clone()),
        };
        let (from, to) = changed_fields(&before, &after);
        if to.as_object().is_none_or(Map::is_empty) {
            return Err(ToolError::invalid_input(
                "Nothing to change: every given field already has that value.",
            )
            .detail(json!({"field": "address_id", "reason": "unchanged"}))
            .next("list_addresses"));
        }
        let postal_unchanged = before.postal_code == after.postal_code
            && before.address_country == after.address_country;
        let mut postal =
            postal_code_check(&after.address_country, &after.postal_code, "update_address")?;
        if postal_unchanged {
            postal.detail = Some("Postal code unchanged".into());
        }
        let facts = json!({"customer_id": customer_id});
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "postal_address".into(),
                id: row.id.clone(),
                from,
                to,
                note: None,
            }],
            checks: vec![postal],
            basis: vec![basis(&row)],
            approval: approval(cx, Self::META.tool, &facts),
            plan: UpdateAddressPlan {
                customer_id,
                address_id: row.id,
                before,
                after,
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &UpdateAddressPlan,
    ) -> Result<Committed, ToolError> {
        if repo::address(tx, &plan.customer_id, &plan.address_id)
            .await?
            .is_none()
        {
            return Err(address_not_found(&plan.address_id));
        }
        repo::write_address(tx, &plan.customer_id, &plan.address_id, &plan.after).await?;
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
        plan: &UpdateAddressPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let now = read_back(cx, &plan.customer_id, &plan.address_id).await?;
        let mismatches = match now {
            Some(a) if AddressFields::from(&a) == plan.after => vec![],
            Some(_) => vec![format!("{} differs from the plan", plan.address_id)],
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
        plan: &UpdateAddressPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        repo::write_address(tx, &plan.customer_id, &plan.address_id, &plan.before).await
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<UpdateAddressInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }

    #[test]
    fn shows_only_changed_fields() {
        let before = super::super::tests::home();
        let after = AddressFields {
            street_address: "12 Harbor St, Apt 4".into(),
            ..super::super::tests::home()
        };
        let (from, to) = changed_fields(&before, &after);
        assert_eq!(from, json!({"street_address": "12 Harbor St, Apt 3"}));
        assert_eq!(to, json!({"street_address": "12 Harbor St, Apt 4"}));
        assert_eq!(
            super::super::one_line(&before),
            "12 Harbor St, Apt 3, Brooklyn, NY 11201"
        );
    }
}
