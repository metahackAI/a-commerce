//! `list_addresses`.

use super::person;
use crate::CUSTOMER_NOT_FOUND;
use crate::access::{Caller, caller, customer_for};
use crate::repo::{self};
use crate::wording::{mask_postal_code, mask_street};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListAddressesInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListAddresses;

impl Operation for ListAddresses {
    const META: OpMeta = OpMeta {
        address: "customer.address.list",
        tool: "list_addresses",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/{customer_id}/addresses",
        summary: "A member's saved addresses with their use (shipping, billing, return) and which one is primary. Masked for operators.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::many("address_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListAddressesInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListAddresses {
    async fn read(&self, cx: &Cx, input: ListAddressesInput) -> Result<Value, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        // The shopper sees their own addresses in full; operators see them masked.
        let masked = !matches!(caller(&cx.principal), Caller::Shopper(_));
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        person(&mut tx, &customer_id).await?;
        let (total, rows) = repo::addresses(&mut tx, &customer_id, page.limit, page.offset).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|a| {
                let (street, postal) = if masked {
                    (
                        mask_street(&a.street_address),
                        mask_postal_code(&a.postal_code),
                    )
                } else {
                    (a.street_address.clone(), a.postal_code.clone())
                };
                json!({"id": a.id, "label": a.label, "street_address": street,
                       "address_locality": a.address_locality, "address_region": a.address_region,
                       "postal_code": postal, "address_country": a.address_country,
                       "usage": a.address_usage, "is_primary": a.is_primary})
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<ListAddressesInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }
}
