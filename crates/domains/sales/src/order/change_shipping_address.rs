//! `change_shipping_address` (`sales.order.change_shipping_address`): where an order goes,
//! changed before it ships (`order.change.allowed_before`). The address is recorded as a new
//! order event, so the order keeps where it was going before; fulfillment reads the latest one
//! (`service::order_delivery`). A shopper changes their own orders, an operator any order.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_customer::service::for_sales as customer;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{DeliveryMode, PostalAddressInput, before_shipping, delivery_choice, visible_order};
use crate::delivery::{self, Address};
use crate::pricing::{self, DeliveryChoice};
use crate::repo::{self, OrderRow};
use crate::rules;
use crate::service::providers;
use crate::{
    ADDRESS_NOT_DELIVERABLE, ADDRESS_NOT_FOUND, ALREADY_SHIPPED, ORDER_NOT_FOUND, approval,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeShippingAddressInput {
    /// The order to send elsewhere (`get_my_orders`, `search_orders`).
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20518")]
    pub order_id: String,
    /// `saved`: one of the shopper's saved addresses (`delivery_address_id`); `one_time`: an
    /// address for this order only (`delivery_address`, not saved).
    #[schemars(example = &"one_time")]
    pub delivery_mode: DeliveryMode,
    /// A saved address of the order's shopper (`list_addresses`). Needed with `saved`.
    #[schemars(regex(pattern = r"^adr_[a-z0-9_]+$"), example = &"adr_home")]
    pub delivery_address_id: Option<String>,
    /// The new address, for this order only. Needed with `one_time`.
    #[schemars(example = json!({"name": "Jamie", "street_address": "40 Pier Rd",
        "address_locality": "Brooklyn", "address_region": "NY", "postal_code": "11231",
        "address_country": "US"}))]
    pub delivery_address: Option<PostalAddressInput>,
}

/// What `change_shipping_address` records: the order's delivery before and after.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct AddressPlan {
    order_id: String,
    /// The delivery the order had (`None` when it recorded none).
    from: Option<Value>,
    /// `{address_id, delivery_address, address_label, arrives, address_saved}`.
    to: Value,
}

pub(crate) struct ChangeShippingAddress;

impl Operation for ChangeShippingAddress {
    const META: OpMeta = OpMeta {
        address: "sales.order.change_shipping_address",
        tool: "change_shipping_address",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/sales/orders/{order_id}/delivery-address",
        summary: "Send an order that has not shipped yet to another address (a saved one or one for this order only), with the area surcharge checked again. Runs right after its preview unless a rule needs approval.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("order_id")],
        checks: &["change.allowed_before", "remote_area_surcharge"],
        errors: &[
            ALREADY_SHIPPED,
            ADDRESS_NOT_DELIVERABLE,
            ADDRESS_NOT_FOUND,
            ORDER_NOT_FOUND,
            ErrorCode::INVALID_STATE,
            ErrorCode::POLICY_VIOLATION,
        ],
        audience: Audience::Both,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = ChangeShippingAddressInput;
    type Output = Value;
}

/// The one line people read for a delivery record: "12 Harbor St, Apt 3, Brooklyn, NY 11201".
fn address_text(d: &Value) -> Value {
    serde_json::from_value::<Address>(d["delivery_address"].clone())
        .map(|a| json!(delivery::full_label(None, &a)))
        .unwrap_or(Value::Null)
}

fn country_of(d: &Value) -> Option<String> {
    d["delivery_address"]["address_country"]
        .as_str()
        .map(str::to_owned)
}

/// The new delivery record, from a saved address of the order's shopper or a one-time address.
async fn new_delivery(
    tx: &mut TxCx,
    o: &OrderRow,
    choice: &DeliveryChoice,
    previous: Option<&Value>,
) -> Result<Value, ToolError> {
    let (address_id, label, address) = match choice {
        DeliveryChoice::Saved(id) => {
            let saved = match o.customer_id.as_deref() {
                Some(c) => customer::address(tx, c, id).await?,
                None => None,
            };
            let a = saved.ok_or_else(|| {
                ToolError::new(
                    ADDRESS_NOT_FOUND,
                    format!("The order's shopper has no saved address {id}."),
                )
                .detail(json!({"entity": "postal_address", "id": id}))
                .next("list_addresses")
            })?;
            (Some(a.id.clone()), a.label.clone(), pricing::address_of(&a))
        }
        DeliveryChoice::OneTime(a) => (None, a.name.clone(), a.clone()),
        _ => return Err(ToolError::invalid_input("Name the new address.")),
    };
    let ships_to = rules::ships_to(&tx.policy);
    if !delivery::deliverable(&address.address_country, &ships_to) {
        return Err(ToolError::new(
            ADDRESS_NOT_DELIVERABLE,
            format!("The store does not ship to {}.", address.address_country),
        )
        .detail(
            json!({"country": address.address_country, "ships_to": ships_to,
                       "policy_key": "shipping.ships_to"}),
        )
        .next("list_addresses"));
    }
    // The address changes, not the day it ships: the estimate stays what the order said.
    let arrives = match previous.and_then(|p| p["arrives"].as_str()) {
        Some(a) => json!(a),
        None => {
            let calendar = rules::ship_calendar(&tx.policy)?;
            let local = tx.clock.now().with_timezone(&tx.time.zone).naive_local();
            json!(
                delivery::arrival(local, &calendar)
                    .format("%Y-%m-%d")
                    .to_string()
            )
        }
    };
    Ok(json!({
        "address_id": address_id,
        "delivery_address": address,
        "address_label": delivery::full_label(label.as_deref(), &address),
        "arrives": arrives,
        "address_saved": address_id.is_some(),
    }))
}

/// The delivery area of an address and what it adds to a parcel.
#[derive(Debug, Clone, PartialEq)]
struct Area {
    name: String,
    surcharge: rust_decimal::Decimal,
    currency: String,
}

/// The area a delivery's address is in: the store's saved shipping zone (fulfillment's reader),
/// or `fulfillment.delivery.default_zone` when no saved zone covers it. `None` without an address.
async fn area_of(
    tx: &mut TxCx,
    policy: &acom_policy::PolicySet,
    d: Option<&Value>,
) -> Result<Option<Area>, ToolError> {
    let Some(country) = d.and_then(country_of) else {
        return Ok(None);
    };
    let region = d.and_then(|d| d["delivery_address"]["address_region"].as_str());
    if let Some(read) = providers::areas()
        && let Some(a) = read(tx, &country, region).await?
    {
        return Ok(Some(Area {
            name: a.name,
            surcharge: a.surcharge.value,
            currency: a.surcharge.currency.to_string(),
        }));
    }
    default_area(policy, &country).map(Some)
}

/// `fulfillment.delivery.default_zone`: the area of every address while the store saves no zones.
fn default_area(policy: &acom_policy::PolicySet, country: &str) -> Result<Area, ToolError> {
    let zone = rules::default_zone(policy)?;
    let covered = zone
        .countries
        .iter()
        .any(|z| z.eq_ignore_ascii_case(country));
    Ok(Area {
        name: zone.name.clone(),
        surcharge: if covered {
            zone.surcharge.value
        } else {
            rust_decimal::Decimal::ZERO
        },
        currency: zone.surcharge.currency.to_string(),
    })
}

/// `remote_area_surcharge`: a new address may not cost more to reach than the old one, because a
/// surcharge cannot be charged after ordering.
fn surcharge_check(from: Option<&Area>, to: Option<&Area>) -> Result<Check, ToolError> {
    let cost = |a: Option<&Area>| a.map_or(rust_decimal::Decimal::ZERO, |a| a.surcharge);
    let extra = cost(to) - cost(from);
    if extra > rust_decimal::Decimal::ZERO {
        let (name, currency) = to.map_or(("", ""), |a| (a.name.as_str(), a.currency.as_str()));
        return Err(ToolError::new(
            ErrorCode::POLICY_VIOLATION,
            format!(
                "The new address is in {name}, which adds a {extra} {currency} area surcharge that cannot be charged after ordering."
            ),
        )
        .detail(json!({"rule": "remote_area_surcharge", "extra": extra.to_string(), "zone": name}))
        .next("list_addresses"));
    }
    Ok(Check {
        rule: "remote_area_surcharge".into(),
        passed: true,
        detail: Some(match to {
            Some(a) if !a.surcharge.is_zero() => {
                format!("{}: no more surcharge than the old address", a.name)
            }
            _ => "No surcharge for this address".to_owned(),
        }),
        fix_tool: None,
    })
}

impl WriteOp<Cx> for ChangeShippingAddress {
    type Plan = AddressPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ChangeShippingAddressInput,
    ) -> Result<Planned<AddressPlan>, ToolError> {
        let choice = delivery_choice(
            input.delivery_mode,
            input.delivery_address_id.as_ref(),
            input.delivery_address.as_ref(),
        )?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let o = visible_order(cx, &mut tx, &input.order_id).await?;
        let stage = before_shipping(&mut tx, &o, "get_order").await?;
        if let Some(g) = repo::gift_order_of(&mut tx, &o.id).await?
            && g.action_status == "potential"
        {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                "This gift waits for its recipient, who gives the address when accepting it.",
            )
            .detail(json!({"entity": "order", "reason": "recipient_gives_address"}))
            .next("get_order"));
        }
        let from = repo::latest_delivery(&mut tx, &o.id).await?;
        let to = new_delivery(&mut tx, &o, &choice, from.as_ref()).await?;
        let from_area = area_of(&mut tx, &cx.policy, from.as_ref()).await?;
        let to_area = area_of(&mut tx, &cx.policy, Some(&to)).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let surcharge = surcharge_check(from_area.as_ref(), to_area.as_ref())?;
        let change = Change {
            op: ChangeOp::Update,
            entity: "order".into(),
            id: o.id.clone(),
            from: json!({"delivery_address": from.as_ref().map_or(Value::Null, address_text)}),
            to: json!({"delivery_address": address_text(&to)}),
            note: (from.as_ref().map(|f| &f["delivery_address"]) == Some(&to["delivery_address"]))
                .then(|| "The order already goes to this address".to_owned()),
        };
        Ok(Planned {
            changes: vec![change],
            checks: vec![
                Check {
                    rule: "change.allowed_before".into(),
                    passed: true,
                    detail: Some(stage),
                    fix_tool: None,
                },
                surcharge,
            ],
            basis: vec![Basis {
                entity: "sales.sales_order".into(),
                id: o.id.clone(),
                version: i64::from(o.version),
            }],
            approval: approval(cx, Self::META.tool, &json!({"order_id": o.id})),
            plan: AddressPlan {
                order_id: o.id,
                from,
                to,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &AddressPlan) -> Result<Committed, ToolError> {
        let o = repo::order_for_update(tx, &plan.order_id)
            .await?
            .ok_or_else(|| ToolError::new(ORDER_NOT_FOUND, "The order is gone."))?;
        // Shipped in the meantime: the parcel left with the old address.
        before_shipping(tx, &o, "get_order").await?;
        let now = repo::latest_delivery(tx, &o.id).await?;
        if now.as_ref() != Some(&plan.to) {
            repo::append_event(
                tx,
                &o.id,
                ActionType::UPDATE.0,
                Self::META.tool,
                None,
                None,
                &json!({"delivery": plan.to, "previous_delivery": plan.from}),
            )
            .await?;
            repo::touch_order(tx, &o.id).await?;
        }
        let mut produced = Map::new();
        produced.insert("order_id".into(), json!(o.id));
        Ok(Committed { id: o.id, produced })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &AddressPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let now = repo::latest_delivery(&mut tx, &plan.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = now.as_ref() == Some(&plan.to);
        Ok(Verification {
            matched,
            mismatches: if matched {
                vec![]
            } else {
                vec![format!(
                    "order {} goes to {}",
                    plan.order_id,
                    now.as_ref().map_or(Value::Null, address_text)
                )]
            },
        })
    }

    /// Back to the address before, while the order has still not shipped.
    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &AddressPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        let Some(before) = &plan.from else {
            return Err(ToolError::new(
                ErrorCode::UNDO_WINDOW_CLOSED,
                "The order recorded no address before this change, so there is none to go back to.",
            ));
        };
        let o = repo::order_for_update(tx, &plan.order_id)
            .await?
            .ok_or_else(|| ToolError::new(ORDER_NOT_FOUND, "The order is gone."))?;
        before_shipping(tx, &o, "get_order").await?;
        repo::append_event(
            tx,
            &o.id,
            ActionType::UPDATE.0,
            "undo change_shipping_address",
            None,
            None,
            &json!({"delivery": before, "previous_delivery": plan.to}),
        )
        .await?;
        repo::touch_order(tx, &o.id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed_with_modes_as_words() {
        let s = closed_schema_for::<ChangeShippingAddressInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["order_id", "delivery_mode"]));
        assert_eq!(
            s["properties"]["delivery_mode"]["enum"],
            json!(["saved", "one_time"])
        );
    }

    fn area(name: &str, surcharge: &str) -> Area {
        Area {
            name: name.into(),
            surcharge: surcharge.parse().unwrap(),
            currency: "USD".into(),
        }
    }

    #[test]
    fn a_new_address_may_not_cost_more_to_reach() {
        let home = area("Contiguous US", "0");
        let pier = area("Contiguous US", "0");
        let c = surcharge_check(Some(&home), Some(&pier)).unwrap();
        assert!(c.passed);
        assert_eq!(c.detail.as_deref(), Some("No surcharge for this address"));
        assert!(surcharge_check(None, Some(&pier)).unwrap().passed);

        let anchorage = area("Alaska and Hawaii", "12.00");
        let e = surcharge_check(Some(&home), Some(&anchorage)).unwrap_err();
        assert_eq!(e.detail.as_ref().unwrap()["extra"], json!("12.00"));
        assert!(e.message.contains("Alaska and Hawaii"), "{}", e.message);
        // Moving within a surcharged area, or out of it, costs nothing more.
        let c = surcharge_check(Some(&anchorage), Some(&anchorage)).unwrap();
        assert_eq!(
            c.detail.as_deref(),
            Some("Alaska and Hawaii: no more surcharge than the old address")
        );
        assert!(
            surcharge_check(Some(&anchorage), Some(&home))
                .unwrap()
                .passed
        );
    }

    #[test]
    fn without_saved_zones_the_default_zone_applies() {
        let policy = crate::rules::tests::defaults();
        let zone = rules::default_zone(&policy).unwrap();
        let a = default_area(&policy, "us").unwrap();
        assert_eq!(
            (a.name, a.surcharge),
            (zone.name.clone(), zone.surcharge.value)
        );
        assert!(default_area(&policy, "ZZ").unwrap().surcharge.is_zero());
    }

    #[tokio::test]
    async fn a_mode_needs_its_address() {
        let cx = crate::testdb::offline("cus_jamie");
        let e = ChangeShippingAddress
            .preview(
                &cx,
                serde_json::from_value(json!({"order_id": "ord_20518", "delivery_mode": "saved"}))
                    .unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["field"], "delivery_address_id");
    }
}
