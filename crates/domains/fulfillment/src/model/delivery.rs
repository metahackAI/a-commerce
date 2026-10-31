//! Dates: when a parcel ships and arrives (`delivery.estimate`, the shipping zone's transit
//! days), when a carrier comes for a return (`return.pickup`), and how long ago something was
//! delivered. Everything is in store-local calendar days.

use chrono::{
    DateTime, Datelike, Days, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc, Weekday,
};
use chrono_tz::Tz;

use super::Rules;

/// The store-local date of an instant.
pub fn local_date(t: DateTime<Utc>, zone: Tz) -> NaiveDate {
    t.with_timezone(&zone).date_naive()
}

/// Whole calendar days from delivery to now, in store time (delivered yesterday = 1).
pub fn days_since(delivered_at: DateTime<Utc>, now: DateTime<Utc>, zone: Tz) -> i64 {
    (local_date(now, zone) - local_date(delivered_at, zone)).num_days()
}

fn next_day(d: NaiveDate) -> NaiveDate {
    d.checked_add_days(Days::new(1)).unwrap_or(d)
}

/// The first ship day: today when it is a ship day and before the cutoff, else the next ship day.
pub fn ships_on(now_local: NaiveDateTime, cutoff: NaiveTime, ship_days: &[Weekday]) -> NaiveDate {
    let today = now_local.date();
    if ship_days.contains(&today.weekday()) && now_local.time() < cutoff {
        return today;
    }
    let mut d = next_day(today);
    for _ in 0..7 {
        if ship_days.contains(&d.weekday()) {
            return d;
        }
        d = next_day(d);
    }
    d
}

/// `n` business days after `from` (the day itself does not count).
pub fn add_business_days(from: NaiveDate, n: i64, business_days: &[Weekday]) -> NaiveDate {
    if business_days.is_empty() {
        return from;
    }
    let mut d = from;
    let mut counted = 0;
    while counted < n {
        d = next_day(d);
        if business_days.contains(&d.weekday()) {
            counted += 1;
        }
    }
    d
}

/// Whether a tracking number has the carrier's format (`delivery.tracking.number_formats`):
/// `None` when the store keeps no format for the carrier (or does not validate), else the
/// verdict and a sentence for the check.
pub fn tracking_format(rules: &Rules, carrier: &str, number: &str) -> Option<(bool, String)> {
    let validate = rules
        .lookup("delivery.tracking.validate_number_format")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !validate {
        return None;
    }
    let formats = rules
        .lookup("delivery.tracking.number_formats")?
        .as_object()?;
    let (name, f) = formats
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(carrier))?;
    let prefix = f.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
    let min = f.get("min_length").and_then(|v| v.as_u64()).unwrap_or(1);
    let max = f.get("max_length").and_then(|v| v.as_u64()).unwrap_or(64);
    let digits = f.get("characters").and_then(|v| v.as_str()) == Some("digits");
    let len = number.chars().count() as u64;
    let chars_ok = number.chars().all(|c| {
        if digits {
            c.is_ascii_digit()
        } else {
            c.is_ascii_alphanumeric()
        }
    });
    let upper = number.to_ascii_uppercase();
    let ok =
        upper.starts_with(&prefix.to_ascii_uppercase()) && (min..=max).contains(&len) && chars_ok;
    Some((
        ok,
        if ok {
            format!("Tracking number matches the {name} format")
        } else {
            let what = if digits {
                "digits"
            } else {
                "letters and digits"
            };
            if prefix.is_empty() {
                format!("{name} tracking numbers are {min}–{max} {what}")
            } else {
                format!("{name} tracking numbers start with {prefix} and are {min}–{max} {what}")
            }
        },
    ))
}

/// When a parcel sent now ships and arrives at an address in the default zone.
pub fn estimate(rules: &Rules, now: DateTime<Utc>, zone: Tz) -> (NaiveDate, NaiveDate) {
    estimate_in(rules, now, zone, rules.transit_business_days)
}

/// When a parcel sent now ships and arrives, `transit_days` business days after it ships.
pub fn estimate_in(
    rules: &Rules,
    now: DateTime<Utc>,
    zone: Tz,
    transit_days: i64,
) -> (NaiveDate, NaiveDate) {
    let local = now.with_timezone(&zone).naive_local();
    let ship = ships_on(local, rules.ship_cutoff, &rules.ship_days);
    let arrive = add_business_days(ship, transit_days, &rules.business_days);
    (ship, arrive)
}

/// An exchange asked for now: the pickup day, the day the replacement ships and the day it
/// arrives. The returned item travels `transit_days` back to the warehouse; with
/// `exchange.reship_after: inspected` it is checked on arrival and the replacement ships the
/// next ship day, with `picked_up` it ships on arrival.
pub fn exchange_dates(
    rules: &Rules,
    now: DateTime<Utc>,
    zone: Tz,
    transit_days: i64,
    reship_after: &str,
) -> (NaiveDate, NaiveDate, NaiveDate) {
    let pickup = add_business_days(
        local_date(now, zone),
        rules.pickup_days_after,
        &rules.business_days,
    );
    let back = add_business_days(pickup, transit_days, &rules.business_days);
    let from = if reship_after == "inspected" {
        next_day(back)
    } else {
        back
    };
    let ship = ships_on(
        from.and_time(NaiveTime::MIN),
        rules.ship_cutoff,
        &rules.ship_days,
    );
    let arrive = add_business_days(ship, transit_days, &rules.business_days);
    (pickup, ship, arrive)
}

/// The carrier's pickup window for a return asked for now, as instants.
pub fn pickup_window(
    rules: &Rules,
    now: DateTime<Utc>,
    zone: Tz,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let day = add_business_days(
        local_date(now, zone),
        rules.pickup_days_after,
        &rules.business_days,
    );
    let at = |t: NaiveTime| {
        zone.from_local_datetime(&day.and_time(t))
            .earliest()
            .map(|x| x.with_timezone(&Utc))
            .unwrap_or_else(|| day.and_time(t).and_utc())
    };
    (at(rules.pickup_start), at(rules.pickup_end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::rules;
    use acom_core::StoreTime;

    const NY: Tz = chrono_tz::America::New_York;

    fn at(s: &str) -> DateTime<Utc> {
        StoreTime::new(NY).parse(s).unwrap()
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn counts_days_in_store_time_across_the_clock_change() {
        let now = at("2026-11-06T10:40");
        assert_eq!(days_since(at("2026-10-31T14:02:00-04:00"), now, NY), 6);
        assert_eq!(days_since(at("2026-11-04T14:30:00-05:00"), now, NY), 2);
        assert_eq!(days_since(at("2026-09-28T16:00:00-04:00"), now, NY), 39);
        assert_eq!(days_since(at("2026-11-02T15:20:00-05:00"), now, NY), 4);
        // Late evening in New York is already the next day in UTC.
        assert_eq!(
            days_since(at("2026-11-05T22:00"), at("2026-11-06T00:30"), NY),
            1
        );
    }

    #[test]
    fn saturday_morning_order_arrives_tuesday() {
        // Scene F2/F4: Sat Nov 7 10:00, Brooklyn.
        let (ship, arrive) = estimate(&rules(), at("2026-11-07T10:00"), NY);
        assert_eq!(ship, date("2026-11-07"));
        assert_eq!(arrive, date("2026-11-10"));
    }

    #[test]
    fn friday_replacement_ships_today_and_arrives_tuesday() {
        // Scene F5: Fri Nov 6 09:50.
        let (ship, arrive) = estimate(&rules(), at("2026-11-06T09:50"), NY);
        assert_eq!(ship, date("2026-11-06"));
        assert_eq!(arrive, date("2026-11-10"));
    }

    #[test]
    fn after_the_cutoff_ships_next_ship_day() {
        let (ship, arrive) = estimate(&rules(), at("2026-11-07T15:00"), NY);
        assert_eq!(ship, date("2026-11-09"));
        assert_eq!(arrive, date("2026-11-11"));
    }

    #[test]
    fn tracking_numbers_follow_the_carrier_format() {
        let r = rules();
        let (ok, detail) = tracking_format(&r, "UPS", "1ZNW204550001").unwrap();
        assert!(ok, "{detail}");
        assert_eq!(detail, "Tracking number matches the UPS format");
        let (ok, detail) = tracking_format(&r, "ups", "9400111").unwrap();
        assert!(!ok);
        assert_eq!(
            detail,
            "UPS tracking numbers start with 1Z and are 10–35 letters and digits"
        );
        assert_eq!(tracking_format(&r, "Northwind Courier", "A1"), None);
    }

    #[test]
    fn an_exchange_reships_the_day_after_the_return_is_checked() {
        // Picked up Mon Nov 9, back Wed Nov 11, checked, ships Thu Nov 12, arrives Mon Nov 16.
        let r = rules();
        let (pickup, ship, arrive) = exchange_dates(&r, at("2026-11-06T10:00"), NY, 2, "inspected");
        assert_eq!(pickup, date("2026-11-09"));
        assert_eq!(ship, date("2026-11-12"));
        assert_eq!(arrive, date("2026-11-16"));
        let (_, ship, _) = exchange_dates(&r, at("2026-11-06T10:00"), NY, 2, "picked_up");
        assert_eq!(ship, date("2026-11-11"));
    }

    #[test]
    fn pickup_is_the_next_business_morning() {
        let (start, end) = pickup_window(&rules(), at("2026-11-06T09:50"), NY);
        let st = StoreTime::new(NY);
        assert_eq!(st.format(start), "2026-11-09T09:00:00-05:00");
        assert_eq!(st.format(end), "2026-11-09T12:00:00-05:00");
    }
}
