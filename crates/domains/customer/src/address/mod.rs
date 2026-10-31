//! `list_addresses`, `add_address`, `update_address`, `set_primary_address`: a member's saved
//! addresses (`PostalAddress`). One address is primary (`address.primary_per_member`).

use acom_contract::{Basis, Check, ToolError};
use acom_core::enums::AddressUsage;
use acom_store::{Cx, TxCx};
use serde_json::json;

use crate::person::customer_not_found;
use crate::repo::{self, AddressFields, AddressRow};
use crate::wording::postal_code_ok;
use crate::{ADDRESS_NOT_FOUND, INVALID_POSTAL_CODE};

mod add_address;
mod list_addresses;
mod set_primary_address;
mod update_address;

pub use add_address::*;
pub use list_addresses::*;
pub use set_primary_address::*;
pub use update_address::*;

fn address_not_found(id: &str) -> ToolError {
    ToolError::new(ADDRESS_NOT_FOUND, format!("There is no address {id}."))
        .detail(json!({"entity": "postal_address", "id": id}))
        .next("list_addresses")
}

/// `200 Water St, Brooklyn, NY 11201`.
fn one_line(a: &AddressFields) -> String {
    let region = a
        .address_region
        .as_deref()
        .map(|r| format!("{r} "))
        .unwrap_or_default();
    format!(
        "{}, {}, {region}{}",
        a.street_address, a.address_locality, a.postal_code
    )
}

fn basis(a: &AddressRow) -> Basis {
    Basis {
        entity: "customer.postal_address".into(),
        id: a.id.clone(),
        version: i64::from(a.version),
    }
}

fn person_basis(p: &repo::PersonRow) -> Basis {
    Basis {
        entity: "customer.person".into(),
        id: p.id.clone(),
        version: i64::from(p.version),
    }
}

/// The `address.postal_code_validation` check, or `invalid_postal_code`.
fn postal_code_check(country: &str, code: &str, tool: &'static str) -> Result<Check, ToolError> {
    if !postal_code_ok(country, code) {
        return Err(ToolError::new(
            INVALID_POSTAL_CODE,
            format!("{code} is not a postal code used in {country}."),
        )
        .detail(json!({"field": "postal_code", "reason": "format", "country": country}))
        .next(tool));
    }
    Ok(Check {
        rule: "address.postal_code_validation".into(),
        passed: true,
        detail: Some(format!("{code} has the {country} postal code format")),
        fix_tool: None,
    })
}

fn usage_words(usage: &[AddressUsage]) -> Vec<String> {
    let mut words: Vec<String> = usage.iter().map(|u| u.as_str().to_owned()).collect();
    words.dedup();
    words
}

/// Reads the person and refuses unknown customers.
async fn person(tx: &mut TxCx, customer_id: &str) -> Result<repo::PersonRow, ToolError> {
    repo::person(tx, customer_id)
        .await?
        .ok_or_else(|| customer_not_found(customer_id))
}

/// What a write did to one address, read back for `verify`.
async fn read_back(cx: &Cx, customer_id: &str, id: &str) -> Result<Option<AddressRow>, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let a = repo::address(&mut tx, customer_id, id).await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(a)
}

#[cfg(test)]
mod tests {
    use acom_contract::WriteOp;
    use acom_core::enums::AddressUsage;

    use super::*;

    pub(super) fn home() -> AddressFields {
        AddressFields {
            label: Some("Home".into()),
            name: Some("Jamie Rivera".into()),
            street_address: "12 Harbor St, Apt 3".into(),
            address_locality: "Brooklyn".into(),
            address_region: Some("NY".into()),
            postal_code: "11201".into(),
            address_country: "US".into(),
            telephone: None,
            usage: vec!["shipping".into(), "billing".into()],
        }
    }

    #[test]
    fn refuses_a_postal_code_of_another_shape() {
        assert!(postal_code_check("US", "11201", "add_address").is_ok());
        let e = postal_code_check("US", "112", "add_address").unwrap_err();
        assert_eq!(e.code, "invalid_postal_code");
        assert_eq!(e.next_tool.as_deref(), Some("add_address"));
    }

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn adds_updates_and_makes_an_address_primary() {
        let cx = crate::testdb::cx("cus_jamie").await;
        let planned = AddAddress
            .preview(
                &cx,
                AddAddressInput {
                    customer_id: "me".into(),
                    label: Some("Work".into()),
                    name: None,
                    street_address: "200 Water St".into(),
                    address_locality: "Brooklyn".into(),
                    address_region: Some("NY".into()),
                    postal_code: "11201".into(),
                    address_country: "US".into(),
                    telephone: None,
                    usage: vec![AddressUsage::Shipping],
                    is_primary: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            planned.changes[0].to,
            json!("200 Water St, Brooklyn, NY 11201")
        );
        assert!(!planned.plan.is_primary);
        let mut tx = cx.begin().await.unwrap();
        let done = AddAddress.commit(&mut tx, &planned.plan).await.unwrap();
        AddAddress.commit(&mut tx, &planned.plan).await.unwrap();
        let (total, _) = repo::addresses(&mut tx, "cus_jamie", 20, 0).await.unwrap();
        assert_eq!(total, 2);
        let plan = SetPrimaryPlan {
            customer_id: "cus_jamie".into(),
            address_id: done.id.clone(),
            previous: Some("adr_home".into()),
            changes_anything: true,
        };
        SetPrimaryAddress.commit(&mut tx, &plan).await.unwrap();
        let primary = repo::primary_address(&mut tx, "cus_jamie").await.unwrap();
        assert_eq!(primary.map(|p| p.id), Some(done.id.clone()));
        SetPrimaryAddress.undo(&mut tx, &plan, &done).await.unwrap();
        let update = UpdateAddressPlan {
            customer_id: "cus_jamie".into(),
            address_id: "adr_home".into(),
            before: home(),
            after: AddressFields {
                street_address: "12 Harbor St, Apt 4".into(),
                ..home()
            },
        };
        UpdateAddress.commit(&mut tx, &update).await.unwrap();
        let home_now = repo::address(&mut tx, "cus_jamie", "adr_home")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(home_now.street_address, "12 Harbor St, Apt 4");
        // Rolled back on drop.
    }
}
