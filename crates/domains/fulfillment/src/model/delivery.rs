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

/// When a parcel sent now ships and arrives at an address in the default zone.
pub fn estimate(rules: &Rules, now: DateTime<Utc>, zone: Tz) -> (NaiveDate, NaiveDate) {
    let local = now.with_timezone(&zone).naive_local();
    let ship = ships_on(local, rules.ship_cutoff, &rules.ship_days);
    let arrive = add_business_days(ship, rules.transit_business_days, &rules.business_days);
    (ship, arrive)
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
    fn pickup_is_the_next_business_morning() {
        let (start, end) = pickup_window(&rules(), at("2026-11-06T09:50"), NY);
        let st = StoreTime::new(NY);
        assert_eq!(st.format(start), "2026-11-09T09:00:00-05:00");
        assert_eq!(st.format(end), "2026-11-09T12:00:00-05:00");
    }
}
