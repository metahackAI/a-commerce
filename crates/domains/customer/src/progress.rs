//! Pure tier rules for `get_member_tier`: which orders count towards a requirement, how far a
//! member is from the next tier, whether they keep the current one, and when the next
//! evaluation runs. Tier definitions are data (`has_tier_requirement`); defaults come from
//! `policies/customer.yml` `tier`.

use acom_core::{Currency, Money, StoreTime};
use chrono::{DateTime, Datelike, Duration, Months, NaiveDate, Utc, Weekday};
use rust_decimal::Decimal;
use serde_json::{Value, json};

use crate::service::providers::MemberOrder;

/// The requirements of a tier, whether stored as `{combine, requirements}` or as a bare list.
pub(crate) fn requirement_list(has_tier_requirement: &Value) -> Vec<Value> {
    match has_tier_requirement {
        Value::Array(l) => l.clone(),
        Value::Object(o) => o
            .get("requirements")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// `any` or `all`, when the tier says.
pub(crate) fn requirement_combine(has_tier_requirement: &Value) -> Option<String> {
    has_tier_requirement
        .get("combine")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// An amount with exactly the currency's minor digits (`0` → `0.00`).
pub(crate) fn cents(value: Decimal, currency: Currency) -> Money {
    let mut v = Money::new(value, currency).rounded().value;
    v.rescale(currency.minor_units());
    Money::new(v, currency)
}

/// Where a requirement's window starts: rolling (the last `length` units), calendar (the start
/// of the current year or month), or lifetime.
pub(crate) fn window_start(window: &Value, now: DateTime<Utc>, time: StoreTime) -> DateTime<Utc> {
    let length = window.get("length").and_then(Value::as_u64).unwrap_or(0);
    let length32 = u32::try_from(length).unwrap_or(0);
    let unit = window
        .get("unit")
        .and_then(Value::as_str)
        .unwrap_or("month");
    match window
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("rolling")
    {
        "lifetime" => DateTime::<Utc>::UNIX_EPOCH,
        "calendar" => {
            let local = now.with_timezone(&time.zone).date_naive();
            let first = match unit {
                "year" => NaiveDate::from_ymd_opt(local.year(), 1, 1),
                _ => NaiveDate::from_ymd_opt(local.year(), local.month(), 1)
                    .and_then(|d| d.checked_sub_months(Months::new(length32.saturating_sub(1)))),
            };
            first
                .and_then(|d| time.parse(&d.format("%Y-%m-%d").to_string()).ok())
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        }
        _ => {
            // Months and years go back on the store's wall clock (Nov 6 09:50 → May 6 09:50).
            let months_back = |months: u32| {
                let local = now.with_timezone(&time.zone).naive_local();
                let earlier = local.checked_sub_months(Months::new(months))?;
                time.parse(&earlier.format("%Y-%m-%dT%H:%M:%S").to_string())
                    .ok()
            };
            let back = match unit {
                "day" => now.checked_sub_signed(Duration::days(length as i64)),
                "week" => now.checked_sub_signed(Duration::weeks(length as i64)),
                "year" => months_back(length32.saturating_mul(12)),
                _ => months_back(length32),
            };
            back.unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        }
    }
}

/// What a member's orders add up to inside a window.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tally {
    /// Delivered long enough ago that a return can no longer take it back.
    pub settled: Decimal,
    pub settled_orders: i64,
    /// Delivered, but still inside the settle days.
    pub pending: Decimal,
    /// `(order number, when it settles)` of the pending orders.
    pub pending_orders: Vec<(String, DateTime<Utc>)>,
}

/// Counts delivered orders in `currency` from the given channels (all when empty) that were
/// delivered at or after `start`.
pub(crate) fn tally(
    orders: &[MemberOrder],
    currency: Currency,
    channels: &[String],
    start: DateTime<Utc>,
    now: DateTime<Utc>,
    settle_days: i64,
) -> Tally {
    let mut t = Tally {
        settled: Decimal::ZERO,
        settled_orders: 0,
        pending: Decimal::ZERO,
        pending_orders: Vec::new(),
    };
    for o in orders {
        let Some(delivered) = o.delivered_at else {
            continue;
        };
        let counted_channel = channels.is_empty() || channels.contains(&o.sales_channel);
        if delivered < start || o.amount.currency != currency || !counted_channel {
            continue;
        }
        let settles = delivered + Duration::days(settle_days);
        if settles <= now {
            t.settled += o.amount.value;
            t.settled_orders += 1;
        } else {
            t.pending += o.amount.value;
            t.pending_orders.push((o.order_number.clone(), settles));
        }
    }
    t.pending_orders.sort_by_key(|(_, at)| *at);
    t
}

/// `Order 20402 counts after Nov 14 unless it is returned`.
pub(crate) fn pending_detail(pending: &[(String, DateTime<Utc>)], time: StoreTime) -> String {
    let day = |t: DateTime<Utc>| t.with_timezone(&time.zone).format("%b %-d").to_string();
    match pending {
        [] => String::new(),
        [(number, at)] => format!(
            "Order {number} counts after {} unless it is returned",
            day(*at)
        ),
        many => format!(
            "{} orders count by {} unless they are returned",
            many.len(),
            many.last().map(|(_, at)| day(*at)).unwrap_or_default()
        ),
    }
}

/// The money threshold of a requirement (`threshold` or `retain_threshold`).
pub(crate) fn threshold_money(req: &Value, key: &str) -> Option<Money> {
    serde_json::from_value(req.get(key)?.clone()).ok()
}

/// One `progress[]` entry towards `to_tier`, for requirements that can be measured from orders
/// (`spend`, `order_count`). Others (`invite`, `manual`, `custom`, `points_earned`) give `None`.
pub(crate) fn progress_entry(
    to_tier: &str,
    req: &Value,
    tally: &Tally,
    time: StoreTime,
) -> Option<Value> {
    match req.get("metric").and_then(Value::as_str)? {
        "spend" => {
            let threshold = threshold_money(req, "threshold")?;
            let c = threshold.currency;
            let remaining = (threshold.value - tally.settled).max(Decimal::ZERO);
            let pending = (tally.pending > Decimal::ZERO).then(|| {
                let mut p = json!(cents(tally.pending, c));
                p["detail"] = json!(pending_detail(&tally.pending_orders, time));
                p
            });
            Some(json!({"to_tier_id": to_tier, "metric": "spend",
                "current": cents(tally.settled, c), "threshold": cents(threshold.value, c),
                "remaining": cents(remaining, c), "pending": pending}))
        }
        "order_count" => {
            let threshold = req.get("threshold").and_then(Value::as_i64)?;
            let pending = (!tally.pending_orders.is_empty()).then(|| {
                json!({"value": tally.pending_orders.len(),
                       "detail": pending_detail(&tally.pending_orders, time)})
            });
            Some(json!({"to_tier_id": to_tier, "metric": "order_count",
                "current": tally.settled_orders, "threshold": threshold,
                "remaining": (threshold - tally.settled_orders).max(0), "pending": pending}))
        }
        _ => None,
    }
}

/// Whether the member keeps their tier: `on_track` or `at_risk`, with the amount that keeps
/// it. A tier without measurable requirements, or one also reached another way (`any` with an
/// invite or manual requirement), is always on track.
pub(crate) fn retention(
    requirements: &[Value],
    combine_any: bool,
    tally_for: impl Fn(&Value) -> Tally,
) -> (&'static str, Option<Money>) {
    let measurable: Vec<&Value> = requirements
        .iter()
        .filter(|r| r.get("metric").and_then(Value::as_str) == Some("spend"))
        .collect();
    let Some(req) = measurable.first() else {
        return ("on_track", None);
    };
    let keep =
        threshold_money(req, "retain_threshold").or_else(|| threshold_money(req, "threshold"));
    let Some(keep) = keep else {
        return ("on_track", None);
    };
    let other_ways = combine_any && measurable.len() < requirements.len();
    let t = tally_for(req);
    let status = if other_ways || t.settled + t.pending >= keep.value {
        "on_track"
    } else {
        "at_risk"
    };
    (status, Some(cents(keep.value, keep.currency)))
}

/// The next scheduled evaluation after `now`: `{every: month | week | year | day, on_day, at}`
/// in store time.
pub(crate) fn next_evaluation(
    schedule: &Value,
    now: DateTime<Utc>,
    time: StoreTime,
) -> Option<DateTime<Utc>> {
    let every = schedule
        .get("every")
        .and_then(Value::as_str)
        .unwrap_or("month");
    let on_day = schedule.get("on_day").and_then(Value::as_u64).unwrap_or(1);
    let on_day = u32::try_from(on_day).ok()?.max(1);
    let at = schedule
        .get("at")
        .and_then(Value::as_str)
        .unwrap_or("00:00");
    let today = now.with_timezone(&time.zone).date_naive();
    let at_day = |d: NaiveDate| time.parse(&format!("{}T{at}", d.format("%Y-%m-%d"))).ok();
    let candidates: Vec<NaiveDate> = match every {
        "day" => (0..2)
            .filter_map(|n| today.checked_add_days(chrono::Days::new(n)))
            .collect(),
        "week" => {
            let target = Weekday::try_from(u8::try_from(on_day.min(7) - 1).ok()?).ok()?;
            (0..8)
                .filter_map(|n| today.checked_add_days(chrono::Days::new(n)))
                .filter(|d| d.weekday() == target)
                .collect()
        }
        "year" => (0..2)
            .filter_map(|n| NaiveDate::from_ymd_opt(today.year() + n, 1, on_day))
            .collect(),
        _ => (0..2)
            .filter_map(|n| {
                NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?
                    .checked_add_months(Months::new(n))?
                    .with_day(on_day)
            })
            .collect(),
    };
    candidates.into_iter().filter_map(at_day).find(|t| *t > now)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn time() -> StoreTime {
        StoreTime::new(chrono_tz::America::New_York)
    }

    fn at(s: &str) -> DateTime<Utc> {
        time().parse(s).unwrap()
    }

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    fn gold() -> Value {
        json!({"metric": "spend", "threshold": {"value": 300.00, "currency": "USD"},
               "window": {"type": "rolling", "length": 6, "unit": "month"},
               "counts": {"channels": ["web", "app"]}})
    }

    fn alex_orders() -> Vec<MemberOrder> {
        vec![MemberOrder {
            order_id: "ord_20402".into(),
            order_number: "20402".into(),
            amount: usd("312.00"),
            sales_channel: "web".into(),
            delivered_at: Some(at("2026-10-31T14:02")),
        }]
    }

    #[test]
    fn alex_is_300_away_from_gold_with_312_pending() {
        let now = at("2026-11-06T09:50");
        let start = window_start(&gold()["window"], now, time());
        assert_eq!(start, at("2026-05-06T09:50"));
        let t = tally(
            &alex_orders(),
            Currency::USD,
            &["web".into(), "app".into()],
            start,
            now,
            14,
        );
        assert_eq!(t.settled, Decimal::ZERO);
        assert_eq!(t.pending, Decimal::from_str("312.00").unwrap());
        let p = progress_entry("gold", &gold(), &t, time()).unwrap();
        assert_eq!(
            p["current"].to_string(),
            r#"{"value":0.00,"currency":"USD"}"#
        );
        assert_eq!(
            p["remaining"].to_string(),
            r#"{"value":300.00,"currency":"USD"}"#
        );
        assert_eq!(p["pending"]["value"].to_string(), "312.00");
        assert_eq!(
            p["pending"]["detail"],
            "Order 20402 counts after Nov 14 unless it is returned"
        );
    }

    #[test]
    fn settled_orders_count_and_channels_filter() {
        let now = at("2026-11-20T09:00");
        let mut orders = alex_orders();
        orders.push(MemberOrder {
            order_id: "ord_1".into(),
            order_number: "1".into(),
            amount: usd("50.00"),
            sales_channel: "agent".into(),
            delivered_at: Some(at("2026-10-01T09:00")),
        });
        let t = tally(
            &orders,
            Currency::USD,
            &["web".into()],
            at("2026-05-20T09:00"),
            now,
            14,
        );
        assert_eq!(t.settled, Decimal::from_str("312.00").unwrap());
        assert_eq!(t.settled_orders, 1);
        let p = progress_entry("gold", &gold(), &t, time()).unwrap();
        assert_eq!(
            p["remaining"].to_string(),
            r#"{"value":0.00,"currency":"USD"}"#
        );
        assert!(p["pending"].is_null());
    }

    #[test]
    fn keeping_a_tier_uses_the_retain_threshold() {
        let vip = vec![
            json!({"metric": "spend", "threshold": {"value": 800.00, "currency": "USD"},
                   "retain_threshold": {"value": 600.00, "currency": "USD"}}),
            json!({"metric": "invite"}),
        ];
        let low = |_: &Value| Tally {
            settled: Decimal::from(262),
            settled_orders: 3,
            pending: Decimal::ZERO,
            pending_orders: vec![],
        };
        assert_eq!(retention(&vip, true, low).0, "on_track");
        let (status, keep) = retention(&vip[..1], true, low);
        assert_eq!(status, "at_risk");
        assert_eq!(keep, Some(usd("600.00")));
        assert_eq!(retention(&[], true, low), ("on_track", None));
    }

    #[test]
    fn next_evaluation_is_the_first_of_next_month_at_three() {
        let schedule = json!({"every": "month", "on_day": 1, "at": "03:00"});
        let next = next_evaluation(&schedule, at("2026-11-06T09:50"), time()).unwrap();
        assert_eq!(time().format(next), "2026-12-01T03:00:00-05:00");
        let early = next_evaluation(&schedule, at("2026-11-01T01:00"), time()).unwrap();
        assert_eq!(time().format(early), "2026-11-01T03:00:00-05:00");
    }

    #[test]
    fn reads_both_requirement_shapes() {
        let wrapped = json!({"combine": "any", "requirements": [gold()]});
        assert_eq!(requirement_list(&wrapped).len(), 1);
        assert_eq!(requirement_combine(&wrapped).as_deref(), Some("any"));
        assert_eq!(requirement_list(&json!([gold()])).len(), 1);
        assert!(requirement_list(&json!({})).is_empty());
    }
}
