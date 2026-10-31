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

// ── tier evaluation (preview_tier_evaluation, run_tier_evaluation, define_tier_program) ──

/// One tier as an evaluation sees it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TierDef {
    pub id: String,
    pub rank: i32,
    pub requirements: Vec<Value>,
    /// `any` (the default) or `all`.
    pub any: bool,
    pub benefits: Value,
}

impl TierDef {
    pub(crate) fn from_stored(
        id: &str,
        rank: i32,
        has_tier_requirement: &Value,
        benefits: &Value,
    ) -> Self {
        Self {
            id: id.to_owned(),
            rank,
            requirements: requirement_list(has_tier_requirement),
            any: requirement_combine(has_tier_requirement).as_deref() != Some("all"),
            benefits: benefits.clone(),
        }
    }
}

/// A member as an evaluation sees them.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Member {
    pub customer_id: String,
    pub name: String,
    pub tier_id: String,
    /// A manual tier that still holds at the evaluation time.
    pub override_active: bool,
    /// The last tier change was `grace_started`.
    pub in_grace: bool,
}

/// How members move (`tier.movement`, the program's or the policy's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Movement {
    pub upgrade: bool,
    pub downgrade: bool,
    pub max_steps_down: i32,
    pub grace_periods: i32,
}

impl Movement {
    pub(crate) fn from_values(program: &Value, policy: &Value) -> Self {
        let get = |k: &str| program.get(k).or_else(|| policy.get(k));
        Self {
            upgrade: get("upgrade").and_then(Value::as_str) != Some("never"),
            downgrade: get("downgrade").and_then(Value::as_str) != Some("never"),
            max_steps_down: get("max_steps_down")
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
                .unwrap_or(i32::MAX),
            grace_periods: get("grace_periods")
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
                .unwrap_or(0),
        }
    }
}

/// What the store measures results with (`tier.requirement`, `tier.evaluation`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Measure {
    pub default_window: Value,
    pub default_channels: Vec<String>,
    pub settle_days: i64,
    pub as_of: DateTime<Utc>,
    pub time: StoreTime,
}

/// `up`, `down`, `grace` (kept for now, with a warning), `same`, or `override` (a manual tier
/// the evaluation leaves alone).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Direction {
    Up,
    Down,
    Grace,
    Same,
    Override,
}

impl Direction {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Grace => "grace",
            Self::Same => "same",
            Self::Override => "override",
        }
    }
}

/// What an evaluation decides for one member.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Move {
    pub customer_id: String,
    pub from_tier_id: String,
    pub to_tier_id: String,
    pub direction: Direction,
    pub reason: String,
    /// The result it was based on: `{metric, value}`.
    pub snapshot: Option<Value>,
}

/// One requirement measured: `Some(true|false)`, or `None` when it cannot be measured from
/// orders (`invite`, `manual`, `points_earned`, `custom`). `keep` uses `retain_threshold`.
fn requirement_met(
    req: &Value,
    orders: &[MemberOrder],
    m: &Measure,
    keep: bool,
) -> (Option<bool>, Option<Value>) {
    let channels: Vec<String> = req
        .pointer("/counts/channels")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_else(|| m.default_channels.clone());
    let start = window_start(
        req.get("window").unwrap_or(&m.default_window),
        m.as_of,
        m.time,
    );
    match req.get("metric").and_then(Value::as_str) {
        Some("spend") => {
            let threshold = if keep {
                threshold_money(req, "retain_threshold")
                    .or_else(|| threshold_money(req, "threshold"))
            } else {
                threshold_money(req, "threshold")
            };
            let Some(t) = threshold else {
                return (None, None);
            };
            let tally = tally(orders, t.currency, &channels, start, m.as_of, m.settle_days);
            let value = cents(tally.settled, t.currency);
            (
                Some(tally.settled >= t.value),
                Some(json!({"metric": "spend", "value": value,
                            "threshold": cents(t.value, t.currency)})),
            )
        }
        Some("order_count") => {
            let key = if keep && req.get("retain_threshold").is_some() {
                "retain_threshold"
            } else {
                "threshold"
            };
            let Some(t) = req.get(key).and_then(Value::as_i64) else {
                return (None, None);
            };
            // Orders in every currency count: an order is an order.
            let count = orders
                .iter()
                .filter(|o| {
                    o.delivered_at.is_some_and(|d| {
                        d >= start
                            && d + Duration::days(m.settle_days) <= m.as_of
                            && (channels.is_empty() || channels.contains(&o.sales_channel))
                    })
                })
                .count();
            let count = i64::try_from(count).unwrap_or(i64::MAX);
            (
                Some(count >= t),
                Some(json!({"metric": "order_count", "value": count, "threshold": t})),
            )
        }
        _ => (None, None),
    }
}

/// Whether a member meets a tier: `Some(true|false)`, or `None` when it depends on something
/// orders cannot show (an invite, a manual grant).
fn tier_met(
    t: &TierDef,
    orders: &[MemberOrder],
    m: &Measure,
    keep: bool,
) -> (Option<bool>, Option<Value>) {
    if t.requirements.is_empty() {
        return (Some(true), None);
    }
    let results: Vec<(Option<bool>, Option<Value>)> = t
        .requirements
        .iter()
        .map(|r| requirement_met(r, orders, m, keep))
        .collect();
    let snapshot = results.iter().find_map(|(_, s)| s.clone());
    let met = if t.any {
        if results.iter().any(|(r, _)| *r == Some(true)) {
            Some(true)
        } else if results.iter().any(|(r, _)| r.is_none()) {
            None
        } else {
            Some(false)
        }
    } else if results.iter().any(|(r, _)| *r == Some(false)) {
        Some(false)
    } else if results.iter().any(|(r, _)| r.is_none()) {
        None
    } else {
        Some(true)
    };
    (met, snapshot)
}

fn describe(snapshot: &Option<Value>, op: &str) -> String {
    match snapshot {
        Some(s) if s["metric"] == "spend" => format!(
            "spend {} {op} {} {}",
            s["value"]["value"],
            s["threshold"]["value"],
            s["threshold"]["currency"].as_str().unwrap_or_default()
        ),
        Some(s) => format!("{} orders {op} {}", s["value"], s["threshold"]),
        None => "no measurable requirement".to_owned(),
    }
}

/// Decides one member's tier. Tiers come highest rank first or in any order; the member's
/// current tier must be one of them. `orders` is `None` when orders are not connected: nothing
/// is measured and nobody moves.
pub(crate) fn evaluate(
    tiers: &[TierDef],
    member: &Member,
    orders: Option<&[MemberOrder]>,
    m: &Measure,
    movement: &Movement,
) -> Move {
    let stay = |direction: Direction, reason: String, snapshot: Option<Value>| Move {
        customer_id: member.customer_id.clone(),
        from_tier_id: member.tier_id.clone(),
        to_tier_id: member.tier_id.clone(),
        direction,
        reason,
        snapshot,
    };
    if member.override_active {
        return stay(
            Direction::Override,
            "Set by an operator; evaluation leaves it until it expires".into(),
            None,
        );
    }
    let Some(orders) = orders else {
        return stay(
            Direction::Same,
            "Orders are not connected; nothing measured".into(),
            None,
        );
    };
    let mut sorted: Vec<&TierDef> = tiers.iter().collect();
    sorted.sort_by_key(|t| std::cmp::Reverse(t.rank));
    let Some(current) = sorted.iter().find(|t| t.id == member.tier_id).copied() else {
        return stay(
            Direction::Same,
            "The current tier is not in the program".into(),
            None,
        );
    };
    // The highest tier the results reach.
    let best = sorted
        .iter()
        .find_map(|t| match tier_met(t, orders, m, false) {
            (Some(true), s) => Some((*t, s)),
            _ => None,
        });
    if let Some((b, s)) = &best
        && b.rank > current.rank
        && movement.upgrade
    {
        return Move {
            customer_id: member.customer_id.clone(),
            from_tier_id: current.id.clone(),
            to_tier_id: b.id.clone(),
            direction: Direction::Up,
            reason: describe(s, ">="),
            snapshot: s.clone(),
        };
    }
    let (keep, snapshot) = tier_met(current, orders, m, true);
    if keep != Some(false) || !movement.downgrade {
        let why = match keep {
            None => "Reached another way (invite or manual); kept".to_owned(),
            _ if current.requirements.is_empty() => "Entry tier".to_owned(),
            _ => format!("Keeps {}: {}", current.id, describe(&snapshot, ">=")),
        };
        return stay(Direction::Same, why, snapshot);
    }
    if movement.grace_periods > 0 && !member.in_grace {
        return stay(
            Direction::Grace,
            format!("{}; grace period starts", describe(&snapshot, "<")),
            snapshot,
        );
    }
    let floor = current.rank.saturating_sub(movement.max_steps_down.max(1));
    let target_rank = best.as_ref().map_or(i32::MIN, |(b, _)| b.rank).max(floor);
    let target = sorted
        .iter()
        .filter(|t| t.rank <= target_rank)
        .max_by_key(|t| t.rank)
        .copied()
        .unwrap_or(current);
    if target.id == current.id {
        return stay(Direction::Same, describe(&snapshot, "<"), snapshot);
    }
    Move {
        customer_id: member.customer_id.clone(),
        from_tier_id: current.id.clone(),
        to_tier_id: target.id.clone(),
        direction: Direction::Down,
        reason: describe(&snapshot, "<"),
        snapshot,
    }
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

    fn program() -> Vec<TierDef> {
        vec![
            TierDef::from_stored(
                "member",
                1,
                &json!({"combine": "any", "requirements": []}),
                &json!([]),
            ),
            TierDef::from_stored(
                "gold",
                2,
                &json!({"combine": "any", "requirements": [gold()]}),
                &json!([]),
            ),
            TierDef::from_stored(
                "vip",
                3,
                &json!({"combine": "any", "requirements": [
                    {"metric": "spend", "threshold": {"value": 800.00, "currency": "USD"},
                     "retain_threshold": {"value": 600.00, "currency": "USD"}},
                    {"metric": "invite"}]}),
                &json!([]),
            ),
        ]
    }

    fn measure() -> Measure {
        Measure {
            default_window: json!({"type": "rolling", "length": 6, "unit": "month"}),
            default_channels: vec!["web".into(), "app".into()],
            settle_days: 14,
            as_of: at("2026-12-01T03:00"),
            time: time(),
        }
    }

    fn member(tier: &str) -> Member {
        Member {
            customer_id: "cus_alex".into(),
            name: "Alex".into(),
            tier_id: tier.into(),
            override_active: false,
            in_grace: false,
        }
    }

    #[test]
    fn evaluates_up_down_grace_and_overrides() {
        let movement = Movement::from_values(
            &json!({"upgrade": "immediate", "downgrade": "at_schedule", "max_steps_down": 1,
                    "grace_periods": 1}),
            &json!({}),
        );
        // Alex's 312.00 settles by Dec 1: Member to Gold.
        let up = evaluate(
            &program(),
            &member("member"),
            Some(&alex_orders()),
            &measure(),
            &movement,
        );
        assert_eq!(
            (up.direction, up.to_tier_id.as_str()),
            (Direction::Up, "gold")
        );
        assert_eq!(up.reason, "spend 312.00 >= 300.00 USD");
        // A Gold member with nothing: first a grace period, then one step down.
        let grace = evaluate(
            &program(),
            &member("gold"),
            Some(&[]),
            &measure(),
            &movement,
        );
        assert_eq!(grace.direction, Direction::Grace);
        let in_grace = Member {
            in_grace: true,
            ..member("gold")
        };
        let down = evaluate(&program(), &in_grace, Some(&[]), &measure(), &movement);
        assert_eq!(
            (down.direction, down.to_tier_id.as_str()),
            (Direction::Down, "member")
        );
        // VIP can also come by invite, which orders cannot show: kept.
        let vip = evaluate(&program(), &member("vip"), Some(&[]), &measure(), &movement);
        assert_eq!(vip.direction, Direction::Same);
        // A manual tier is left alone; without orders nobody moves.
        let manual = Member {
            override_active: true,
            ..member("member")
        };
        let kept = evaluate(
            &program(),
            &manual,
            Some(&alex_orders()),
            &measure(),
            &movement,
        );
        assert_eq!(kept.direction, Direction::Override);
        let none = evaluate(&program(), &member("member"), None, &measure(), &movement);
        assert_eq!(none.direction, Direction::Same);
    }
}
