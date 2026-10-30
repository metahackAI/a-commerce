//! Time: one clock for the whole back end, and store-local date-times.
//!
//! Instants on the wire are RFC 3339 with an offset. Inputs may give a local date-time
//! without an offset ("2026-11-07T00:00"); it is read in the store's time zone. Code never
//! calls `Utc::now()` directly: it asks the `Clock`, so the demo store can run on a fixed day.

use std::sync::{Arc, RwLock};

use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock that only moves when told to. Used by the demo store and by tests.
#[derive(Debug, Clone)]
pub struct FixedClock(Arc<RwLock<DateTime<Utc>>>);

impl FixedClock {
    pub fn at(t: DateTime<Utc>) -> Self {
        Self(Arc::new(RwLock::new(t)))
    }

    pub fn set(&self, t: DateTime<Utc>) {
        if let Ok(mut guard) = self.0.write() {
            *guard = t;
        }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
            .read()
            .map(|t| *t)
            .unwrap_or_else(|poisoned| *poisoned.into_inner())
    }
}

/// Reads and writes date-times in one store's time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreTime {
    pub zone: Tz,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    #[error("`{0}` is not a date-time (expected 2026-11-07T00:00 or RFC 3339)")]
    Unreadable(String),
    #[error("`{0}` does not exist in the store's time zone (clock change)")]
    Skipped(String),
}

impl StoreTime {
    pub fn new(zone: Tz) -> Self {
        Self { zone }
    }

    /// Accepts RFC 3339, a local date-time, or a date (start of day), all in store time.
    pub fn parse(&self, s: &str) -> Result<DateTime<Utc>, TimeError> {
        if let Ok(t) = DateTime::parse_from_rfc3339(s) {
            return Ok(t.with_timezone(&Utc));
        }
        let naive = ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"]
            .iter()
            .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
            .or_else(|| {
                NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
            })
            .ok_or_else(|| TimeError::Unreadable(s.to_owned()))?;
        match self.zone.from_local_datetime(&naive) {
            LocalResult::Single(t) => Ok(t.with_timezone(&Utc)),
            // Ambiguous (clocks go back): take the earlier instant.
            LocalResult::Ambiguous(first, _) => Ok(first.with_timezone(&Utc)),
            LocalResult::None => Err(TimeError::Skipped(s.to_owned())),
        }
    }

    /// RFC 3339 with the store's offset, as every output carries it.
    pub fn format(&self, t: DateTime<Utc>) -> String {
        t.with_timezone(&self.zone).to_rfc3339()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_local_times_in_store_zone() {
        let st = StoreTime::new(chrono_tz::America::New_York);
        let t = st.parse("2026-11-07T00:00").unwrap();
        assert_eq!(st.format(t), "2026-11-07T00:00:00-05:00");
        assert_eq!(st.parse("2026-11-07").unwrap(), t);
        assert_eq!(st.parse("2026-11-07T05:00:00Z").unwrap(), t);
    }

    #[test]
    fn fixed_clock_moves_only_when_set() {
        let start = StoreTime::new(chrono_tz::UTC)
            .parse("2026-11-06T09:00")
            .unwrap();
        let clock = FixedClock::at(start);
        assert_eq!(clock.now(), start);
        let later = start + chrono::Duration::days(1);
        clock.set(later);
        assert_eq!(clock.now(), later);
    }
}
