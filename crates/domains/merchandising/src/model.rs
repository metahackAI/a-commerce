//! The merchandising domain's rules as pure functions: display status from the period and the
//! clock, free time of a slot per audience (`slot_free`), banner text limits, what a slot kind
//! may hold, how many popups share a moment, and the policy values they use. No I/O, so the B1
//! numbers are unit-tested here.

use std::collections::BTreeMap;

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
    /// `display.max_items_per_slot`.
    pub max_items_per_slot: usize,
    /// `display.exclude_discontinued`: discontinued products cannot be put on display.
    pub exclude_discontinued: bool,
    /// `display.slot_items`: per slot kind, the kinds of item it may hold (`product`,
    /// `banner`, `collection_page`, `brand`).
    pub slot_items: BTreeMap<String, Vec<String>>,
    /// `popup.max_concurrent_per_page`.
    pub popups_per_page: usize,
    /// `popup.suppress_days_max`.
    pub suppress_days_max: i64,
    /// `curation.no_empty_section`.
    pub no_empty_section: bool,
    /// `cache_invalidation.allowed_resource_types`.
    pub cache_resource_types: Vec<String>,
    /// `cache_invalidation.all_scope_max_per_hour`.
    pub all_scope_max_per_hour: i64,
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

fn number(p: &PolicySet, key: &str) -> Result<i64, ToolError> {
    p.lookup(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| missing(key))
}

fn kinds(p: &PolicySet, key: &str) -> Result<BTreeMap<String, Vec<String>>, ToolError> {
    p.lookup(key)
        .and_then(Value::as_object)
        .ok_or_else(|| missing(key))?
        .iter()
        .map(|(slot, l)| {
            let words = l.as_array().ok_or_else(|| missing(key))?;
            Ok((
                slot.clone(),
                words
                    .iter()
                    .filter_map(|w| w.as_str().map(str::to_owned))
                    .collect(),
            ))
        })
        .collect()
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
            max_items_per_slot: size(p, "merchandising.display.max_items_per_slot")?,
            exclude_discontinued: flag(p, "merchandising.display.exclude_discontinued")?,
            slot_items: kinds(p, "merchandising.display.slot_items")?,
            popups_per_page: size(p, "merchandising.popup.max_concurrent_per_page")?,
            suppress_days_max: number(p, "merchandising.popup.suppress_days_max")?,
            no_empty_section: flag(p, "merchandising.curation.no_empty_section")?,
            cache_resource_types: list(
                p,
                "merchandising.cache_invalidation.allowed_resource_types",
            )?,
            all_scope_max_per_hour: number(
                p,
                "merchandising.cache_invalidation.all_scope_max_per_hour",
            )?,
        })
    }

    /// Whether a booking for `other` keeps `audience` from booking the same time.
    pub fn blocks(&self, audience: &str, other: &str) -> bool {
        audience == other || (other == "everyone" && self.everyone_blocks_tiers)
    }

    /// The item kinds a slot of this kind may hold (`display.slot_items`); none for a kind the
    /// policy does not name.
    pub fn slot_holds(&self, slot_type: &str) -> &[String] {
        self.slot_items.get(slot_type).map_or(&[], Vec::as_slice)
    }

    /// Whether `tool` changes what shoppers see (`reaches_customers`).
    pub fn reaches(&self, tool: &str) -> bool {
        self.reaches_customers.iter().any(|t| t == tool)
    }
}

/// The kind of item an id names, by its prefix: `product`, `banner`, `collection_page` or
/// `brand`.
pub fn item_kind(id: &str) -> Option<&'static str> {
    let (prefix, rest) = id.split_once('_')?;
    if rest.is_empty() {
        return None;
    }
    match prefix {
        "prd" => Some("product"),
        "bnr" => Some("banner"),
        "col" => Some("collection_page"),
        "brn" => Some("brand"),
        _ => None,
    }
}

/// A curated page's status at `at`: a draft stays a draft; a published page follows its period
/// like a booking.
pub fn page_status(
    stored: &str,
    valid_from: Option<DateTime<Utc>>,
    valid_through: Option<DateTime<Utc>>,
    at: DateTime<Utc>,
) -> DisplayStatus {
    match (stored.parse::<DisplayStatus>(), valid_from) {
        (Ok(DisplayStatus::Draft) | Err(_), _) | (_, None) => {
            stored.parse().unwrap_or(DisplayStatus::Draft)
        }
        (_, Some(from)) => display_status(stored, from, valid_through, at),
    }
}

/// The most of `others` that run at one moment inside `from`–`through` (both inclusive).
pub fn most_at_once(
    others: &[(DateTime<Utc>, DateTime<Utc>)],
    from: DateTime<Utc>,
    through: DateTime<Utc>,
) -> usize {
    // The count only rises where a period starts, so the window's start and the starts inside
    // it are the moments to look at.
    let moments = std::iter::once(from).chain(
        others
            .iter()
            .map(|o| o.0)
            .filter(|s| *s > from && *s <= through),
    );
    moments
        .map(|m| others.iter().filter(|o| o.0 <= m && m <= o.1).count())
        .max()
        .unwrap_or(0)
}

/// Ids to try, in order, for a new row named `name`: `col_winter_edit`, then `col_winter_edit_2`
/// … `_9`. Empty when the name has no letter or digit.
pub fn id_candidates(prefix: &str, name: &str) -> Vec<String> {
    let words: String = slug(name).chars().take(40).collect();
    let words = words.trim_end_matches('_');
    if words.is_empty() {
        return Vec::new();
    }
    let base = format!("{prefix}_{words}");
    std::iter::once(base.clone())
        .chain((2..10).map(|n| format!("{base}_{n}")))
        .collect()
}

/// Lowercase words joined by `_`, for ids made from names: `Winter outerwear edit` →
/// `winter_outerwear_edit`.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
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
        // An open-ended booking takes the rest of the window.
        cursor = b.through.map(|t| c.max(t + minute));
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
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
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
        assert_eq!(r.max_items_per_slot, 24);
        assert_eq!(r.popups_per_page, 2);
        assert_eq!(r.suppress_days_max, 7);
        assert_eq!(r.all_scope_max_per_hour, 2);
        assert_eq!(r.slot_holds("product_grid"), ["product"]);
        assert_eq!(r.slot_holds("brand_strip"), ["brand"]);
        assert!(r.slot_holds("unknown").is_empty());
        assert!(r.reaches("set_popup") && r.reaches("update_curation"));
        assert!(!r.reaches("arrange_display_slot"));
    }

    #[test]
    fn items_pages_and_popups() {
        assert_eq!(item_kind("prd_wool_overcoat"), Some("product"));
        assert_eq!(item_kind("brn_northwind"), Some("brand"));
        assert_eq!(item_kind("col_winter_edit"), Some("collection_page"));
        assert_eq!(item_kind("cat_knitwear"), None);
        assert_eq!(item_kind("prd_"), None);
        assert_eq!(slug("Winter outerwear edit!"), "winter_outerwear_edit");

        let fri = t("2026-11-06T11:00:00-05:00");
        assert_eq!(page_status("draft", None, None, fri), DisplayStatus::Draft);
        let from = t("2026-11-13T00:00:00-05:00");
        let through = Some(t("2026-11-20T23:59:00-05:00"));
        // A draft with a tentative period is still a draft.
        assert_eq!(
            page_status("draft", Some(from), through, fri),
            DisplayStatus::Draft
        );
        assert_eq!(
            page_status("scheduled", Some(from), through, fri),
            DisplayStatus::Scheduled
        );
        assert_eq!(
            page_status(
                "scheduled",
                Some(from),
                through,
                t("2026-11-14T09:00:00-05:00")
            ),
            DisplayStatus::Active
        );

        // Holiday hours Dec 20–26; a sale popup Dec 18–21 and a survey Dec 24–31 never share a
        // moment, so at most one other popup runs at once.
        let window = (
            t("2026-12-20T00:00:00-05:00"),
            t("2026-12-26T23:59:00-05:00"),
        );
        let sale = (
            t("2026-12-18T00:00:00-05:00"),
            t("2026-12-21T23:59:00-05:00"),
        );
        let survey = (
            t("2026-12-24T00:00:00-05:00"),
            t("2026-12-31T23:59:00-05:00"),
        );
        assert_eq!(most_at_once(&[sale, survey], window.0, window.1), 1);
        let wrap = (
            t("2026-12-19T00:00:00-05:00"),
            t("2026-12-25T23:59:00-05:00"),
        );
        assert_eq!(most_at_once(&[sale, survey, wrap], window.0, window.1), 2);
        assert_eq!(most_at_once(&[], window.0, window.1), 0);
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
