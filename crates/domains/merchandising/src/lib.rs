//! The `merchandising` domain: categories, display slots, banners, popups, curated pages.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/merchandising.md`.
//! Layout: `model` (rules as pure functions), one module per resource with one type per
//! operation implementing `ReadOp`/`WriteOp`, `repo` (SQL). Other domains are reached only
//! through the service functions they export, never through their tables.

pub mod service;

mod banner;
mod category;
mod curation;
mod display_slot;
mod model;
mod repo;

use acom_contract::{ErrorClass, ErrorCode, RegistryBuilder, ToolError};
use acom_core::StoreTime;
use acom_customer::service::for_merchandising::{self as customers, Tier};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

/// Adds every operation of this domain to the registry, and lends promotion the category
/// placement it needs (promotion cannot depend on this crate).
pub fn register(b: &mut RegistryBuilder<Cx>) {
    acom_promotion::service::for_merchandising::provide_category_products(
        service::for_promotion::lookup,
    );
    b.read(category::ListCategories)
        .read(category::GetCategory)
        .read(display_slot::ListDisplaySlots)
        .read(display_slot::GetDisplaySlot)
        .write(banner::ScheduleBanner)
        .read(banner::ListBanners)
        .write(banner::SetBanner)
        .read(curation::ListCurations)
        .read(curation::GetCuration);
}

pub const TIER_NOT_FOUND: ErrorCode = ErrorCode::new("tier_not_found", ErrorClass::NotFound);
pub const INVALID_PERIOD: ErrorCode = ErrorCode::new("invalid_period", ErrorClass::InvalidInput);

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

/// The audience word for a stored tier (`everyone` is stored as no tier).
pub(crate) fn audience(tier: Option<&str>) -> &str {
    tier.unwrap_or("everyone")
}

/// The tier, or `tier_not_found`. `everyone` needs no tier.
pub(crate) async fn known_tier(tx: &mut TxCx, audience: &str) -> Result<Option<Tier>, ToolError> {
    if audience == "everyone" {
        return Ok(None);
    }
    customers::tier(tx, audience)
        .await?
        .map(Some)
        .ok_or_else(|| {
            ToolError::new(
                TIER_NOT_FOUND,
                format!("The store has no tier `{audience}`."),
            )
            .detail(json!({"entity": "tier", "id": audience}))
            .next("list_tiers")
        })
}

/// An id input that must have this prefix.
pub(crate) fn check_id(field: &str, prefix: &str, s: &str) -> Result<(), ToolError> {
    let ok = s
        .strip_prefix(prefix)
        .and_then(|r| r.strip_prefix('_'))
        .is_some_and(|r| {
            !r.is_empty()
                && r.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        });
    if ok {
        Ok(())
    } else {
        Err(
            ToolError::invalid_input(format!("`{field}` must look like `{prefix}_…`; got `{s}`."))
                .detail(json!({"field": field, "reason": "format"})),
        )
    }
}

/// Like [`check_id`], but also accepts a `$item[n].slot` placeholder (an id an earlier item of
/// the same change set produces). Returns whether it is a placeholder.
pub(crate) fn check_id_or_placeholder(
    field: &str,
    prefix: &str,
    s: &str,
) -> Result<bool, ToolError> {
    if acom_changeset::bind::placeholder(s).is_some() {
        return Ok(true);
    }
    check_id(field, prefix, s).map(|()| false)
}

/// Keeps only the asked fields (`fields` in an input); `id` always stays.
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
