//! Periods of store days and the days a period is compared with. Pure date arithmetic.

use chrono::{DateTime, Days, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use schemars::JsonSchema;
use serde::Deserialize;

/// Weeks `same_weekday_last_4_weeks` looks back.
pub const WEEKS_BACK: u64 = 4;
/// The longest period, in days, any read takes.
pub const MAX_DAYS: i64 = 366;

/// What a period is compared with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompareMode {
    /// The same weekdays in each of the last 4 weeks, averaged (a period of 7 days at most).
    #[serde(rename = "same_weekday_last_4_weeks")]
    SameWeekdayLast4Weeks,
    /// The days just before the period, as many as it has.
    PreviousPeriod,
    /// `compare_from` to `compare_through`, scaled to the period's length; it must end before
    /// the period starts.
    Custom,
}

impl CompareMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SameWeekdayLast4Weeks => "same_weekday_last_4_weeks",
            Self::PreviousPeriod => "previous_period",
            Self::Custom => "custom",
        }
    }
}

/// A run of store days, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub from: NaiveDate,
    pub through: NaiveDate,
}

/// Why a period cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodProblem {
    pub field: &'static str,
    pub message: String,
}

fn problem(field: &'static str, message: impl Into<String>) -> PeriodProblem {
    PeriodProblem {
        field,
        message: message.into(),
    }
}

impl Span {
    /// A period that starts no later than it ends, is at most [`MAX_DAYS`] long, and does not
    /// end after `today`.
    pub fn new(
        from: NaiveDate,
        through: NaiveDate,
        today: NaiveDate,
    ) -> Result<Self, PeriodProblem> {
        if through < from {
            return Err(problem(
                "valid_through",
                format!("The period ends ({through}) before it starts ({from})."),
            ));
        }
        if through > today {
            return Err(problem(
                "valid_through",
                format!(
                    "The period ends on {through}, after today ({today}); sales after today are not known yet."
                ),
            ));
        }
        let d = Self { from, through };
        if d.day_count() > MAX_DAYS {
            return Err(problem(
                "valid_from",
                format!(
                    "The period is {} days long; {MAX_DAYS} is the most.",
                    d.day_count()
                ),
            ));
        }
        Ok(d)
    }

    /// Number of days, both ends included.
    pub fn day_count(&self) -> i64 {
        (self.through - self.from).num_days() + 1
    }

    pub fn dates(&self) -> Vec<NaiveDate> {
        self.from
            .iter_days()
            .take_while(|d| *d <= self.through)
            .collect()
    }

    fn back(&self, days: u64) -> Option<Self> {
        Some(Self {
            from: self.from.checked_sub_days(Days::new(days))?,
            through: self.through.checked_sub_days(Days::new(days))?,
        })
    }

    /// The first moment of the period and the first moment after it, in the store's zone.
    pub fn instants(&self, zone: Tz) -> (DateTime<Utc>, DateTime<Utc>) {
        let start = |d: NaiveDate| {
            zone.from_local_datetime(&d.and_time(chrono::NaiveTime::MIN))
                .earliest()
                .map_or_else(
                    || d.and_time(chrono::NaiveTime::MIN).and_utc(),
                    |t| t.with_timezone(&Utc),
                )
        };
        let next = self.through.succ_opt().unwrap_or(self.through);
        (start(self.from), start(next))
    }
}

/// The days a period is compared with, in order.
pub fn compare_days(
    period: &Span,
    mode: CompareMode,
    custom: Option<(Option<NaiveDate>, Option<NaiveDate>)>,
) -> Result<Vec<NaiveDate>, PeriodProblem> {
    match mode {
        CompareMode::SameWeekdayLast4Weeks => {
            if period.day_count() > 7 {
                return Err(problem(
                    "compare_mode",
                    format!(
                        "same_weekday_last_4_weeks compares periods of 7 days at most; this one has {}. Use previous_period or custom.",
                        period.day_count()
                    ),
                ));
            }
            let mut out = Vec::new();
            for week in (1..=WEEKS_BACK).rev() {
                let w = period
                    .back(7 * week)
                    .ok_or_else(|| problem("valid_from", "The period is too early to compare."))?;
                out.extend(w.dates());
            }
            Ok(out)
        }
        CompareMode::PreviousPeriod => {
            let n = u64::try_from(period.day_count()).unwrap_or(0);
            let w = period
                .back(n)
                .ok_or_else(|| problem("valid_from", "The period is too early to compare."))?;
            Ok(w.dates())
        }
        CompareMode::Custom => {
            let (Some(from), Some(through)) = custom.unwrap_or((None, None)) else {
                return Err(problem(
                    "compare_from",
                    "compare_mode custom needs compare_from and compare_through.",
                ));
            };
            if through < from {
                return Err(problem(
                    "compare_through",
                    format!("The comparison ends ({through}) before it starts ({from})."),
                ));
            }
            if through >= period.from {
                return Err(problem(
                    "compare_through",
                    format!(
                        "The comparison must end before the period starts ({}).",
                        period.from
                    ),
                ));
            }
            let w = Span { from, through };
            if w.day_count() > MAX_DAYS {
                return Err(problem(
                    "compare_from",
                    format!(
                        "The comparison is {} days long; {MAX_DAYS} is the most.",
                        w.day_count()
                    ),
                ));
            }
            Ok(w.dates())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn the_last_four_thursdays_for_a_thursday() {
        let p = Span::new(d(11, 5), d(11, 5), d(11, 6)).unwrap();
        assert_eq!(
            compare_days(&p, CompareMode::SameWeekdayLast4Weeks, None).unwrap(),
            [d(10, 8), d(10, 15), d(10, 22), d(10, 29)]
        );
        let week = Span::new(d(11, 2), d(11, 3), d(11, 6)).unwrap();
        assert_eq!(
            compare_days(&week, CompareMode::SameWeekdayLast4Weeks, None)
                .unwrap()
                .len(),
            8
        );
    }

    #[test]
    fn previous_and_custom_periods() {
        let p = Span::new(d(11, 2), d(11, 5), d(11, 6)).unwrap();
        assert_eq!(
            compare_days(&p, CompareMode::PreviousPeriod, None).unwrap(),
            [d(10, 29), d(10, 30), d(10, 31), d(11, 1)]
        );
        let c = compare_days(
            &p,
            CompareMode::Custom,
            Some((Some(d(10, 23)), Some(d(10, 25)))),
        )
        .unwrap();
        assert_eq!(c.len(), 3);
        let e = compare_days(
            &p,
            CompareMode::Custom,
            Some((Some(d(10, 23)), Some(d(11, 2)))),
        )
        .unwrap_err();
        assert_eq!(e.field, "compare_through");
        assert!(compare_days(&p, CompareMode::Custom, None).is_err());
    }

    #[test]
    fn bad_periods_say_which_field() {
        assert_eq!(
            Span::new(d(11, 5), d(11, 4), d(11, 6)).unwrap_err().field,
            "valid_through"
        );
        assert!(
            Span::new(d(11, 5), d(11, 7), d(11, 6)).is_err(),
            "ends after today"
        );
        let long = Span::new(d(10, 1), d(10, 20), d(11, 6)).unwrap();
        assert_eq!(
            compare_days(&long, CompareMode::SameWeekdayLast4Weeks, None)
                .unwrap_err()
                .field,
            "compare_mode"
        );
    }

    #[test]
    fn instants_follow_the_store_zone_across_the_clock_change() {
        let p = Span::new(d(11, 1), d(11, 1), d(11, 6)).unwrap();
        let (start, end) = p.instants(chrono_tz::America::New_York);
        assert_eq!(start.to_rfc3339(), "2026-11-01T04:00:00+00:00");
        assert_eq!(end.to_rfc3339(), "2026-11-02T05:00:00+00:00");
    }
}
