//! Actionable errors: a code, a class, a sentence a person can read, and the next tool to try.
//!
//! The catalog is `docs/api/errors.md`. The class decides the HTTP status; the code is as
//! specific as an agent needs to branch on. Domain crates declare their own codes as
//! `ErrorCode` constants next to their operations.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The small closed set every error belongs to (docs/api/errors.md §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    Unauthenticated,
    Forbidden,
    NotFound,
    InvalidInput,
    AlreadyExists,
    InvalidState,
    LimitExceeded,
    RateLimited,
    Locked,
    PolicyViolation,
    PreviewExpired,
    ApprovalRequired,
    VersionOutOfDate,
    UndoWindowClosed,
    EstimateChanged,
    IdempotencyConflict,
    Domain,
    UpstreamUnavailable,
    Internal,
}

impl ErrorClass {
    pub fn http_status(self) -> u16 {
        use ErrorClass::*;
        match self {
            Unauthenticated => 401,
            Forbidden => 403,
            NotFound => 404,
            PreviewExpired => 410,
            Locked => 423,
            RateLimited => 429,
            AlreadyExists | InvalidState | ApprovalRequired | VersionOutOfDate
            | UndoWindowClosed | EstimateChanged | IdempotencyConflict => 409,
            InvalidInput | LimitExceeded | PolicyViolation | Domain => 422,
            UpstreamUnavailable => 503,
            Internal => 500,
        }
    }
}

/// A lowercase snake_case error code and the class it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct ErrorCode {
    pub code: &'static str,
    pub class: ErrorClass,
}

impl ErrorCode {
    pub const fn new(code: &'static str, class: ErrorClass) -> Self {
        Self { code, class }
    }

    pub const UNAUTHENTICATED: Self = Self::new("unauthenticated", ErrorClass::Unauthenticated);
    pub const FORBIDDEN: Self = Self::new("forbidden", ErrorClass::Forbidden);
    pub const NOT_FOUND: Self = Self::new("not_found", ErrorClass::NotFound);
    pub const INVALID_INPUT: Self = Self::new("invalid_input", ErrorClass::InvalidInput);
    pub const PREVIEW_INPUT_CONFLICT: Self =
        Self::new("preview_input_conflict", ErrorClass::InvalidInput);
    pub const INVALID_STATE: Self = Self::new("invalid_state", ErrorClass::InvalidState);
    pub const RATE_LIMITED: Self = Self::new("rate_limited", ErrorClass::RateLimited);
    pub const PREVIEW_EXPIRED: Self = Self::new("preview_expired", ErrorClass::PreviewExpired);
    pub const PREVIEW_NOT_FOUND: Self = Self::new("preview_not_found", ErrorClass::PreviewExpired);
    pub const PREVIEW_SUPERSEDED: Self =
        Self::new("preview_superseded", ErrorClass::PreviewExpired);
    pub const APPROVAL_REQUIRED: Self =
        Self::new("approval_required", ErrorClass::ApprovalRequired);
    pub const VERSION_OUT_OF_DATE: Self =
        Self::new("version_out_of_date", ErrorClass::VersionOutOfDate);
    pub const UNDO_WINDOW_CLOSED: Self =
        Self::new("undo_window_closed", ErrorClass::UndoWindowClosed);
    pub const ESTIMATE_CHANGED: Self = Self::new("estimate_changed", ErrorClass::EstimateChanged);
    pub const POLICY_CHANGED: Self = Self::new("policy_changed", ErrorClass::EstimateChanged);
    pub const IDEMPOTENCY_CONFLICT: Self =
        Self::new("idempotency_conflict", ErrorClass::IdempotencyConflict);
    pub const UPSTREAM_UNAVAILABLE: Self =
        Self::new("upstream_unavailable", ErrorClass::UpstreamUnavailable);
    pub const INTERNAL: Self = Self::new("internal", ErrorClass::Internal);
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code)
    }
}

/// The error every operation returns. Serialized inside `{"error": ...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ToolError {
    pub code: Cow<'static, str>,
    pub class: ErrorClass,
    pub message: String,
    /// A tool name, or `None`. Never prose: hints go in `message` or `detail`.
    pub next_tool: Option<Cow<'static, str>>,
    /// Structured context, for example the policy rule and the products that failed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Box<Value>>,
}

impl ToolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code: Cow::Borrowed(code.code),
            class: code.class,
            message: message.into(),
            next_tool: None,
            detail: None,
        }
    }

    pub fn next(mut self, tool: &'static str) -> Self {
        self.next_tool = Some(Cow::Borrowed(tool));
        self
    }

    pub fn detail(mut self, detail: Value) -> Self {
        self.detail = Some(Box::new(detail));
        self
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::INVALID_INPUT, message)
    }

    pub fn http_status(&self) -> u16 {
        self.class.http_status()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BELOW_PRICE_FLOOR: ErrorCode =
        ErrorCode::new("below_price_floor", ErrorClass::PolicyViolation);

    #[test]
    fn serializes_code_and_class_without_empty_detail() {
        let e = ToolError::new(ErrorCode::PREVIEW_EXPIRED, "The preview expired.")
            .next("set_sale_prices");
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"code":"preview_expired","class":"preview_expired","message":"The preview expired.","next_tool":"set_sale_prices"}"#
        );
        assert_eq!(e.http_status(), 410);
    }

    #[test]
    fn status_comes_from_the_class() {
        let e = ToolError::new(BELOW_PRICE_FLOOR, "Below cost + 15%.")
            .detail(serde_json::json!({"rule": "price_floor"}));
        assert_eq!(e.http_status(), 422);
        assert_eq!(
            ToolError::new(ErrorCode::PREVIEW_NOT_FOUND, "x").http_status(),
            410
        );
        assert_eq!(
            ToolError::new(ErrorCode::VERSION_OUT_OF_DATE, "x").http_status(),
            409
        );
    }
}
