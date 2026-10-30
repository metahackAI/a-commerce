//! Delivery facts shown with a cart and an order: when a parcel arrives, how an address reads,
//! and whether the store ships there.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::rules::ShipCalendar;

/// A postal address as orders record it (`schema:PostalAddress`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Address {
    pub name: Option<String>,
    pub street_address: String,
    pub address_locality: String,
    pub address_region: Option<String>,
    pub postal_code: String,
    pub address_country: String,
}

/// When an order placed at `now` (store time) arrives: it leaves the same day before the
/// cutoff on a shipping day, else on the next shipping day, then travels `transit_days`
/// further shipping days.
pub(crate) fn arrival(now: NaiveDateTime, cal: &ShipCalendar) -> NaiveDate {
    let ships = |d: NaiveDate| cal.ship_days.contains(&d.weekday());
    let today = now.date();
    let mut day = if ships(today) && now.time() < cal.cutoff {
        today
    } else {
        next_ship_day(today, &ships)
    };
    for _ in 0..cal.transit_days {
        day = next_ship_day(day, &ships);
    }
    day
}

fn next_ship_day(from: NaiveDate, ships: &impl Fn(NaiveDate) -> bool) -> NaiveDate {
    let mut d = from + Duration::days(1);
    // A calendar has at least one shipping day, so a week always finds one.
    for _ in 0..7 {
        if ships(d) {
            return d;
        }
        d += Duration::days(1);
    }
    d
}

/// "Home · 12 Harbor St, Apt 3": what a cart shows.
pub(crate) fn short_label(label: Option<&str>, street: &str) -> String {
    match label.filter(|l| !l.trim().is_empty()) {
        Some(l) => format!("{l} · {street}"),
        None => street.to_owned(),
    }
}

/// "Home · 12 Harbor St, Apt 3, Brooklyn, NY 11201": what the checkout card shows.
/// A gift's address may come without region or postal code ("Dad · 48 Pine Rd, Lakeview").
pub(crate) fn full_label(label: Option<&str>, a: &Address) -> String {
    let region = a.address_region.as_deref().map(str::trim).unwrap_or("");
    let postal = a.postal_code.trim();
    let tail = [region, postal]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let place = if tail.is_empty() {
        a.address_locality.clone()
    } else if region.is_empty() {
        format!("{} {tail}", a.address_locality)
    } else {
        format!("{}, {tail}", a.address_locality)
    };
    short_label(label, &format!("{}, {place}", a.street_address))
}

/// Whether the store ships to this country (`order.shipping.ships_to`).
pub(crate) fn deliverable(country: &str, ships_to: &[String]) -> bool {
    ships_to
        .iter()
        .any(|c| c.eq_ignore_ascii_case(country.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveTime, Weekday};

    fn calendar() -> ShipCalendar {
        ShipCalendar {
            cutoff: NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
            ship_days: vec![
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
                Weekday::Sat,
            ],
            transit_days: 2,
        }
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").unwrap()
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// F2: Jamie's cart on Sat Nov 7, 10:00 arrives Tue Nov 10.
    #[test]
    fn saturday_morning_arrives_tuesday() {
        assert_eq!(
            arrival(at("2026-11-07T10:00"), &calendar()),
            day("2026-11-10")
        );
    }

    #[test]
    fn after_the_cutoff_or_on_sunday_it_leaves_next_shipping_day() {
        assert_eq!(
            arrival(at("2026-11-07T15:00"), &calendar()),
            day("2026-11-11")
        );
        assert_eq!(
            arrival(at("2026-11-08T09:00"), &calendar()),
            day("2026-11-11")
        );
        assert_eq!(
            arrival(at("2026-11-06T09:50"), &calendar()),
            day("2026-11-09")
        );
    }

    #[test]
    fn labels_read_like_the_cards() {
        let home = Address {
            name: Some("Jamie".into()),
            street_address: "12 Harbor St, Apt 3".into(),
            address_locality: "Brooklyn".into(),
            address_region: Some("NY".into()),
            postal_code: "11201".into(),
            address_country: "US".into(),
        };
        assert_eq!(
            short_label(Some("Home"), &home.street_address),
            "Home · 12 Harbor St, Apt 3"
        );
        assert_eq!(
            full_label(Some("Home"), &home),
            "Home · 12 Harbor St, Apt 3, Brooklyn, NY 11201"
        );
        assert_eq!(
            full_label(None, &home),
            "12 Harbor St, Apt 3, Brooklyn, NY 11201"
        );
        let dad = Address {
            name: Some("Dad".into()),
            street_address: "48 Pine Rd".into(),
            address_locality: "Lakeview".into(),
            address_region: None,
            postal_code: String::new(),
            address_country: "US".into(),
        };
        assert_eq!(full_label(Some("Dad"), &dad), "Dad · 48 Pine Rd, Lakeview");
        assert!(deliverable("us", &["US".to_owned()]));
        assert!(!deliverable("CA", &["US".to_owned()]));
    }
}
