//! The `brand` resource (docs/api/catalog.md#brand): what its operations share.

pub mod create_brand;
pub mod get_brand;
pub mod list_brands;
pub mod update_brand;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use serde_json::{Value, json};

use crate::repo::Brand;

fn brand_json(b: &Brand) -> Value {
    json!({"id": b.id, "@type": "Brand", "name": b.name, "logo": b.logo,
           "description": b.description})
}

/// Another brand already has this name.
pub const DUPLICATE_BRAND: ErrorCode = ErrorCode::new("duplicate_brand", ErrorClass::AlreadyExists);

fn duplicate(name: &str, existing: &str) -> ToolError {
    ToolError::new(
        DUPLICATE_BRAND,
        format!("The brand {name} already exists as {existing}."),
    )
    .detail(json!({"entity": "brand", "field": "name", "existing_id": existing}))
    .next("list_brands")
}

fn logo_of(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}
