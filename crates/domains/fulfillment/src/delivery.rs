//! Parcels: `get_delivery_dates` (when a product can arrive), `get_delivery`, `track_shipment`.

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::Money;
use acom_customer::service::for_fulfillment as customer;
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, Page, TxCx};
use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::errors::{
    ADDRESS_NOT_DELIVERABLE, ADDRESS_NOT_FOUND, DELIVERY_NOT_FOUND, OUT_OF_STOCK,
    PRODUCT_NOT_FOUND, TRACKING_NOT_FOUND,
};
use crate::model::delivery::{estimate, local_date};
use crate::model::money_text;
use crate::repo::{self, DeliveryRow};
use crate::{rules, select, shopper};

/// Which address the estimate is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AddressMode {
    /// One of the shopper's saved addresses (`address_id`).
    Saved,
    /// An address used once and not saved, such as a gift recipient's (`delivery_address`).
    OneTime,
}

/// A postal address given for this estimate only.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddressInput {
    /// Street, number and unit.
    #[schemars(example = &"48 Pine Rd")]
    pub street_address: String,
    /// City or town.
    #[schemars(example = &"Lakeview")]
    pub address_locality: String,
    /// State or region code.
    #[schemars(example = &"NY")]
    pub address_region: Option<String>,
    /// Postal or ZIP code.
    #[schemars(example = &"14085")]
    pub postal_code: String,
    /// ISO 3166-1 alpha-2 country code.
    #[schemars(regex(pattern = r"^[A-Z]{2}$"), example = &"US")]
    pub address_country: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetDeliveryDatesInput {
    /// The product people talk about.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_windbreaker")]
    pub product_id: String,
    /// The size and color chosen, when there is one (the stock check is then exact).
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_windbreaker_m_slate")]
    pub variant_id: Option<String>,
    /// saved: one of the shopper's saved addresses (needs address_id); one_time: an address
    /// for this estimate only, not saved (needs delivery_address).
    #[schemars(example = &"saved")]
    pub address_mode: AddressMode,
    /// The saved address, when address_mode is saved.
    #[schemars(regex(pattern = r"^adr_[a-z0-9_]+$"), example = &"adr_home")]
    pub address_id: Option<String>,
    /// The address, when address_mode is one_time.
    pub delivery_address: Option<AddressInput>,
    /// The day it has to arrive by.
    #[schemars(example = &"2026-11-14")]
    pub by: Option<String>,
}

pub struct GetDeliveryDates;

impl Operation for GetDeliveryDates {
    const META: OpMeta = OpMeta {
        address: "fulfillment.parcel_delivery.get_delivery_dates",
        tool: "get_delivery_dates",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/delivery-dates",
        summary: "The earliest day a product can arrive at an address, its ship day and shipping cost; with `by`, whether it arrives in time.",
        consumes: &[
            Slot::one("product_id"),
            Slot::one("variant_id"),
            Slot::one("address_id"),
        ],
        produces: &[],
        checks: &[],
        errors: &[
            PRODUCT_NOT_FOUND,
            ADDRESS_NOT_FOUND,
            ADDRESS_NOT_DELIVERABLE,
            OUT_OF_STOCK,
        ],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetDeliveryDatesInput;
    type Output = Value;
}

fn in_stock(v: &catalog::Variant) -> bool {
    !v.is_managed || v.stock.as_ref().is_some_and(|s| s.inventory_level > 0)
}

impl ReadOp<Cx> for GetDeliveryDates {
    async fn read(&self, cx: &Cx, input: GetDeliveryDatesInput) -> Result<Value, ToolError> {
        let rules = rules(&cx.policy)?;
        let by = input
            .by
            .as_deref()
            .map(|s| {
                cx.time
                    .parse(s)
                    .map(|t| local_date(t, cx.time.zone))
                    .map_err(|e| {
                        ToolError::invalid_input(e.to_string())
                            .detail(json!({"field": "by", "reason": "format"}))
                    })
            })
            .transpose()?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let variants = catalog::product_variants(&mut tx, &input.product_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    PRODUCT_NOT_FOUND,
                    format!("There is no product {}.", input.product_id),
                )
                .detail(json!({"entity": "product", "id": input.product_id}))
                .next("search_products")
            })?;
        let variant = match &input.variant_id {
            Some(id) => variants.iter().find(|v| &v.id == id).ok_or_else(|| {
                ToolError::invalid_input(format!(
                    "{id} is not a size or color of {}.",
                    input.product_id
                ))
                .detail(json!({"field": "variant_id", "reason": "format"}))
            })?,
            None => variants
                .iter()
                .find(|v| in_stock(v))
                .or(variants.first())
                .ok_or_else(|| out_of_stock(&input.product_id))?,
        };
        if !in_stock(variant) {
            return Err(out_of_stock(&input.product_id));
        }
        let country = match input.address_mode {
            AddressMode::Saved => {
                let Some(address_id) = &input.address_id else {
                    return Err(
                        ToolError::invalid_input("address_mode saved needs address_id.")
                            .detail(json!({"field": "address_id", "reason": "missing"})),
                    );
                };
                let me = shopper(&cx.principal);
                let found = match &me {
                    Some(me) => customer::address(&mut tx, me, address_id).await?,
                    None => None,
                };
                found
                    .ok_or_else(|| {
                        ToolError::new(
                            ADDRESS_NOT_FOUND,
                            format!("There is no saved address {address_id}."),
                        )
                        .detail(json!({"entity": "address", "id": address_id}))
                        .next("get_my_profile")
                    })?
                    .address_country
            }
            AddressMode::OneTime => {
                let Some(a) = &input.delivery_address else {
                    return Err(ToolError::invalid_input(
                        "address_mode one_time needs delivery_address.",
                    )
                    .detail(json!({"field": "delivery_address", "reason": "missing"})));
                };
                let blank = [&a.street_address, &a.address_locality, &a.postal_code]
                    .iter()
                    .any(|f| f.trim().is_empty())
                    || a.address_region
                        .as_deref()
                        .is_some_and(|r| r.trim().is_empty());
                if blank {
                    return Err(ToolError::invalid_input(
                        "The address needs a street, a city and a postal code.",
                    )
                    .detail(json!({"field": "delivery_address", "reason": "missing"})));
                }
                a.address_country.clone()
            }
        };
        tx.commit().await.map_err(ToolError::from)?;
        let no_ship = rules.list("delivery.no_ship_for");
        if !rules.zone_countries.contains(&country) || no_ship.contains(&variant.sale_type) {
            return Err(ToolError::new(
                ADDRESS_NOT_DELIVERABLE,
                if no_ship.contains(&variant.sale_type) {
                    format!("{} is not shipped.", variant.product_name)
                } else {
                    format!("The store does not deliver to {country}.")
                },
            )
            .detail(json!({"country": country}))
            .next("get_my_profile"));
        }

        let (ships_on, arrives) = estimate(&rules, cx.clock.now(), cx.time.zone);
        let price = variant
            .stock
            .as_ref()
            .map_or(Money::zero(rules.shipping_rate.currency), |s| s.price);
        let free = price.value >= rules.free_shipping_min.value;
        let item = json!({
            "product_id": input.product_id,
            "variant_id": variant.id,
            "method": rules.delivery_methods.first(),
            "ships_on": ships_on.to_string(),
            "earliest_arrival": arrives.to_string(),
            "shipping": if free { Money::zero(rules.shipping_rate.currency) } else { rules.shipping_rate },
            "shipping_note": if free {
                format!("Free over {} before member benefits", money_text(rules.free_shipping_min))
            } else {
                format!("{} standard shipping; free over {} before member benefits",
                        money_text(rules.shipping_rate), money_text(rules.free_shipping_min))
            },
            "remote_area_surcharge": rules.zone_surcharge,
            "by": by.map(|d| d.to_string()),
            "arrives_by_requested_date": by.map(|d: NaiveDate| arrives <= d),
        });
        Ok(json!({"items": [item], "total": 1}))
    }
}

fn out_of_stock(product_id: &str) -> ToolError {
    ToolError::new(OUT_OF_STOCK, "That size and color is out of stock.")
        .detail(json!({"product_id": product_id}))
        .next("get_product")
}

/// A delivery the caller may see: shoppers only see parcels of their own orders.
async fn visible_delivery(
    tx: &mut TxCx,
    d: Option<DeliveryRow>,
) -> Result<Option<DeliveryRow>, ToolError> {
    let Some(d) = d else {
        return Ok(None);
    };
    if let Some(me) = shopper(&tx.principal) {
        let owner = sales::order(tx, &d.order_id)
            .await?
            .and_then(|o| o.customer_id);
        if owner.as_deref() != Some(me.as_str()) {
            return Ok(None);
        }
    }
    Ok(Some(d))
}

/// The parcel's history as carrier events: picked up when shipped, delivered when delivered,
/// and the current state when no event explains it.
fn events(cx: &Cx, d: &DeliveryRow) -> Vec<Value> {
    let place = {
        let a = &d.delivery_address;
        match (
            a.get("address_locality").and_then(Value::as_str),
            a.get("address_region").and_then(Value::as_str),
        ) {
            (Some(city), Some(region)) => Some(format!("{city}, {region}")),
            (Some(city), None) => Some(city.to_owned()),
            _ => None,
        }
    };
    let mut out = Vec::new();
    if let Some(t) = d.shipped_at {
        out.push(json!({"at": cx.time.format(t), "delivery_state": "picked_up", "location": null}));
    }
    if let Some(t) = d.delivered_at {
        out.push(
            json!({"at": cx.time.format(t), "delivery_state": "delivered", "location": place}),
        );
    }
    let covered = (d.delivery_state == "delivered" && d.delivered_at.is_some())
        || (d.delivery_state == "picked_up" && d.shipped_at.is_some());
    if !covered {
        out.push(
            json!({"at": cx.time.format(d.updated_at), "delivery_state": d.delivery_state,
                        "location": null}),
        );
    }
    out
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetDeliveryInput {
    /// The parcel.
    #[schemars(regex(pattern = r"^dlv_[a-z0-9_]+$"), example = &"dlv_20402")]
    pub delivery_id: String,
    /// Top-level fields to return, for example only the state.
    #[schemars(example = ["delivery_state", "expected_arrival_until"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetDelivery;

impl Operation for GetDelivery {
    const META: OpMeta = OpMeta {
        address: "fulfillment.parcel_delivery.get",
        tool: "get_delivery",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/deliveries/{delivery_id}",
        summary: "One parcel: where it goes, carrier and tracking number, its state and history, and when it is expected.",
        consumes: &[Slot::one("delivery_id")],
        produces: &[Slot::one("order_id")],
        checks: &[],
        errors: &[DELIVERY_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetDeliveryInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetDelivery {
    async fn read(&self, cx: &Cx, input: GetDeliveryInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::delivery(&mut tx, &input.delivery_id).await?;
        let d = visible_delivery(&mut tx, row).await?.ok_or_else(|| {
            ToolError::new(
                DELIVERY_NOT_FOUND,
                format!("There is no parcel {}.", input.delivery_id),
            )
            .detail(json!({"entity": "parcel_delivery", "id": input.delivery_id}))
            .next("get_order")
        })?;
        tx.commit().await.map_err(ToolError::from)?;
        let zone = cx.time.zone;
        let out = json!({
            "id": d.id,
            "order_id": d.order_id,
            "delivery_address": d.delivery_address.get("street_address"),
            "tracking_number": d.tracking_number,
            "tracking_url": d.tracking_url,
            "carrier": d.carrier,
            "has_delivery_method": d.has_delivery_method,
            "delivery_state": d.delivery_state,
            "expected_arrival_until": d.expected_arrival_until.map(|t| local_date(t, zone).to_string()),
            "events": events(cx, &d),
        });
        Ok(select(out, &input.fields.unwrap_or_default()))
    }
}

/// How to find the parcel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrackBy {
    /// By the parcel id (`delivery_id`).
    Delivery,
    /// By the carrier's tracking number (`tracking_number`).
    TrackingNumber,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackShipmentInput {
    /// delivery: find it by delivery_id; tracking_number: by the carrier's number.
    #[schemars(example = &"delivery")]
    pub mode: TrackBy,
    /// The parcel, when mode is delivery.
    #[schemars(regex(pattern = r"^dlv_[a-z0-9_]+$"), example = &"dlv_20402")]
    pub delivery_id: Option<String>,
    /// The carrier's tracking number, when mode is tracking_number.
    #[schemars(length(min = 1, max = 64), example = &"1ZNW204020001")]
    pub tracking_number: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct TrackShipment;

impl Operation for TrackShipment {
    const META: OpMeta = OpMeta {
        address: "fulfillment.parcel_delivery.track",
        tool: "track_shipment",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/deliveries/{delivery_id}/tracking",
        summary: "The carrier's tracking events for a parcel, found by parcel id or tracking number.",
        consumes: &[Slot::one("delivery_id")],
        produces: &[],
        checks: &[],
        errors: &[TRACKING_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = TrackShipmentInput;
    type Output = Value;
}

impl ReadOp<Cx> for TrackShipment {
    async fn read(&self, cx: &Cx, input: TrackShipmentInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = match input.mode {
            TrackBy::Delivery => match &input.delivery_id {
                Some(id) => repo::delivery(&mut tx, id).await?,
                None => {
                    return Err(ToolError::invalid_input("mode delivery needs delivery_id.")
                        .detail(json!({"field": "delivery_id", "reason": "missing"})));
                }
            },
            TrackBy::TrackingNumber => match &input.tracking_number {
                Some(n) => repo::delivery_by_tracking(&mut tx, n).await?,
                None => {
                    return Err(ToolError::invalid_input(
                        "mode tracking_number needs tracking_number.",
                    )
                    .detail(json!({"field": "tracking_number", "reason": "missing"})));
                }
            },
        };
        let d = visible_delivery(&mut tx, row).await?.ok_or_else(|| {
            ToolError::new(
                TRACKING_NOT_FOUND,
                "No parcel matches that tracking request.",
            )
            .detail(json!({"entity": "parcel_delivery"}))
            .next("get_delivery")
        })?;
        tx.commit().await.map_err(ToolError::from)?;
        let all = events(cx, &d);
        let total = all.len() as i64;
        let items: Vec<Value> = all
            .into_iter()
            .skip(usize::try_from(page.offset).unwrap_or(0))
            .take(usize::try_from(page.limit).unwrap_or(0))
            .collect();
        let mut out = json!({"items": items, "total": total, "delivery_id": d.id,
                             "carrier": d.carrier, "tracking_number": d.tracking_number});
        if let Some(next) = page.next_cursor(items.len(), total) {
            out["next_cursor"] = json!(next);
        }
        Ok(out)
    }
}
