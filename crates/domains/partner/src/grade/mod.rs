//! `partner.grade`: how well a seller serves shoppers. The server computes the metrics
//! (shipping compliance, order fulfillment, claim rate, service score) from the seller's orders,
//! parcels, claims and product questions, and the grade they give (`grade.levels`); a person
//! confirms the grade for a period or adjusts it with a reason.

pub mod get_seller_grade;
pub mod set_seller_grade;

pub use get_seller_grade::GetSellerGrade;
pub use set_seller_grade::SetSellerGrade;

use chrono::NaiveDate;
use serde_json::{Value, json};

use crate::model::{self, Metrics, rate_value};
use crate::rules::Rules;

/// The rates of a period as the tools return them; a metric the policy does not grade with
/// (`grade.computed_metrics`) is `null`.
pub(crate) fn rates_wire(m: &Metrics, rules: &Rules) -> Value {
    let pick = |metric: &str, v| {
        if rules.grades_with(metric) {
            rate_value(v)
        } else {
            Value::Null
        }
    };
    json!({
        "acom:shipping_compliance_rate": pick("shipping_compliance", m.shipping_compliance_rate),
        "acom:order_fulfillment_rate": pick("order_fulfillment", m.order_fulfillment_rate),
        "acom:claim_rate": pick("claim_rate", m.claim_rate),
        "acom:service_score": pick("service_score", m.service_score),
    })
}

/// The counts behind the rates.
pub(crate) fn basis_wire(m: &Metrics) -> Value {
    json!({
        "order_count": m.order_count,
        "order_lines": m.lines,
        "shipped_on_time": m.on_time,
        "late_shipments": m.late_shipments,
        "unfulfilled_lines": m.unfulfilled_lines,
        "claimed_lines": m.claimed_lines,
        "questions_asked": m.questions_asked,
        "questions_answered": m.questions_answered,
    })
}

/// The month a grade starting on `day` is reviewed on: the calendar month before.
pub(crate) fn review_period(day: NaiveDate) -> (NaiveDate, NaiveDate) {
    model::last_month(day)
}
