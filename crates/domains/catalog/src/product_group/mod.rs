//! The `product_group` resource (docs/api/catalog.md#product_group): what its operations share. A
//! new product starts as a draft; its content changes without approval, and a structural change
//! (category, option axes) on a product shoppers can see waits for a person.

pub mod create_product_group;
pub mod get_product_group;
pub mod search_product_groups;
pub mod update_product_group;

use acom_contract::{Check, ErrorClass, ErrorCode, ToolError};
use acom_store::TxCx;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::neighbors::{self, CategoryTerm};
use crate::repo::{self};
use crate::write::{self, DraftItem};
use crate::{BRAND_NOT_FOUND, slug};

/// A required field is missing from the input.
pub const MISSING_REQUIRED_FIELD: ErrorCode =
    ErrorCode::new("missing_required_field", ErrorClass::InvalidInput);
/// The category is not a standard category of this store (or is inactive).
pub const INVALID_CATEGORY: ErrorCode =
    ErrorCode::new("invalid_category", ErrorClass::InvalidInput);

/// Option axes a product can vary by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptionAxis {
    /// Variants differ in color.
    Color,
    /// Variants differ in size.
    Size,
}

impl OptionAxis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Color => "color",
            Self::Size => "size",
        }
    }
}

/// The standard category with this id, or `invalid_category`.
pub(crate) async fn standard_category(
    tx: &mut TxCx,
    category_id: &str,
) -> Result<CategoryTerm, ToolError> {
    let terms = neighbors::category_terms(tx).await?;
    match terms.into_iter().find(|t| t.id == category_id) {
        Some(t) if t.term_set == "standard" && t.active => Ok(t),
        Some(t) => Err(ToolError::new(
            INVALID_CATEGORY,
            if t.term_set == "standard" {
                format!("{category_id} is inactive; products go in active standard categories.")
            } else {
                format!("{category_id} is a display category; products go in a standard category, and display categories show them through map_category.")
            },
        )
        .detail(json!({"field": "category_id", "reason": "enum", "term_set": t.term_set, "active": t.active}))
        .next("list_categories")),
        None => Err(ToolError::new(
            INVALID_CATEGORY,
            format!("There is no category {category_id} in this store."),
        )
        .detail(json!({"field": "category_id", "reason": "enum"}))
        .next("list_categories")),
    }
}

/// The product's primary category now.
pub(crate) async fn primary_category(
    tx: &mut TxCx,
    product_id: &str,
) -> Result<Option<String>, ToolError> {
    Ok(neighbors::categories(tx, Some(&[product_id.to_owned()]))
        .await?
        .into_iter()
        .filter(|c| c.direct)
        .max_by_key(|c| c.is_primary)
        .map(|c| c.category_id))
}

async fn known_brand(tx: &mut TxCx, brand_id: &str) -> Result<(), ToolError> {
    match repo::brand(tx, brand_id).await? {
        Some(_) => Ok(()),
        None => Err(
            ToolError::new(BRAND_NOT_FOUND, format!("There is no brand {brand_id}."))
                .detail(json!({"entity": "brand", "id": brand_id}))
                .next("list_brands"),
        ),
    }
}

/// An id for a new product from its name: `prd_wool_blend_overcoat`, then `_2`, `_3` while
/// taken.
pub(crate) async fn free_product_id(tx: &mut TxCx, base: &str) -> Result<String, ToolError> {
    let base = format!("prd_{}", slug(base));
    let mut candidates = vec![base.clone()];
    candidates.extend((2..50).map(|n| format!("{base}_{n}")));
    let taken = write::existing_groups(tx, &candidates).await?;
    candidates
        .into_iter()
        .find(|c| !taken.contains_key(c))
        .ok_or_else(|| ToolError::new(ErrorCode::INTERNAL, "no free product id"))
}

fn max_axes_check(p: &acom_policy::PolicySet, axes: usize) -> Option<Check> {
    let max = p
        .lookup("catalog.variant.max_option_axes")
        .and_then(Value::as_u64)?;
    let ok = (axes as u64) <= max;
    Some(Check {
        rule: "variant.max_option_axes".into(),
        passed: ok,
        detail: Some(format!("{axes} option axes; the store allows {max}")),
        fix_tool: (!ok).then(|| "update_product_group".into()),
    })
}

/// The first image URL of an `image` list.
fn first_image(images: Option<&Vec<String>>) -> Option<String> {
    images
        .and_then(|l| l.iter().map(|s| s.trim()).find(|s| !s.is_empty()))
        .map(str::to_owned)
}

fn trimmed(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// What `create_product_group` runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateGroupPlan {
    pub product_id: String,
    pub name: String,
    pub description: Option<String>,
    pub brand_id: Option<String>,
    pub category_id: String,
    pub image: Option<String>,
    pub varies_by: Vec<String>,
    pub material: Option<String>,
}

/// Where a draft came from and what is still open on it (`acom:draft`), for operators.
pub(crate) fn draft_json(i: &DraftItem) -> Value {
    json!({"import_id": i.product_import_id, "row": i.position, "supplier_code": i.supplier_code,
           "draft_status": i.draft_status, "open_fields": i.open_fields, "note": i.draft_note,
           "supplier_text": i.supplier_text})
}
