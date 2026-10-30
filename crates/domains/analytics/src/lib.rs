//! The `analytics` domain: read-only aggregates of sales and customers, and the explanation of
//! a sales change (scene B3). Specified in `docs/api/analytics.md`.
//!
//! Layout: `period` (store days and the days they are compared with) and `decompose` (the
//! arithmetic of an explanation) are pure; `repo` reads this domain's own read model, the
//! `analytics` views; `sales` and `customer` hold one type per operation. Everything is a read:
//! no operation has side effects, and results are aggregates, never personal data.
//!
//! Analytics depends on no other domain's tables. Its views (migrations 0011 and 0016) are the
//! read model over the other domains; counts that a view could only give by naming people (how
//! many members bought) come from `acom_sales::service::for_analytics`.

pub mod decompose;
pub mod period;

mod customer;
mod repo;
mod sales;
#[cfg(test)]
mod tests;

use acom_contract::{ErrorClass, ErrorCode, RegistryBuilder, ToolError};
use acom_core::Money;
use acom_store::Cx;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde_json::{Value, json};

pub use customer::GetCustomerOverview;
pub use sales::{ExplainSalesChange, GetSalesOverview, GetSalesTrend};

pub const INVALID_PERIOD: ErrorCode = ErrorCode::new("invalid_period", ErrorClass::InvalidInput);
pub const NO_DATA_FOR_PERIOD: ErrorCode = ErrorCode::new("no_data_for_period", ErrorClass::Domain);
pub const INVALID_GRANULARITY: ErrorCode =
    ErrorCode::new("invalid_granularity", ErrorClass::InvalidInput);
pub const DIMENSION_NOT_SUPPORTED: ErrorCode =
    ErrorCode::new("dimension_not_supported", ErrorClass::InvalidInput);

/// Adds every operation of this domain to the registry.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    b.read(sales::ExplainSalesChange)
        .read(sales::GetSalesOverview)
        .read(sales::GetSalesTrend)
        .read(customer::GetCustomerOverview);
}

/// Today in the store's time zone, on the store clock.
pub(crate) fn today(cx: &Cx) -> NaiveDate {
    cx.clock.now().with_timezone(&cx.time.zone).date_naive()
}

/// A period from the input, or `invalid_period` naming the field.
pub(crate) fn span(
    cx: &Cx,
    from: NaiveDate,
    through: NaiveDate,
    next: &'static str,
) -> Result<period::Span, ToolError> {
    period::Span::new(from, through, today(cx)).map_err(|p| period_error(p, next))
}

pub(crate) fn period_error(p: period::PeriodProblem, next: &'static str) -> ToolError {
    ToolError::new(INVALID_PERIOD, p.message)
        .next(next)
        .detail(json!({"field": p.field, "reason": "range"}))
}

/// An amount in the store's currency, written with its minor units (`0.00`, not `0`).
pub(crate) fn money(cx: &Cx, value: Decimal) -> Money {
    let mut v = value;
    v.rescale(cx.currency.minor_units());
    Money::new(v, cx.currency)
}

pub(crate) fn money_json(cx: &Cx, value: Decimal) -> Value {
    serde_json::to_value(money(cx, value)).unwrap_or(Value::Null)
}

/// An exact decimal as a JSON number (`247`, `0.2035`), trailing zeros dropped.
pub(crate) fn number(value: Decimal) -> Value {
    serde_json::from_str(&value.normalize().to_string()).unwrap_or(Value::Null)
}

/// `part / whole` as a 0–1 rate to four places, or null when `whole` is zero.
pub(crate) fn rate(part: Decimal, whole: Decimal) -> Value {
    if whole.is_zero() {
        Value::Null
    } else {
        number(decompose::round(part / whole, decompose::RATE_PLACES))
    }
}
