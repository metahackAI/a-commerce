//! The agent's checks (docs/backend/agent.md §6–§7). Rule checks are commerce's (the preview's
//! `checks`); the agent only compares, deterministically, what was asked with what a preview
//! will do, and the numbers in its own reply with the turn's tool results.

use std::collections::BTreeSet;
use std::str::FromStr;

use acom_contract::{ChangeOp, Preview};
use acom_core::StoreTime;
use chrono::{Datelike, NaiveDate, Timelike, Weekday};
use rust_decimal::Decimal;
use serde_json::Value;

/// A value the preview does not carry the way it was asked.
#[derive(Debug, Clone, PartialEq)]
pub struct Mismatch {
    pub tool: String,
    pub field: String,
    pub asked: Value,
    /// The values the preview's changes have for the field.
    pub got: Vec<Value>,
}

impl Mismatch {
    pub fn sentence(&self) -> String {
        if self.field == "changes" {
            return format!("The preview of {} changes nothing.", self.tool);
        }
        let got: Vec<String> = self.got.iter().map(ToString::to_string).collect();
        format!(
            "{} was asked with {} = {}, but the preview has {}.",
            self.tool,
            self.field,
            self.asked,
            got.join(", ")
        )
    }
}

fn same(a: &Value, b: &Value, time: &StoreTime) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            match (
                Decimal::from_str(&x.to_string()),
                Decimal::from_str(&y.to_string()),
            ) {
                (Ok(x), Ok(y)) => x == y,
                _ => x == y,
            }
        }
        (Value::String(x), Value::String(y)) => {
            x == y || matches!((time.parse(x), time.parse(y)), (Ok(p), Ok(q)) if p == q)
        }
        _ => a == b,
    }
}

/// Compares a write's arguments with its preview: every plain argument that the changes also
/// carry must appear there with the asked value (other values may appear too, as overrides do).
/// A preview that changes nothing is a mismatch.
pub fn intent_mismatches(
    tool: &str,
    input: &Value,
    preview: &Preview,
    time: &StoreTime,
) -> Vec<Mismatch> {
    let changes: Vec<&Value> = preview
        .changes
        .iter()
        .filter(|c| c.op != ChangeOp::Exclude)
        .map(|c| &c.to)
        .collect();
    if changes.is_empty() {
        return vec![Mismatch {
            tool: tool.to_owned(),
            field: "changes".into(),
            asked: Value::Null,
            got: vec![],
        }];
    }
    let mut out = Vec::new();
    for (field, asked) in input.as_object().into_iter().flatten() {
        let plain = matches!(asked, Value::Number(_) | Value::Bool(_))
            || asked.as_str().is_some_and(|s| !s.starts_with("$item["));
        if !plain {
            continue;
        }
        let got: Vec<Value> = changes
            .iter()
            .filter_map(|to| to.get(field))
            .filter(|v| !v.is_null())
            .cloned()
            .collect();
        if got.is_empty() || got.iter().any(|g| same(asked, g, time)) {
            continue;
        }
        let mut distinct: Vec<Value> = Vec::new();
        for g in got {
            if !distinct.contains(&g) {
                distinct.push(g);
            }
        }
        out.push(Mismatch {
            tool: tool.to_owned(),
            field: field.clone(),
            asked: asked.clone(),
            got: distinct,
        });
    }
    out
}

/// Numbers written in text (`20%`, `$71.20`, `1,284`, `2026-11-07` gives 2026, 11, 7),
/// normalized (`71.20` → `71.2`).
pub fn numbers_in(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len()
            && (chars[i].is_ascii_digit()
                || (matches!(chars[i], ',' | '.')
                    && chars.get(i + 1).is_some_and(char::is_ascii_digit)))
        {
            i += 1;
        }
        let raw: String = chars[start..i].iter().filter(|c| **c != ',').collect();
        if let Ok(d) = Decimal::from_str(&raw) {
            out.push(d.normalize().to_string());
        }
    }
    out
}

fn collect(v: &Value, into: &mut BTreeSet<String>) {
    match v {
        Value::Number(n) => {
            if let Ok(d) = Decimal::from_str(&n.to_string()) {
                into.insert(d.normalize().to_string());
                // Text drops the sign ("down $4,660" for -4660.00), and a 0-1 rate is said
                // as a percent ("20%" for -0.2035, conventions.md §4).
                let size = d.abs();
                into.insert(size.normalize().to_string());
                if !size.is_zero() && size < Decimal::ONE {
                    let percent = size * Decimal::ONE_HUNDRED;
                    into.insert(percent.round().normalize().to_string());
                    into.insert(percent.round_dp(1).normalize().to_string());
                }
            }
        }
        Value::String(s) => {
            into.extend(numbers_in(s));
            // A time is said on the 12-hour clock ("by 8 PM" for 20:00), as the reply prompt
            // asks; timestamps carry the store's offset, so the hour is already local.
            if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
                into.insert(t.hour12().1.to_string());
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect(x, into)),
        Value::Object(o) => o.values().for_each(|x| collect(x, into)),
        _ => {}
    }
}

/// Numbers in `reply` that appear in none of the sources (tool results, the request, the facts
/// the reply was written from). A reply with any is not shown.
pub fn unsupported_numbers(reply: &str, sources: &[&Value], texts: &[&str]) -> Vec<String> {
    let mut known = BTreeSet::new();
    for s in sources {
        collect(s, &mut known);
    }
    for t in texts {
        known.extend(numbers_in(t));
    }
    let mut out: Vec<String> = numbers_in(reply)
        .into_iter()
        .filter(|n| !known.contains(n))
        .collect();
    out.dedup();
    out
}

/// Where `reply` starts copying the instructions it was written from (a small model sometimes
/// carries on with its prompt after the answer), if it does: the first sentence of the prompt,
/// long enough not to be a common phrase, found in the reply.
pub fn echo_start(reply: &str, prompt: &str) -> Option<usize> {
    prompt
        .split(['\n', '.', ';'])
        .map(|s| s.trim().trim_start_matches("- ").trim())
        .filter(|s| s.len() >= 20)
        .filter_map(|s| reply.find(s))
        .min()
}

/// Whether `reply` repeats the person's whole request: a model narrating its own reasoning
/// ("X is selling fast. Draft a reorder… The results show…") instead of answering. Requests
/// under 20 characters are too short to tell.
pub fn repeats_request(reply: &str, request: &str) -> bool {
    let flat = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let request = flat(request);
    request.chars().count() >= 20 && flat(reply).contains(&request)
}

/// The reply with every date said as weekday, month and day ("Sun Nov 2", "Monday, November 2")
/// given the weekday that date falls on, in the year nearest `today`. A small model names the
/// weekday of a date wrong now and then; the calendar does not.
pub fn fix_weekdays(reply: &str, today: NaiveDate) -> String {
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (i, c) in reply
        .char_indices()
        .chain(std::iter::once((reply.len(), ' ')))
    {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                words.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    let gap = |a: usize, b: usize| reply[a..b].chars().all(|c| matches!(c, ' ' | ',' | '.'));
    let mut fixes: Vec<(usize, usize, &str)> = Vec::new();
    for w in words.windows(3) {
        let [(a0, a1), (b0, b1), (c0, c1)] = [w[0], w[1], w[2]];
        let Some((said, long)) = weekday_word(&reply[a0..a1]) else {
            continue;
        };
        let (Some(month), Ok(day)) = (month_word(&reply[b0..b1]), reply[c0..c1].parse::<u32>())
        else {
            continue;
        };
        if !gap(a1, b0) || !gap(b1, c0) {
            continue;
        }
        let nearest = (today.year() - 1..=today.year() + 1)
            .filter_map(|y| NaiveDate::from_ymd_opt(y, month, day))
            .min_by_key(|d| (*d - today).num_days().abs());
        if let Some(date) = nearest
            && date.weekday() != said
        {
            fixes.push((a0, a1, weekday_name(date.weekday(), long)));
        }
    }
    let mut out = reply.to_owned();
    for (a, b, name) in fixes.into_iter().rev() {
        out.replace_range(a..b, name);
    }
    out
}

fn weekday_word(w: &str) -> Option<(Weekday, bool)> {
    let day = match w.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    };
    // Every full name is longer than every short one ("Thurs").
    Some((day, w.len() > 5))
}

fn weekday_name(d: Weekday, long: bool) -> &'static str {
    match (d, long) {
        (Weekday::Mon, false) => "Mon",
        (Weekday::Tue, false) => "Tue",
        (Weekday::Wed, false) => "Wed",
        (Weekday::Thu, false) => "Thu",
        (Weekday::Fri, false) => "Fri",
        (Weekday::Sat, false) => "Sat",
        (Weekday::Sun, false) => "Sun",
        (Weekday::Mon, true) => "Monday",
        (Weekday::Tue, true) => "Tuesday",
        (Weekday::Wed, true) => "Wednesday",
        (Weekday::Thu, true) => "Thursday",
        (Weekday::Fri, true) => "Friday",
        (Weekday::Sat, true) => "Saturday",
        (Weekday::Sun, true) => "Sunday",
    }
}

fn month_word(w: &str) -> Option<u32> {
    let m = match w.to_ascii_lowercase().as_str() {
        "jan" | "january" => 1,
        "feb" | "february" => 2,
        "mar" | "march" => 3,
        "apr" | "april" => 4,
        "may" => 5,
        "jun" | "june" => 6,
        "jul" | "july" => 7,
        "aug" | "august" => 8,
        "sep" | "sept" | "september" => 9,
        "oct" | "october" => 10,
        "nov" | "november" => 11,
        "dec" | "december" => 12,
        _ => return None,
    };
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn time() -> StoreTime {
        StoreTime::new(chrono_tz::America::New_York)
    }

    fn preview(changes: Value) -> Preview {
        serde_json::from_value(
            json!({"preview_id": "pv_1", "expires_at": null, "changes": changes,
            "checks": [], "needs_approval": true, "approval_reason": null}),
        )
        .unwrap()
    }

    #[test]
    fn asked_values_must_appear_in_the_preview() {
        let p = preview(json!([
            {"op": "create", "entity": "promotion", "id": "", "to": {"valid_for_member_tier": "vip",
              "valid_from": "2026-11-07T00:00:00-05:00"}},
            {"op": "update", "entity": "unit_price_specification", "id": "", "to": {"product_id": "prd_a", "percent_off": 20}},
            {"op": "update", "entity": "unit_price_specification", "id": "", "to": {"product_id": "prd_b", "percent_off": 15}},
            {"op": "exclude", "entity": "product", "id": "prd_c", "to": {"why": "promo_min_stock"}}
        ]));
        let ok = json!({"percent_off": 20.0, "valid_for_member_tier": "vip",
                        "valid_from": "2026-11-07T05:00:00Z", "promotion_id": "$item[1].promotion_id"});
        assert_eq!(
            intent_mismatches("set_sale_prices", &ok, &p, &time()),
            vec![]
        );

        let wrong = json!({"percent_off": 25, "valid_for_member_tier": "gold"});
        let m = intent_mismatches("set_sale_prices", &wrong, &p, &time());
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].field, "percent_off");
        assert_eq!(m[0].got, vec![json!(20), json!(15)]);
        assert!(m[1].sentence().contains("valid_for_member_tier"));

        let nothing =
            preview(json!([{"op": "exclude", "entity": "product", "id": "prd_c", "to": {}}]));
        let m = intent_mismatches("set_sale_prices", &ok, &nothing, &time());
        assert_eq!(
            m[0].sentence(),
            "The preview of set_sale_prices changes nothing."
        );
    }

    #[test]
    fn reply_numbers_must_come_from_the_turn() {
        assert_eq!(
            numbers_in("$71.20, 1,284 members, 20% off on 2026-11-07."),
            ["71.2", "1284", "20", "2026", "11", "7"]
        );
        let results = json!({"total": {"value": 71.20, "currency": "USD"}, "members": 1284});
        let ok = "Your total is $71.20 for 1,284 VIP members at 20% off.";
        assert!(unsupported_numbers(ok, &[&results], &["Run a 20% sale"]).is_empty());
        let made_up = "That saves you $19.99 compared with last week.";
        assert_eq!(unsupported_numbers(made_up, &[&results], &[]), ["19.99"]);
    }

    #[test]
    fn a_drop_is_said_without_its_sign_and_a_rate_as_a_percent() {
        let explained =
            json!({"gap": {"value": -4660.00, "currency": "USD"}, "change_rate": -0.2035});
        let reply = "Revenue fell $4,660, about 20% (20.4%) under a normal Thursday.";
        assert!(unsupported_numbers(reply, &[&explained], &[]).is_empty());
        assert_eq!(
            unsupported_numbers("It fell 30%.", &[&explained], &[]),
            ["30"]
        );
    }

    #[test]
    fn a_time_is_said_on_the_twelve_hour_clock() {
        let order = json!({"expected_arrival_until": "2026-11-06T20:00:00-05:00",
                           "scans": [{"at": "2026-11-02T12:43:00-05:00"}]});
        let reply = "UPS picked it up Mon Nov 2 at 12:43 PM; it is due today by 8 PM.";
        assert!(unsupported_numbers(reply, &[&order], &[]).is_empty());
        assert_eq!(
            unsupported_numbers("It is due by 9 PM.", &[&order], &[]),
            ["9"]
        );
    }

    #[test]
    fn a_reply_that_copies_its_instructions_is_cut_there() {
        let prompt = "You write the short answer.\n\n- Two or three plain sentences. The blocks show the details; do not repeat tables.\n- Say only what the results say.";
        let reply =
            "One task needs you. The blocks show the details; do not repeat tables. Say only";
        assert_eq!(echo_start(reply, prompt), Some(20));
        assert_eq!(echo_start("One task needs you: the reorder.", prompt), None);
    }

    #[test]
    fn a_weekday_is_the_one_the_date_falls_on() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        assert_eq!(
            fix_weekdays(
                "The only scan is Sun Nov 2 at 12:43 PM; it is due Fri Nov 6.",
                today
            ),
            "The only scan is Mon Nov 2 at 12:43 PM; it is due Fri Nov 6."
        );
        assert_eq!(
            fix_weekdays(
                "Undo until Tuesday, November 7. Order on sale Sat, Jan 2.",
                today
            ),
            "Undo until Saturday, November 7. Order on sale Sat, Jan 2."
        );
        let said = "since Monday, 3 orders";
        assert_eq!(fix_weekdays(said, today), said);
    }

    #[test]
    fn a_reply_that_repeats_the_whole_request_is_narrating() {
        let asked = "Cushioned ankle socks is selling fast. Draft a reorder for the sizes that will run out.";
        assert!(repeats_request(
            "Cushioned ankle socks is selling fast.  Draft a reorder for the sizes that will run out. The results show a search…",
            asked
        ));
        assert!(!repeats_request(
            "Cushioned ankle socks is selling fast, but nothing needs ordering yet: every size lasts 24 days or more.",
            asked
        ));
        assert!(!repeats_request("Hi there. Hi!", "Hi!"));
    }
}
