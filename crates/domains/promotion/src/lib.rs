//! The `promotion` domain: promotions, member sale prices, coupons, benefits, loyalty, events, segments.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/promotion.md`.
//! Layout: `model` (rules as pure functions), one module per resource with one type per
//! operation implementing `ReadOp`/`WriteOp`, `repo` (SQL). Other domains are reached only
//! through the service functions they export, never through their tables.

pub mod benefit;
pub mod service;

mod coupon;
mod loyalty;
mod model;
mod offer;
mod promotion;
mod repo;

#[cfg(test)]
mod b1;
#[cfg(test)]
mod contract;
#[cfg(test)]
mod integration;
#[cfg(test)]
mod testing;

use acom_contract::{ErrorClass, ErrorCode, RegistryBuilder, ToolError};
use acom_core::StoreTime;
use acom_customer::service::for_promotion::{self as customers, Tier};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};

/// Adds every operation of this domain to the registry.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    benefit::register(b);
    b.read(promotion::ListActivePromotions)
        .read(promotion::SearchPromotions)
        .read(promotion::GetPromotion)
        .write(promotion::PausePromotion)
        .write(promotion::EndPromotion)
        .write(offer::SetSalePrices)
        .write(coupon::CreateCoupon)
        .write(coupon::IssueCoupon)
        .read(coupon::SearchCoupons)
        .read(coupon::GetCoupon)
        .read(coupon::ListMemberCoupons)
        .read(loyalty::GetLoyaltyBalance)
        .read(loyalty::ListLoyaltyTransactions);
}

pub const TIER_NOT_FOUND: ErrorCode = ErrorCode::new("tier_not_found", ErrorClass::NotFound);
pub const INVALID_PERIOD: ErrorCode = ErrorCode::new("invalid_period", ErrorClass::InvalidInput);
pub const INVALID_FILTER: ErrorCode = ErrorCode::new("invalid_filter", ErrorClass::InvalidInput);

/// Store time from an input field; a bad value is `invalid_input` naming the field.
pub(crate) fn parse_time(
    time: &StoreTime,
    field: &str,
    s: &str,
) -> Result<DateTime<Utc>, ToolError> {
    time.parse(s).map_err(|e| {
        ToolError::invalid_input(e.to_string()).detail(json!({"field": field, "reason": "format"}))
    })
}

/// `everyone` is stored as no tier.
pub(crate) fn tier_of(audience: &str) -> Option<&str> {
    (audience != "everyone").then_some(audience)
}

/// The audience word for a stored tier.
pub(crate) fn audience(tier: Option<&str>) -> &str {
    tier.unwrap_or("everyone")
}

/// The tier, or `tier_not_found`. `everyone` needs no tier.
pub(crate) async fn known_tier(tx: &mut TxCx, audience: &str) -> Result<Option<Tier>, ToolError> {
    let Some(id) = tier_of(audience) else {
        return Ok(None);
    };
    customers::tier(tx, id).await?.map(Some).ok_or_else(|| {
        ToolError::new(TIER_NOT_FOUND, format!("The store has no tier `{id}`."))
            .detail(json!({"entity": "tier", "id": id}))
            .next("list_tiers")
    })
}

/// `not_found` with the entity and id in `detail`.
pub(crate) fn not_found(entity: &str, id: &str, next: &'static str) -> ToolError {
    ToolError::new(ErrorCode::NOT_FOUND, format!("There is no {entity} {id}."))
        .detail(json!({"entity": entity, "id": id}))
        .next(next)
}

/// An id input that must have this prefix, or be a `$item[n].slot` placeholder when
/// `placeholder_ok`. Returns whether it is a placeholder.
pub(crate) fn check_id(
    field: &str,
    prefix: &str,
    s: &str,
    placeholder_ok: bool,
) -> Result<bool, ToolError> {
    if placeholder_ok && acom_changeset::bind::placeholder(s).is_some() {
        return Ok(true);
    }
    let ok = s
        .strip_prefix(prefix)
        .and_then(|r| r.strip_prefix('_'))
        .is_some_and(|r| {
            !r.is_empty()
                && r.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        });
    if ok {
        Ok(false)
    } else {
        Err(
            ToolError::invalid_input(format!("`{field}` must look like `{prefix}_…`; got `{s}`."))
                .detail(json!({"field": field, "reason": "format"})),
        )
    }
}

/// A money amount as a JSON number with two decimals.
pub(crate) fn amount(d: Decimal) -> Value {
    let d = model::cents(d);
    serde_json::from_str(&d.to_string()).unwrap_or(Value::Null)
}

/// `{"value": 12.40, "currency": "USD"}`.
pub(crate) fn money(d: Decimal, currency: &str) -> Value {
    json!({"value": amount(d), "currency": currency})
}

/// A percentage as a JSON number without trailing zeros (`20`, `12.5`).
pub(crate) fn percent(d: Decimal) -> Value {
    serde_json::from_str(&model::percent_text(d)).unwrap_or(Value::Null)
}

/// Keeps only the asked fields of an entity (`fields` in a `get_*` input); `id` always stays.
pub(crate) fn pick(v: Value, fields: Option<&[String]>) -> Value {
    match (v, fields) {
        (Value::Object(o), Some(f)) if !f.is_empty() => Value::Object(
            o.into_iter()
                .filter(|(k, _)| k == "id" || f.iter().any(|x| x == k))
                .collect(),
        ),
        (v, _) => v,
    }
}

/// Time in store time, or `null`.
pub(crate) fn time_or_null(time: &StoreTime, t: Option<DateTime<Utc>>) -> Value {
    t.map_or(Value::Null, |t| json!(time.format(t)))
}

/// The customer a shopper may look at: `me` is the signed-in shopper; a shopper may not look at
/// someone else. Operators may look at anyone.
pub(crate) fn whose(cx: &Cx, customer_id: &str) -> Result<String, ToolError> {
    let me = cx.principal.customer_id();
    let shopper = cx.principal.kind == acom_core::enums::ActorKind::Shopper || me.is_some();
    let refuse = |message: &str| {
        ToolError::new(ErrorCode::FORBIDDEN, message.to_owned()).detail(json!({"reason": "owner"}))
    };
    if customer_id == "me" {
        return me
            .map(str::to_owned)
            .ok_or_else(|| refuse("Sign in to see your own coupons and store points."));
    }
    check_id("customer_id", "cus", customer_id, false)?;
    if shopper && me != Some(customer_id) {
        return Err(refuse(
            "A shopper can only see their own coupons and store points.",
        ));
    }
    Ok(customer_id.to_owned())
}

/// The member, or `not_found` (entity `customer`).
pub(crate) async fn known_member(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<customers::Member, ToolError> {
    customers::members(tx, &[customer_id.to_owned()])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| not_found("customer", customer_id, "search_members"))
}
