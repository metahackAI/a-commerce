//! `list_addresses`, `add_address`, `update_address`, `set_primary_address`: a member's saved
//! addresses (`PostalAddress`). One address is primary (`address.primary_per_member`).

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, ReadOp, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::AddressUsage;
use acom_core::ids::AddressId;
use acom_store::{Cx, Page, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::access::{Caller, approval, caller, customer_for, policy_number};
use crate::person::customer_not_found;
use crate::repo::{self, AddressFields, AddressRow};
use crate::wording::{mask_postal_code, mask_street, postal_code_ok};
use crate::{ADDRESS_LIMIT_EXCEEDED, ADDRESS_NOT_FOUND, CUSTOMER_NOT_FOUND, INVALID_POSTAL_CODE};

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

// ── list_addresses ──────────────────────────────────────────────────────────────────────

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

// ── add_address ─────────────────────────────────────────────────────────────────────────

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

// ── update_address ──────────────────────────────────────────────────────────────────────

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

// ── set_primary_address ─────────────────────────────────────────────────────────────────

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
    fn inputs_are_closed_and_described() {
        for s in [
            closed_schema_for::<ListAddressesInput>(),
            closed_schema_for::<AddAddressInput>(),
            closed_schema_for::<UpdateAddressInput>(),
            closed_schema_for::<SetPrimaryAddressInput>(),
        ] {
            assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        }
    }

    fn home() -> AddressFields {
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
    fn shows_only_changed_fields() {
        let before = home();
        let after = AddressFields {
            street_address: "12 Harbor St, Apt 4".into(),
            ..home()
        };
        let (from, to) = changed_fields(&before, &after);
        assert_eq!(from, json!({"street_address": "12 Harbor St, Apt 3"}));
        assert_eq!(to, json!({"street_address": "12 Harbor St, Apt 4"}));
        assert_eq!(one_line(&before), "12 Harbor St, Apt 3, Brooklyn, NY 11201");
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
