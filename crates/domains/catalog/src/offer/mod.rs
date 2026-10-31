//! The `offer` resource (docs/api/catalog.md#offer): list prices and costs of variants.

pub mod get_price_breakdown;
pub mod schedule_price_change;
pub mod set_price;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::write::Schedule;

/// The list price in effect at `at`: the latest scheduled price whose period holds `at`, else
/// the offer's own price.
pub fn price_at(offer_price: Decimal, schedules: &[&Schedule], at: DateTime<Utc>) -> Decimal {
    schedules
        .iter()
        .filter(|s| s.valid_from <= at && s.valid_through.is_none_or(|t| t >= at))
        .max_by_key(|s| s.valid_from)
        .map_or(offer_price, |s| s.price)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::str::FromStr;

    #[test]
    fn a_scheduled_price_applies_only_inside_its_period() {
        let t = |s: &str| {
            acom_core::StoreTime::new(chrono_tz::America::New_York)
                .parse(s)
                .unwrap()
        };
        let s = Schedule {
            id: "ups_x".into(),
            offer_id: "off_x".into(),
            price: Decimal::from_str("95.00").unwrap(),
            price_currency: "USD".into(),
            valid_from: t("2026-12-01T00:00"),
            valid_through: Some(t("2026-12-31T23:59")),
        };
        let base = Decimal::from_str("89.00").unwrap();
        assert_eq!(price_at(base, &[&s], t("2026-11-30T23:59")), base);
        assert_eq!(
            price_at(base, &[&s], t("2026-12-15T12:00")).to_string(),
            "95.00"
        );
        assert_eq!(price_at(base, &[&s], t("2027-01-01T00:00")), base);
    }
}
