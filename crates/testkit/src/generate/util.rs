//! Helpers shared by the generator: the seeded dice, money and store time.

use std::str::FromStr;

use chrono::{DateTime, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, TimeDelta, TimeZone};
use chrono_tz::Tz;
use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::{Number, Value};

/// The generator's one random stream. Only `next_u64` of ChaCha8 is used and every draw is
/// integer arithmetic, so the rows do not depend on the platform or on `rand`'s samplers.
#[derive(Debug)]
pub(crate) struct Dice(ChaCha8Rng);

impl Dice {
    pub(crate) fn new(seed: u64) -> Self {
        Self(ChaCha8Rng::seed_from_u64(seed))
    }

    /// Uniform in `0..n` (0 when `n <= 1`, without a draw).
    pub(crate) fn below(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        let n = n as u64;
        let limit = u64::MAX - u64::MAX % n;
        loop {
            let x = self.0.next_u64();
            if x < limit {
                return (x % n) as usize;
            }
        }
    }

    /// Uniform in `lo..=hi`.
    pub(crate) fn range(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        lo + self.below((hi - lo + 1) as usize) as i64
    }

    /// One element of a non-empty slice.
    pub(crate) fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// An index drawn with the given integer weights.
    pub(crate) fn weighted(&mut self, weights: &[u64]) -> usize {
        let total: u64 = weights.iter().sum();
        let mut x = self.below(total as usize) as u64;
        for (i, w) in weights.iter().enumerate() {
            if x < *w {
                return i;
            }
            x -= w;
        }
        weights.len().saturating_sub(1)
    }

    /// Fisher-Yates.
    pub(crate) fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

/// `n / d` rounded half up, for `n >= 0` and `d > 0`.
pub(crate) fn round_div(n: i64, d: i64) -> i64 {
    (2 * n + d) / (2 * d)
}

/// A decimal number with exactly `places` digits after the point (`money(8900)` is `89.00`).
pub(crate) fn decimal(units: i64, places: u32) -> Value {
    let scale = 10_i64.pow(places);
    let sign = if units < 0 { "-" } else { "" };
    let a = units.unsigned_abs();
    let s = scale.unsigned_abs();
    let text = format!("{sign}{}.{:0width$}", a / s, a % s, width = places as usize);
    // The text is always a valid JSON number; Null only if serde_json ever disagrees.
    Number::from_str(&text)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// Money in cents as a JSON number with two decimals.
pub(crate) fn money(cents: i64) -> Value {
    decimal(cents, 2)
}

/// Cents of a JSON number (`89.0` or `89.00` both give 8900).
pub(crate) fn cents(v: &Value) -> Option<i64> {
    v.as_f64().map(|x| (x * 100.0).round() as i64)
}

/// Basis points of a decimal fraction (`0.45` gives 4500).
pub(crate) fn basis_points(x: f64) -> i64 {
    (x * 10_000.0).round() as i64
}

/// Store time for a wall-clock time: the first of two in the autumn overlap, and an hour later
/// for a time that does not exist in the spring gap.
pub(crate) fn local(tz: Tz, at: NaiveDateTime) -> DateTime<Tz> {
    match tz.from_local_datetime(&at) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => t,
        LocalResult::None => local(tz, at + TimeDelta::hours(1)),
    }
}

/// Store time at `h:m` on `date`.
pub(crate) fn at(tz: Tz, date: NaiveDate, h: u32, m: u32) -> DateTime<Tz> {
    let t = date
        .and_hms_opt(h.min(23), m.min(59), 0)
        .unwrap_or_else(|| date.and_time(chrono::NaiveTime::MIN));
    local(tz, t)
}

/// RFC 3339 with the store offset, the way the named files write times.
pub(crate) fn rfc3339(t: &DateTime<Tz>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

/// A time from the spec or a named row: RFC 3339 with an offset, or a store wall-clock time
/// (`2026-10-01T00:00`).
pub(crate) fn parse_time(tz: Tz, text: &str) -> Result<DateTime<Tz>, String> {
    if let Ok(t) = DateTime::<FixedOffset>::parse_from_rfc3339(text) {
        return Ok(t.with_timezone(&tz));
    }
    for format in ["%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(t) = NaiveDateTime::parse_from_str(text, format) {
            return Ok(local(tz, t));
        }
    }
    Err(format!("`{text}` is not a time"))
}

pub(crate) fn parse_date(text: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| format!("`{text}` is not a date"))
}

/// `Sand` -> `sand`, `Heather grey` -> `heather_grey` (ids) or `heather-grey` (SKUs).
pub(crate) fn slug(text: &str, sep: char) -> String {
    text.to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(&sep.to_string())
}
