//! `get_delivery_dates`: the earliest day a product can arrive at an address.

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::Money;
use acom_customer::service::for_fulfillment as customer;
use acom_store::Cx;
use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::errors::*;
use crate::model::delivery::{estimate, local_date};
use crate::model::money_text;
use crate::{rules, shopper};

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
