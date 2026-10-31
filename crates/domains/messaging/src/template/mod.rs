//! The `template` resource (docs/api/messaging.md#template): `create_message_template` and
//! `list_message_templates`, reusable messages per channel. Creating one sends nothing, so it
//! needs no approval.

use acom_contract::{ErrorClass, ErrorCode};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod create_message_template;
mod list_message_templates;

pub use create_message_template::*;
pub use list_message_templates::*;

acom_core::define_id!(
    /// A message template (`messaging.message_template`).
    TemplateId,
    "tpl"
);

pub const UNKNOWN_VARIABLE: ErrorCode =
    ErrorCode::new("unknown_variable", ErrorClass::InvalidInput);
pub const CHANNEL_NOT_SUPPORTED: ErrorCode =
    ErrorCode::new("channel_not_supported", ErrorClass::InvalidInput);
pub const INVALID_FILTER: ErrorCode = ErrorCode::new("invalid_filter", ErrorClass::InvalidInput);

/// What a template is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessagePurpose {
    /// Promotions: only to shoppers who opted in, held back in quiet hours.
    Marketing,
    /// About an order or a claim.
    Transactional,
}

impl MessagePurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Marketing => "marketing",
            Self::Transactional => "transactional",
        }
    }
}
