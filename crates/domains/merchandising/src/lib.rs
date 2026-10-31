//! The `merchandising` domain: categories, display slots, banners, popups, curated pages.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/merchandising.md`.
//! Layout: the folders follow the API hierarchy `merchandising.{resource}.{operation}`: one
//! folder per resource (`category`, `display_slot`, `banner`, `popup`, `collection_page`,
//! `cache`) and one file
//! per operation, named after its tool (`banner/schedule_banner.rs`); what a resource's
//! operations share is in its `mod.rs`. Domain-wide code: `model` (rules as pure functions and
//! the policy values they use) and `repo` (SQL). Other domains are reached only through the
//! service functions they export, never through their tables.

pub mod service;

mod model;
mod repo;

mod banner;
mod cache;
mod category;
mod collection_page;
mod display_slot;
mod popup;

#[cfg(test)]
mod b1;
#[cfg(test)]
mod contract;
#[cfg(test)]
mod integration;
#[cfg(test)]
mod integration_writes;
#[cfg(test)]
mod testing;

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
    // category
    b.read(category::list_categories::ListCategories)
        .read(category::get_category::GetCategory)
        .write(category::create_category::CreateCategory)
        .write(category::update_category::UpdateCategory)
        .write(category::map_category::MapCategory)
        .write(category::unmap_category::UnmapCategory);
    // display_slot
    b.read(display_slot::list_display_slots::ListDisplaySlots)
        .read(display_slot::get_display_slot::GetDisplaySlot)
        .write(display_slot::arrange_display_slot::ArrangeDisplaySlot)
        .write(display_slot::schedule_display::ScheduleDisplay);
    // banner
    b.write(banner::schedule_banner::ScheduleBanner)
        .read(banner::list_banners::ListBanners)
        .write(banner::set_banner::SetBanner);
    // popup
    b.read(popup::list_popups::ListPopups)
        .write(popup::set_popup::SetPopup);
    // collection_page
    b.read(collection_page::list_curations::ListCurations)
        .read(collection_page::get_curation::GetCuration)
        .write(collection_page::create_curation::CreateCuration)
        .write(collection_page::update_curation::UpdateCuration)
        .write(collection_page::publish_curation::PublishCuration);
    // cache
    b.write(cache::invalidate_display_cache::InvalidateDisplayCache);
}

pub const TIER_NOT_FOUND: ErrorCode = ErrorCode::new("tier_not_found", ErrorClass::NotFound);
pub const INVALID_PERIOD: ErrorCode = ErrorCode::new("invalid_period", ErrorClass::InvalidInput);

/// Whether `approval.yml` asks for a person on this write (`thresholds.*` whose `applies_to`
/// names the tool and whose `when` holds for `facts`). Writes with no threshold run without one.
pub(crate) fn policy_approval(cx: &Cx, tool: &str, facts: &Value) -> acom_contract::ApprovalNeed {
    let a = cx.policy.approval_for(tool, facts);
    match a.reason {
        Some(reason) if a.needed => acom_contract::ApprovalNeed::Needed { reason },
        _ if a.needed => acom_contract::ApprovalNeed::Needed {
            reason: format!("The store asks a person to approve {tool}"),
        },
        _ => acom_contract::ApprovalNeed::NotNeeded,
    }
}

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
