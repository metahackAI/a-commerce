//! The `product_import` resource (docs/api/integration.md `## product_import`): bulk imports of
//! products, prices, stock and availability from an outside feed (a seller's, or the store's
//! own), with a result per row. New products become catalog drafts through catalog's own draft
//! path; products already in the catalog change through catalog's rules.

pub(crate) mod get_import_status;
pub(crate) mod import_products;

pub use get_import_status::GetImportStatus;
pub use import_products::ImportProducts;

use acom_contract::ErrorCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Why one row was not taken: a code from errors.md, a sentence, and the tool that fixes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowError {
    pub code: String,
    pub message: String,
    pub next_tool: Option<String>,
}

impl RowError {
    pub fn new(code: ErrorCode, message: impl Into<String>, next_tool: Option<&str>) -> Self {
        Self {
            code: code.code.to_owned(),
            message: message.into(),
            next_tool: next_tool.map(str::to_owned),
        }
    }

    pub fn json(&self) -> Value {
        json!({"code": self.code, "message": self.message, "next_tool": self.next_tool})
    }
}

/// What the import does with one row of the feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowResult {
    /// Row number in the feed, from 1.
    pub position: i32,
    pub external_id: String,
    /// The product the row creates or changes; `None` when it is rejected.
    pub product_id: Option<String>,
    /// The feed's variant codes and the variants they are.
    pub variants: Vec<(String, String)>,
    pub error: Option<RowError>,
}

impl RowResult {
    pub fn accepted(&self) -> bool {
        self.error.is_none()
    }

    /// `[{external_id, variant_id}]`, as the row's record keeps it.
    pub fn variant_mapping(&self) -> Value {
        Value::Array(
            self.variants
                .iter()
                .map(|(e, v)| json!({"external_id": e, "variant_id": v}))
                .collect(),
        )
    }
}
