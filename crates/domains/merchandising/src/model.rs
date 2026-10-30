//! The merchandising domain's rules as pure functions: display status from the period and the
//! clock, free time of a slot per audience (`slot_free`), banner text limits, and the policy
//! values they use. No I/O, so the B1 numbers are unit-tested here.

use acom_contract::{ErrorCode, ToolError};
use acom_core::enums::DisplayStatus;
use acom_policy::PolicySet;
use chrono::{DateTime, Datelike, Duration, Utc};
use chrono_tz::Tz;
use serde_json::Value;

fn missing(key: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INTERNAL,
        format!("The policy value `{key}` is missing or not readable."),
    )
}

/// The policy values displays use (`merchandising.yml`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayRules {
    /// `slot_free.everyone_blocks_tiers`: a booking for everyone also takes the slot from tiers.
    pub everyone_blocks_tiers: bool,
    /// `display.start_not_in_past`.
    pub start_not_in_past: bool,
    /// `display.visible_when`: `in_period`, `in_stock`, `product_sellable`.
    pub visible_when: Vec<String>,
    /// `banner.text_max_length.eyebrow`, `.headline`, `.cta_label`.
    pub eyebrow_max: usize,
    pub headline_max: usize,
    pub cta_label_max: usize,
    /// `banner.image_spec.formats`.
    pub image_formats: Vec<String>,
    /// `reaches_customers`: the tools whose results shoppers see.
    pub reaches_customers: Vec<String>,
}

fn list(p: &PolicySet, key: &str) -> Result<Vec<String>, ToolError> {
    Ok(p.lookup(key)
        .and_then(Value::as_array)
        .ok_or_else(|| missing(key))?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect())
}

fn flag(p: &PolicySet, key: &str) -> Result<bool, ToolError> {
    p.lookup(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| missing(key))
}

fn size(p: &PolicySet, key: &str) -> Result<usize, ToolError> {
    p.lookup(key)
        .and_then(Value::as_u64)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| missing(key))
}

impl DisplayRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            everyone_blocks_tiers: flag(p, "merchandising.slot_free.everyone_blocks_tiers")?,
            start_not_in_past: flag(p, "merchandising.display.start_not_in_past")?,
            visible_when: list(p, "merchandising.display.visible_when")?,
            eyebrow_max: size(p, "merchandising.banner.text_max_length.eyebrow")?,
            headline_max: size(p, "merchandising.banner.text_max_length.headline")?,
            cta_label_max: size(p, "merchandising.banner.text_max_length.cta_label")?,
            image_formats: list(p, "merchandising.banner.image_spec.formats")?,
            reaches_customers: list(p, "merchandising.reaches_customers")?,
        })
    }

    /// Whether a booking for `other` keeps `audience` from booking the same time.
    pub fn blocks(&self, audience: &str, other: &str) -> bool {
        audience == other || (other == "everyone" && self.everyone_blocks_tiers)
    }
}

/// A booking's status at `at`: `draft` and `ended` (ended early or cancelled by a person) stay;
/// the rest follows the period (both ends inclusive, minute precision).
pub fn display_status(
    stored: &str,
    valid_from: DateTime<Utc>,
    valid_through: Option<DateTime<Utc>>,
    at: DateTime<Utc>,
) -> DisplayStatus {
    match stored.parse::<DisplayStatus>() {
        Ok(s @ (DisplayStatus::Draft | DisplayStatus::Ended)) => s,
        _ if at < valid_from => DisplayStatus::Scheduled,
        _ if valid_through.is_some_and(|t| at >= t + Duration::minutes(1)) => DisplayStatus::Ended,
        _ => DisplayStatus::Active,
    }
}

/// A period one audience has booked; `None` as the end means open-ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy {
    pub from: DateTime<Utc>,
    pub through: Option<DateTime<Utc>>,
}

/// The free stretches of `from`–`through` (both inclusive, minute precision) that none of the
/// busy periods cover, in time order.
pub fn free_spans(
    from: DateTime<Utc>,
    through: DateTime<Utc>,
    busy: &[Busy],
) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let minute = Duration::minutes(1);
    let mut busy: Vec<Busy> = busy
        .iter()
        .copied()
        .filter(|b| b.from <= through && b.through.is_none_or(|t| t >= from))
        .collect();
    busy.sort_by_key(|b| b.from);
    let mut out = Vec::new();
    let mut cursor = Some(from);
    for b in busy {
        let Some(c) = cursor else {
            break;
        };
        if b.from > c {
            out.push((c, (b.from - minute).min(through)));
        }
        cursor = match b.through {
            None => None,
            Some(t) => Some(c.max(t + minute)),
        };
    }
    if let Some(c) = cursor
        && c <= through
    {
        out.push((c, through));
    }
    out
}

/// `Nov 7–8`, `Nov 30–Dec 2`, `Nov 7`, in store time; `from Nov 6` when open-ended.
pub fn period_words(from: DateTime<Utc>, through: Option<DateTime<Utc>>, zone: Tz) -> String {
    let f = from.with_timezone(&zone);
    let Some(t) = through.map(|t| t.with_timezone(&zone)) else {
        return format!("from {}", f.format("%b %-d"));
    };
    if f.date_naive() == t.date_naive() {
        f.format("%b %-d").to_string()
    } else if f.month() == t.month() && f.year() == t.year() {
        format!("{}–{}", f.format("%b %-d"), t.format("%-d"))
    } else {
        format!("{}–{}", f.format("%b %-d"), t.format("%b %-d"))
    }
}

/// `Hero slot`, `Product grid slot`.
pub fn slot_words(slot_type: &str) -> String {
    let words = slot_type.replace('_', " ");
    let mut c = words.chars();
    match c.next() {
        Some(first) => format!("{}{} slot", first.to_uppercase(), c.as_str()),
        None => "Slot".to_owned(),
    }
}

/// Where a page's slots show, for approval sentences.
pub fn page_words(page: &str) -> &'static str {
    match page {
        "main" => "the main page",
        "category" => "category pages",
        "product" => "product pages",
        "search" => "search results",
        _ => "the store",
    }
}

/// The image format a URL names by its extension (`webp`, `jpeg`), if any.
pub fn image_format(url: &str) -> Option<&'static str> {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    let ext = path.rsplit('.').next()?;
    match ext {
        "jpg" | "jpeg" => Some("jpeg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        "gif" => Some("gif"),
        "avif" => Some("avif"),
        _ => None,
    }
}

/// `1,284`.
pub fn count_text(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn policies() -> PolicySet {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
        let texts: Vec<(&str, String)> = acom_policy::FILES
            .iter()
            .map(|f| {
                (
                    *f,
                    std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
                )
            })
            .collect();
        PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str()))).unwrap()
    }

    pub(crate) fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn reads_the_display_rules() {
        let r = DisplayRules::from_policy(&policies()).unwrap();
        assert!(!r.everyone_blocks_tiers);
        assert!(r.start_not_in_past);
        assert_eq!(
            (r.eyebrow_max, r.headline_max, r.cta_label_max),
            (24, 60, 24)
        );
        assert!(r.blocks("vip", "vip"));
        assert!(!r.blocks("vip", "everyone"));
        assert!(r.reaches_customers.contains(&"schedule_banner".to_owned()));
    }

    #[test]
    fn free_time_leaves_out_bookings_by_the_minute() {
        let from = t("2026-11-07T00:00:00-05:00");
        let through = t("2026-11-08T23:59:00-05:00");
        // Fall layers holds everyone from Fri 10:00 with no end: nothing is free.
        let fall = Busy {
            from: t("2026-11-06T10:00:00-05:00"),
            through: None,
        };
        assert!(free_spans(from, through, &[fall]).is_empty());
        // No booking for VIP: the whole weekend is free.
        assert_eq!(free_spans(from, through, &[]), vec![(from, through)]);
        // A booking Sat 12:00–18:59 splits the weekend.
        let sat = Busy {
            from: t("2026-11-07T12:00:00-05:00"),
            through: Some(t("2026-11-07T18:59:00-05:00")),
        };
        assert_eq!(
            free_spans(from, through, &[sat]),
            vec![
                (from, t("2026-11-07T11:59:00-05:00")),
                (t("2026-11-07T19:00:00-05:00"), through)
            ]
        );
        // The October edit ended Fri 09:59; it takes nothing on the weekend.
        let october = Busy {
            from: t("2026-10-01T00:00:00-04:00"),
            through: Some(t("2026-11-06T09:59:00-05:00")),
        };
        assert_eq!(free_spans(from, through, &[october]), vec![(from, through)]);
    }

    #[test]
    fn status_follows_the_clock() {
        let from = t("2026-11-07T00:00:00-05:00");
        let through = Some(t("2026-11-08T23:59:00-05:00"));
        let fri = t("2026-11-06T11:05:00-05:00");
        assert_eq!(
            display_status("scheduled", from, through, fri),
            DisplayStatus::Scheduled
        );
        assert_eq!(
            display_status("scheduled", from, through, t("2026-11-07T10:00:00-05:00")),
            DisplayStatus::Active
        );
        assert_eq!(
            display_status("active", from, through, t("2026-11-09T00:00:00-05:00")),
            DisplayStatus::Ended
        );
        assert_eq!(
            display_status("ended", from, None, fri),
            DisplayStatus::Ended
        );
    }

    #[test]
    fn words_for_people() {
        let ny = chrono_tz::America::New_York;
        let from = t("2026-11-07T00:00:00-05:00");
        assert_eq!(
            period_words(from, Some(t("2026-11-08T23:59:00-05:00")), ny),
            "Nov 7–8"
        );
        assert_eq!(
            period_words(
                t("2026-11-30T00:00:00-05:00"),
                Some(t("2026-12-02T23:59:00-05:00")),
                ny
            ),
            "Nov 30–Dec 2"
        );
        assert_eq!(slot_words("hero"), "Hero slot");
        assert_eq!(slot_words("product_grid"), "Product grid slot");
        assert_eq!(
            image_format("https://x.example/b/vip.webp?v=2"),
            Some("webp")
        );
        assert_eq!(image_format("https://x.example/b/vip.JPG"), Some("jpeg"));
        assert_eq!(count_text(1284), "1,284");
    }
}
