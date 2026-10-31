//! `schedule_shipment`: tell the warehouse to ship an order's lines (or an exchange's
//! replacement) in one parcel. In stock it is `ship_instructed`; short of stock it waits
//! (`delivery.shortage_state`). The tracking number comes later with `confirm_shipment`.

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_customer::service::for_fulfillment as customer;
use acom_sales::service::for_fulfillment as sales;
use acom_sales::service::order_delivery;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, NaiveTime, TimeZone, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{CLOSED_PARCEL_STATES, parcel_lines, parcel_state_error};
use crate::errors::{NO_SHIP_ITEM, ORDER_ITEM_NOT_FOUND, ORDER_NOT_FOUND, OUT_OF_STOCK};
use crate::model::delivery::estimate_in;
use crate::model::zone::zone_for;
use crate::repo::{self, ClaimEvent, ClaimRow, DeliveryRow, NewDelivery};
use crate::shipping_rate::zones;
use crate::{broken, rules};

/// How the parcel travels (`delivery.methods`, schema.org `ParcelService`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMethod {
    /// The store's standard parcel service.
    Standard,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleShipmentInput {
    /// The order to ship.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20455")]
    pub order_id: String,
    /// Only these lines (a partial shipment, or an exchange's lines to reship). Every line
    /// waiting to ship when left out.
    #[schemars(inner(regex(pattern = r"^oln_[a-z0-9_]+$")), example = ["oln_1", "oln_2"])]
    pub order_item_ids: Option<Vec<String>>,
    /// The shipping group, when an order ships from several sellers, warehouses or to several
    /// addresses (`delivery.shipping_group_by`); the order's only group when left out.
    #[schemars(length(min = 1, max = 60), example = &"main")]
    pub shipping_group: Option<String>,
    /// How the parcel travels. standard when left out.
    #[schemars(example = &"standard")]
    pub delivery_method: Option<DeliveryMethod>,
    /// The carrier, when it is already known (it can also be given at confirm_shipment).
    #[schemars(length(min = 1, max = 40), example = &"UPS")]
    pub carrier: Option<String>,
}

/// One line in the parcel, with what it was before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShippedLine {
    pub order_item_id: String,
    pub units: i32,
    pub variant_id: String,
    /// The line's status and step before, for the order's own lines; `None` for a reshipment.
    pub before: Option<sales::ItemProgress>,
}

/// The parcel to create, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchedulePlan {
    pub delivery_id: String,
    pub order_id: String,
    /// The exchange this parcel reships, if it is a reshipment.
    pub claim_id: Option<String>,
    pub lines: Vec<ShippedLine>,
    pub delivery_state: String,
    pub address_id: Option<String>,
    pub address: Value,
    pub method: String,
    pub carrier: Option<String>,
    pub expected_arrival_until: Option<DateTime<Utc>>,
    /// The shipping group named by the operator, recorded with the parcel's first event.
    pub shipping_group: Option<String>,
}

pub struct ScheduleShipment;

impl Operation for ScheduleShipment {
    const META: OpMeta = OpMeta {
        address: "fulfillment.parcel_delivery.schedule",
        tool: "schedule_shipment",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/fulfillment/deliveries",
        summary: "Tell the warehouse to ship an order's lines (all waiting lines, or some) or an exchange's replacement in one parcel: ship_instructed when in stock, awaiting_stock when short. Gift cards are not shipped.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("delivery_id")],
        checks: &["delivery.shortage_state", "delivery.no_ship_for"],
        errors: &[
            ORDER_NOT_FOUND,
            ORDER_ITEM_NOT_FOUND,
            NO_SHIP_ITEM,
            OUT_OF_STOCK,
            ErrorCode::INVALID_STATE,
        ],
        audience: Audience::Operator,
        action_type: ActionType::SCHEDULE,
        person_scope: "",
    };
    type Input = ScheduleShipmentInput;
    type Output = Value;
}

fn nothing_to_ship(order_number: &str, why: String) -> ToolError {
    ToolError::new(
        ErrorCode::INVALID_STATE,
        format!("Nothing on order {order_number} waits to ship: {why}."),
    )
    .detail(json!({"entity": "order", "reason": why}))
    .next("get_delivery")
}

/// Lines already in a parcel that is still on its way (not cancelled), the order's own ones.
fn lines_in_parcels(deliveries: &[DeliveryRow], items: &[sales::OrderItem]) -> Vec<String> {
    deliveries
        .iter()
        .filter(|d| d.claim_id.is_none() && d.delivery_state != "cancelled")
        .flat_map(|d| parcel_lines(d, items).into_iter().map(|(id, _)| id))
        .collect()
}

/// Where the parcel goes: the address chosen at checkout, else the address on an earlier
/// parcel of the order, else the shopper's default shipping address.
async fn address_for(
    tx: &mut TxCx,
    order: &sales::Order,
    deliveries: &[DeliveryRow],
    claim: Option<&ClaimRow>,
) -> Result<Option<(Option<String>, Value)>, ToolError> {
    if let Some(c) = claim {
        // A replacement goes where the shopper asked when sending the exchange.
        let asked = repo::claim_events(tx, &c.id)
            .await?
            .into_iter()
            .find_map(|e| {
                let r = e.result?;
                let a = r.get("delivery_address").filter(|a| a.is_object())?.clone();
                let id = r
                    .get("delivery_address_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Some((id, a))
            });
        if asked.is_some() {
            return Ok(asked);
        }
    }
    if claim.is_none()
        && let Some(d) = order_delivery::delivery(tx, &order.id).await?
        && let Some(a) = d.get("delivery_address").filter(|a| a.is_object())
    {
        let id = d
            .get("address_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        return Ok(Some((id, a.clone())));
    }
    if let Some(d) = deliveries
        .iter()
        .filter(|d| d.claim_id.is_none())
        .find(|d| d.delivery_address.is_object())
    {
        return Ok(Some((None, d.delivery_address.clone())));
    }
    if let Some(me) = &order.customer_id
        && let Some(a) = customer::default_shipping_address(tx, me).await?
    {
        return Ok(Some((
            Some(a.id.clone()),
            json!({"name": a.name, "street_address": a.street_address,
                   "address_locality": a.address_locality, "address_region": a.address_region,
                   "postal_code": a.postal_code, "address_country": a.address_country}),
        )));
    }
    Ok(None)
}

impl WriteOp<Cx> for ScheduleShipment {
    type Plan = SchedulePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ScheduleShipmentInput,
    ) -> Result<Planned<SchedulePlan>, ToolError> {
        if input.order_item_ids.as_ref().is_some_and(Vec::is_empty) {
            return Err(ToolError::invalid_input("Name at least one line.")
                .detail(json!({"field": "order_item_ids", "reason": "range"})));
        }
        let rules = rules(&cx.policy)?;
        let method = match input.delivery_method.unwrap_or(DeliveryMethod::Standard) {
            DeliveryMethod::Standard => "standard",
        };
        if !rules.delivery_methods.iter().any(|m| m == method) {
            return Err(
                ToolError::invalid_input(format!("The store does not ship by {method}."))
                    .detail(json!({"field": "delivery_method", "allowed": rules.delivery_methods})),
            );
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let order = sales::order(&mut tx, &input.order_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    ORDER_NOT_FOUND,
                    format!("There is no order {}.", input.order_id),
                )
                .detail(json!({"entity": "order", "id": input.order_id}))
                .next("search_orders")
            })?;
        let items = sales::order_items(&mut tx, &order.id).await?;
        let deliveries =
            repo::deliveries_of_orders(&mut tx, std::slice::from_ref(&order.id)).await?;
        let claims = repo::claims_on_order(&mut tx, &order.id).await?;
        let claim_ids: Vec<String> = claims.iter().map(|c| c.id.clone()).collect();
        let claim_items = repo::claim_items(&mut tx, &claim_ids).await?;

        let in_parcels = lines_in_parcels(&deliveries, &items);
        let waiting: Vec<&sales::OrderItem> = items
            .iter()
            .filter(|i| i.order_item_status == "processing" && !in_parcels.contains(&i.id))
            .collect();
        let reships: Vec<&ClaimRow> = claims
            .iter()
            .filter(|c| c.claim_type == "exchange" && c.claim_status == "reship_requested")
            .filter(|c| {
                !deliveries.iter().any(|d| {
                    d.claim_id.as_deref() == Some(c.id.as_str()) && d.delivery_state != "cancelled"
                })
            })
            .collect();
        let reship_lines = |c: &ClaimRow| -> Vec<(String, i32, String)> {
            claim_items
                .iter()
                .filter(|i| i.claim_id == c.id)
                .filter_map(|i| {
                    i.replacement_product_id
                        .clone()
                        .map(|v| (i.order_item_id.clone(), i.order_quantity, v))
                })
                .collect()
        };

        // The lines this parcel carries: (order_item_id, units, variant), and the exchange.
        let (picked, claim): (Vec<(String, i32, String)>, Option<&ClaimRow>) =
            match &input.order_item_ids {
                Some(ids) => {
                    if let Some(c) = reships.iter().find(|c| {
                        let mine = reship_lines(c);
                        ids.iter().all(|id| mine.iter().any(|(l, _, _)| l == id))
                    }) {
                        let lines = reship_lines(c)
                            .into_iter()
                            .filter(|(l, _, _)| ids.contains(l))
                            .collect();
                        (lines, Some(*c))
                    } else {
                        let mut lines = Vec::new();
                        for id in ids {
                            let Some(item) = items.iter().find(|i| &i.id == id) else {
                                return Err(ToolError::new(
                                    ORDER_ITEM_NOT_FOUND,
                                    format!("Order {} has no line {id}.", order.order_number),
                                )
                                .detail(json!({"entity": "order_item", "id": id}))
                                .next("get_order"));
                            };
                            if !waiting.iter().any(|w| w.id == item.id) {
                                let why = if in_parcels.contains(&item.id) {
                                    format!("{} is already in a parcel", item.name)
                                } else {
                                    format!(
                                        "{} is {}",
                                        item.name,
                                        item.order_item_status.replace('_', " ")
                                    )
                                };
                                return Err(nothing_to_ship(&order.order_number, why));
                            }
                            lines.push((
                                item.id.clone(),
                                item.order_quantity,
                                item.variant_id.clone(),
                            ));
                        }
                        (lines, None)
                    }
                }
                None if !waiting.is_empty() => (
                    waiting
                        .iter()
                        .map(|i| (i.id.clone(), i.order_quantity, i.variant_id.clone()))
                        .collect(),
                    None,
                ),
                None => match reships.first() {
                    Some(c) => (reship_lines(c), Some(*c)),
                    None => {
                        return Err(nothing_to_ship(
                            &order.order_number,
                            format!("the order is {}", order.order_status.replace('_', " ")),
                        ));
                    }
                },
            };

        // Gift cards are not shipped (`delivery.no_ship_for`): left out when the line was not
        // named, refused when it was.
        let no_ship = rules.list("delivery.no_ship_for");
        let mut changes = Vec::new();
        let mut kept = Vec::new();
        let mut left_out = Vec::new();
        let mut short = Vec::new();
        for (line, units, variant) in picked {
            let v = catalog::variant(&mut tx, &variant).await?;
            let name = items
                .iter()
                .find(|i| i.id == line)
                .map_or_else(|| line.clone(), |i| i.name.clone());
            match v {
                Some(v) if no_ship.contains(&v.sale_type) => {
                    if input.order_item_ids.is_some() {
                        return Err(ToolError::new(
                            NO_SHIP_ITEM,
                            format!(
                                "{name} is a {} and is not shipped; leave it out.",
                                v.sale_type.replace('_', " ")
                            ),
                        )
                        .detail(json!({"rule": "delivery.no_ship_for", "items": [line]}))
                        .next("schedule_shipment"));
                    }
                    changes.push(Change {
                        op: ChangeOp::Exclude,
                        entity: "order_item".into(),
                        id: line.clone(),
                        from: Value::Null,
                        to: Value::Null,
                        note: Some(format!("{name} is not shipped")),
                    });
                    left_out.push(name);
                }
                Some(v) => {
                    let gone = v
                        .stock
                        .as_ref()
                        .is_none_or(|s| s.availability == "discontinued");
                    if gone {
                        return Err(ToolError::new(
                            OUT_OF_STOCK,
                            format!("{name} is no longer sold, so it cannot ship; register the shortage."),
                        )
                        .detail(json!({"order_item_id": line, "variant_id": v.id}))
                        .next("register_shortage"));
                    }
                    let waits = v.stock.as_ref().is_some_and(|s| {
                        matches!(s.availability.as_str(), "pre_order" | "back_order")
                    });
                    if waits && claim.is_none() {
                        short.push(name);
                    }
                    kept.push((line, units, variant));
                }
                None => return Err(broken(format!("variant {variant} is missing"))),
            }
        }
        if kept.is_empty() {
            return Err(nothing_to_ship(
                &order.order_number,
                "only items that are not shipped are left".into(),
            ));
        }
        let address = address_for(&mut tx, &order, &deliveries, claim).await?;
        let all_zones = zones(&mut tx, &rules).await?;
        let mut lines = Vec::new();
        let mut basis = Vec::new();
        for (line, units, variant) in &kept {
            let before = if claim.is_none() {
                items.iter().find(|i| &i.id == line).map(|i| {
                    basis.push(Basis {
                        entity: "sales.order_item".into(),
                        id: i.id.clone(),
                        version: i64::from(i.version),
                    });
                    sales::ItemProgress {
                        order_item_status: i.order_item_status.clone(),
                        fulfillment_stage: i.fulfillment_stage.clone(),
                    }
                })
            } else {
                None
            };
            lines.push(ShippedLine {
                order_item_id: line.clone(),
                units: *units,
                variant_id: variant.clone(),
                before,
            });
        }
        tx.commit().await.map_err(ToolError::from)?;
        let Some((address_id, address)) = address else {
            return Err(nothing_to_ship(
                &order.order_number,
                "the order has no delivery address".into(),
            ));
        };

        let shortage_state = rules
            .lookup("delivery.shortage_state")
            .and_then(Value::as_str)
            .unwrap_or("awaiting_stock")
            .to_owned();
        let state = if short.is_empty() {
            "ship_instructed".to_owned()
        } else {
            shortage_state
        };
        let zone = zone_for(
            &all_zones,
            address
                .get("address_country")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            address.get("address_region").and_then(Value::as_str),
        );
        let transit = zone.map_or(rules.transit_business_days, |z| z.transit_business_days);
        let (_, arrives) = estimate_in(&rules, cx.clock.now(), cx.time.zone, transit);
        let evening = NaiveTime::from_hms_opt(20, 0, 0).unwrap_or(NaiveTime::MIN);
        let expected = cx
            .time
            .zone
            .from_local_datetime(&arrives.and_time(evening))
            .earliest()
            .map(|t| t.with_timezone(&Utc));

        let ids: Vec<&String> = lines.iter().map(|l| &l.order_item_id).collect();
        changes.insert(
            0,
            Change {
                op: ChangeOp::Create,
                entity: "parcel_delivery".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"order_id": order.id, "delivery_state": state, "order_item_ids": ids,
                           "claim_id": claim.map(|c| &c.id), "expected_arrival_until": arrives.to_string()}),
                note: claim.map(|c| format!("Reships the replacement of exchange {}", c.id)),
            },
        );
        for l in &lines {
            if let Some(b) = &l.before {
                changes.push(Change {
                    op: ChangeOp::Update,
                    entity: "order_item".into(),
                    id: l.order_item_id.clone(),
                    from: json!({"fulfillment_stage": b.fulfillment_stage}),
                    to: json!({"fulfillment_stage": state}),
                    note: None,
                });
            }
        }
        let count = |n: usize| {
            if n == 1 {
                "1 item".to_owned()
            } else {
                format!("{n} items")
            }
        };
        let checks = vec![
            Check {
                rule: "delivery.shortage_state".into(),
                passed: true,
                detail: Some(if short.is_empty() {
                    format!("All {} in stock", count(lines.len()))
                } else {
                    format!(
                        "{} waits for stock ({}); the parcel is {}",
                        count(short.len()),
                        short.join(", "),
                        state.replace('_', " ")
                    )
                }),
                fix_tool: Some("register_shortage".into()),
            },
            Check {
                rule: "delivery.no_ship_for".into(),
                passed: true,
                detail: Some(if left_out.is_empty() {
                    "Nothing in this parcel is a gift card".into()
                } else {
                    format!("Left out, not shipped: {}", left_out.join(", "))
                }),
                fix_tool: None,
            },
        ];
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: SchedulePlan {
                delivery_id: format!("dlv_{}", acom_core::id::new_suffix()),
                order_id: order.id.clone(),
                claim_id: claim.map(|c| c.id.clone()),
                lines,
                delivery_state: state,
                address_id,
                address,
                method: method.into(),
                carrier: input
                    .carrier
                    .map(|c| c.trim().to_owned())
                    .filter(|c| !c.is_empty()),
                expected_arrival_until: expected,
                shipping_group: input
                    .shipping_group
                    .map(|g| g.trim().to_owned())
                    .filter(|g| !g.is_empty()),
            },
            changes,
            checks,
            basis,
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Schedules a shipment".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SchedulePlan) -> Result<Committed, ToolError> {
        let done = Committed {
            id: plan.delivery_id.clone(),
            produced: Map::from_iter([
                ("delivery_id".to_owned(), json!(plan.delivery_id)),
                ("order_id".to_owned(), json!(plan.order_id)),
            ]),
        };
        if repo::delivery(tx, &plan.delivery_id).await?.is_some() {
            return Ok(done);
        }
        // The same lines must not go out twice (`(order_id, shipping_group, order_item_ids)`).
        let deliveries =
            repo::deliveries_of_orders(tx, std::slice::from_ref(&plan.order_id)).await?;
        let items = sales::order_items(tx, &plan.order_id).await?;
        let taken = match &plan.claim_id {
            None => {
                let in_parcels = lines_in_parcels(&deliveries, &items);
                plan.lines
                    .iter()
                    .find(|l| in_parcels.contains(&l.order_item_id))
                    .map(|l| l.order_item_id.clone())
            }
            Some(c) => deliveries
                .iter()
                .find(|d| {
                    d.claim_id.as_deref() == Some(c.as_str())
                        && !CLOSED_PARCEL_STATES.contains(&d.delivery_state.as_str())
                })
                .map(|d| d.id.clone()),
        };
        if let Some(t) = taken {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("{t} is already being shipped."),
            )
            .detail(json!({"entity": "parcel_delivery", "existing_id": t}))
            .next("get_delivery"));
        }
        let shipped: Vec<Value> = plan
            .lines
            .iter()
            .map(|l| {
                let mut v = json!({"order_item_id": l.order_item_id, "order_quantity": l.units});
                if plan.claim_id.is_some() {
                    v["variant_id"] = json!(l.variant_id);
                }
                v
            })
            .collect();
        repo::insert_delivery(
            tx,
            &NewDelivery {
                id: &plan.delivery_id,
                order_id: &plan.order_id,
                item_shipped: Value::Array(shipped),
                delivery_state: &plan.delivery_state,
                delivery_address_id: plan.address_id.as_deref(),
                delivery_address: plan.address.clone(),
                method: &plan.method,
                carrier: plan.carrier.as_deref(),
                expected_arrival_until: plan.expected_arrival_until,
                claim_id: plan.claim_id.as_deref(),
            },
        )
        .await?;
        for l in plan.lines.iter().filter(|l| l.before.is_some()) {
            sales::set_item_progress(tx, &l.order_item_id, None, Some(&plan.delivery_state))
                .await?;
        }
        repo::append_delivery_event(
            tx,
            &plan.delivery_id,
            &ClaimEvent {
                action_type: "CreateAction",
                action_status: "completed",
                instrument: Self::META.tool,
                from_state: None,
                to_state: Some(&plan.delivery_state),
                result: Some(json!({"order_item_ids": plan.lines.iter().map(|l| &l.order_item_id).collect::<Vec<_>>(),
                                    "claim_id": plan.claim_id, "shipping_group": plan.shipping_group})),
            },
        )
        .await?;
        if let Some(c) = &plan.claim_id {
            repo::append_event(
                tx,
                c,
                &ClaimEvent {
                    action_type: "ScheduleAction",
                    action_status: "active",
                    instrument: Self::META.tool,
                    from_state: None,
                    to_state: None,
                    result: Some(json!({"delivery_id": plan.delivery_id})),
                },
            )
            .await?;
        }
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SchedulePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let d = repo::delivery(&mut tx, &plan.delivery_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match d {
            None => vec![format!("parcel {} is missing", plan.delivery_id)],
            Some(d) if d.order_id == plan.order_id && d.delivery_state != "cancelled" => Vec::new(),
            Some(d) => vec![format!("parcel {} is {}", d.id, d.delivery_state)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SchedulePlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        let d = repo::delivery_for_update(tx, &plan.delivery_id)
            .await?
            .ok_or_else(|| broken(format!("parcel {} is missing", plan.delivery_id)))?;
        if d.delivery_state != plan.delivery_state {
            return Err(parcel_state_error(&d, &["cancelled"], "taken back"));
        }
        repo::set_delivery_state(tx, &d.id, "cancelled").await?;
        for l in &plan.lines {
            if let Some(b) = &l.before {
                sales::set_item_progress(
                    tx,
                    &l.order_item_id,
                    Some(&b.order_item_status),
                    b.fulfillment_stage.as_deref(),
                )
                .await?;
            }
        }
        repo::append_delivery_event(
            tx,
            &d.id,
            &ClaimEvent {
                action_type: "UpdateAction",
                action_status: "completed",
                instrument: "undo_change_set",
                from_state: Some(&plan.delivery_state),
                to_state: Some("cancelled"),
                result: None,
            },
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    #[tokio::test]
    async fn needs_at_least_one_line_when_naming_lines() {
        let e = ScheduleShipment
            .preview(
                &offline("adm_sj"),
                input(json!({"order_id": "ord_20455", "order_item_ids": []})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[test]
    fn only_the_standard_method_is_a_word() {
        assert!(
            serde_json::from_value::<ScheduleShipmentInput>(
                json!({"order_id": "ord_20455", "delivery_method": "express"})
            )
            .is_err()
        );
    }
}
