//! The `partner` domain: sellers that list in the store, their contracts and grades.
//! Specified in `docs/api/partner.md`; rules in `policies/partner.yml`.
//!
//! Layout (docs/backend/README.md §2.2): one folder per resource of the spec (`seller`,
//! `contract`, `grade`, `dashboard`), one file per operation named after its tool. Domain-wide
//! code: `model` (pure business logic: masking, periods, seller metrics, grades, the settlement
//! estimate), `rules` (the partner policy values), `repo` (SQL on the `partner` schema),
//! `viewer` (store operator or a seller's own staff), `activity` (a seller's orders, parcels,
//! claims and questions, read through the services of catalog, sales and fulfillment, never
//! their tables) and `jobs` (status changes booked for later).

pub mod model;
pub mod rules;

mod activity;
mod jobs;
mod repo;
mod viewer;

mod contract;
mod dashboard;
mod grade;
mod seller;

#[cfg(test)]
mod tests;

use acom_contract::{ErrorClass, ErrorCode, RegistryBuilder, ToolError};
use acom_store::{Cx, StoreError};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

/// No seller with that id in this store.
pub const SELLER_NOT_FOUND: ErrorCode = ErrorCode::new("seller_not_found", ErrorClass::NotFound);
/// The seller has no contract (or no version with that number).
pub const CONTRACT_NOT_FOUND: ErrorCode =
    ErrorCode::new("contract_not_found", ErrorClass::NotFound);
/// A search filter that cannot be used.
pub const INVALID_FILTER: ErrorCode = ErrorCode::new("invalid_filter", ErrorClass::InvalidInput);
/// Another seller already uses this tax id (`tax_id_unique`).
pub const DUPLICATE_TAX_ID: ErrorCode =
    ErrorCode::new("duplicate_tax_id", ErrorClass::AlreadyExists);
/// A tax id that is not a business registration number in form.
pub const INVALID_TAX_ID: ErrorCode = ErrorCode::new("invalid_tax_id", ErrorClass::InvalidInput);
/// The application names a category the store does not take sellers for.
pub const PROHIBITED_CATEGORY: ErrorCode =
    ErrorCode::new("prohibited_category", ErrorClass::PolicyViolation);
/// The application lacks a document the store requires.
pub const MISSING_DOCUMENT: ErrorCode =
    ErrorCode::new("missing_document", ErrorClass::PolicyViolation);
/// A profile field the store's policy does not let `update_seller` change.
pub const IMMUTABLE_FIELD: ErrorCode =
    ErrorCode::new("immutable_field", ErrorClass::PolicyViolation);
/// The seller still has orders in progress.
pub const OPEN_ORDERS_EXIST: ErrorCode =
    ErrorCode::new("open_orders_exist", ErrorClass::InvalidState);
/// The seller still has money to be paid out.
pub const PENDING_SETTLEMENT: ErrorCode =
    ErrorCode::new("pending_settlement", ErrorClass::InvalidState);
/// A commission rate outside the store's bounds (`commission`).
pub const COMMISSION_OUT_OF_BOUNDS: ErrorCode =
    ErrorCode::new("commission_out_of_bounds", ErrorClass::PolicyViolation);
/// A contract period that overlaps another version (`contract.no_period_overlap`).
pub const CONTRACT_OVERLAP: ErrorCode =
    ErrorCode::new("contract_overlap", ErrorClass::PolicyViolation);
/// Contracts are set only for approved or active sellers.
pub const SELLER_NOT_ACTIVE: ErrorCode =
    ErrorCode::new("seller_not_active", ErrorClass::InvalidState);
/// The period has no orders to compute a grade from.
pub const NO_DATA_FOR_PERIOD: ErrorCode = ErrorCode::new("no_data_for_period", ErrorClass::Domain);
/// A grade that differs from the computed one without the reason the policy wants.
pub const OVERRIDE_NOT_ALLOWED: ErrorCode =
    ErrorCode::new("override_not_allowed", ErrorClass::PolicyViolation);
/// A grade level the store does not grade with.
pub const INVALID_LEVEL: ErrorCode = ErrorCode::new("invalid_level", ErrorClass::InvalidInput);
/// A termination cannot be taken back.
pub const NOT_REVOCABLE: ErrorCode = ErrorCode::new("not_revocable", ErrorClass::UndoWindowClosed);

/// Adds every operation of this domain to the registry, resource by resource as the spec
/// lists them, and the job that runs status changes booked for later.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    // seller
    b.read(seller::SearchSellers)
        .read(seller::GetSeller)
        .write(seller::OnboardSeller)
        .write(seller::UpdateSeller)
        .write(seller::SetSellerStatus);
    // contract
    b.read(contract::GetContract).write(contract::SetContract);
    // grade
    b.read(grade::GetSellerGrade).write(grade::SetSellerGrade);
    // dashboard
    b.read(dashboard::GetSellerOverview);
    b.job(jobs::ChangeSellerStatus);
}

pub(crate) fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

pub(crate) fn store(e: StoreError) -> ToolError {
    ToolError::from(e)
}

/// `seller_not_found`, pointing at the search.
pub(crate) fn seller_not_found(id: &str) -> ToolError {
    ToolError::new(SELLER_NOT_FOUND, format!("No seller {id} in this store."))
        .detail(json!({"entity": "seller", "id": id}))
        .next("search_sellers")
}

/// Keeps only the asked top-level fields (and `id`, `seller_id`); all of them when none.
pub(crate) fn select(v: Value, fields: &[String]) -> Value {
    if fields.is_empty() {
        return v;
    }
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .filter(|(k, _)| k == "id" || k == "seller_id" || fields.iter().any(|f| f == k))
                .collect(),
        ),
        other => other,
    }
}

/// A time input in store time (`2026-11-07T00:00`, a date, or RFC 3339).
pub(crate) fn parse_time(cx: &Cx, field: &str, s: &str) -> Result<DateTime<Utc>, ToolError> {
    cx.time.parse(s).map_err(|e| {
        ToolError::invalid_input(e.to_string()).detail(json!({"field": field, "reason": "format"}))
    })
}

/// A required sentence (a reason) that is not blank.
pub(crate) fn sentence(field: &str, s: &str, what: &str) -> Result<String, ToolError> {
    let t = s.trim();
    if t.is_empty() {
        return Err(ToolError::invalid_input(format!("Say {what}."))
            .detail(json!({"field": field, "reason": "missing"})));
    }
    Ok(t.to_owned())
}
