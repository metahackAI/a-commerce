//! `sales_explainer` (docs/api/chat.md §3.3): a sales question answered from
//! `explain_sales_change`, with the daily series of `get_sales_trend`. Read only: its buttons
//! are `say`s whose words are the next request, each one a change set for another agent.
//!
//! Every amount is copied from the tool results; the builder only words them.

use std::collections::BTreeMap;
use std::str::FromStr;

use chrono::{DateTime, FixedOffset, NaiveDate};
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::{Action, Block, SuggestedIntent};
use crate::profile::NextAction;

/// An amount a tool returned, written short for a sentence: `$2,700`, `$1,312.50`.
fn short_money(v: &Value) -> Option<String> {
    let value = match v.get("value")? {
        Value::Number(n) => Decimal::from_str(&n.to_string()).ok()?,
        Value::String(s) => Decimal::from_str(s).ok()?,
        _ => return None,
    };
    let size = value.abs();
    let whole = size.trunc();
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let cents = size - whole;
    let number = if cents.is_zero() {
        grouped
    } else {
        format!("{grouped}{}", &format!("{:.2}", cents.round_dp(2))[1..])
    };
    let sign = if value.is_sign_negative() && !value.is_zero() {
        "−"
    } else {
        ""
    };
    Some(match v.get("currency").and_then(Value::as_str) {
        Some("USD") | None => format!("{sign}${number}"),
        Some(c) => format!("{sign}{number} {c}"),
    })
}

/// "M and L", "A, B and C".
fn and_list(words: &[String]) -> String {
    match words {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_lowercase().chain(c).collect())
        .unwrap_or_default()
}

fn upper_first(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// An RFC 3339 time a tool wrote, in its own (store) offset.
fn time(v: &Value) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(v.as_str()?).ok()
}

fn date(v: &Value) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(v.as_str()?, "%Y-%m-%d").ok()
}

/// How the comparison reads in sentences: ("Thursday average", "vs the last 4 Thursdays",
/// "an average Thursday").
fn basis_words(explain: &Value) -> (String, String, String) {
    let period = &explain["period"];
    let one_day = period["valid_from"] == period["valid_through"];
    match explain["compare_to"]["basis"].as_str() {
        Some("same_weekday_last_4_weeks") if one_day => {
            let day = date(&period["valid_from"])
                .map(|d| d.format("%A").to_string())
                .unwrap_or_else(|| "day".to_owned());
            (
                format!("{day} average"),
                format!("vs the last 4 {day}s"),
                format!("an average {day}"),
            )
        }
        Some("same_weekday_last_4_weeks") => (
            "Average of the last 4 weeks".to_owned(),
            "vs the same days of the last 4 weeks".to_owned(),
            "an average week".to_owned(),
        ),
        Some("previous_period") => (
            "Period before".to_owned(),
            "vs the period before".to_owned(),
            "the period before".to_owned(),
        ),
        _ => (
            "Comparison".to_owned(),
            "vs the comparison period".to_owned(),
            "the comparison period".to_owned(),
        ),
    }
}

/// `{name}` placeholders filled from `values`; `None` when one has no value.
fn fill(template: &str, values: &BTreeMap<&str, String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}')? + open;
        let key = &rest[open + 1..close];
        out.push_str(values.get(key).filter(|v| !v.is_empty())?);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// One cause of the tool's output as a table row, and the values its next steps are worded from.
fn row(cause: &Value, basis: &str) -> (Value, BTreeMap<&'static str, String>) {
    let kind = cause["cause"].as_str().unwrap_or("other");
    let detail = cause["detail"].as_str().unwrap_or_default();
    let expected = short_money(&cause["expected"]).unwrap_or_else(|| cause["expected"].to_string());
    let mut values = BTreeMap::new();
    let (title, sub, evidence, ids) = match kind {
        "out_of_stock" => {
            let products = cause["products"].as_array().cloned().unwrap_or_default();
            let names: Vec<String> = products
                .iter()
                .filter_map(|p| p["name"].as_str().map(str::to_owned))
                .collect();
            let mut sizes: Vec<String> = Vec::new();
            for p in &products {
                for s in p["sizes"].as_array().into_iter().flatten() {
                    if let Some(s) = s.as_str().filter(|s| !sizes.iter().any(|x| x == s)) {
                        sizes.push(s.to_owned());
                    }
                }
            }
            let back: Vec<(&Value, DateTime<FixedOffset>)> = products
                .iter()
                .filter_map(|p| time(&p["availability_starts"]).map(|t| (p, t)))
                .collect();
            values.insert("sizes", and_list(&sizes));
            values.insert("count", products.len().to_string());
            values.insert("names", and_list(&names));
            if let Some(latest) = back.iter().map(|(_, t)| *t).max() {
                let back_names: Vec<String> = back
                    .iter()
                    .filter_map(|(p, _)| p["name"].as_str().map(str::to_owned))
                    .collect();
                values.insert("back", latest.format("%b %-d").to_string());
                values.insert("back_count", back.len().to_string());
                values.insert("back_names", and_list(&back_names));
            }
            let since = time(&cause["since"])
                .map(|t| format!("Sold out {}; ", t.format("%a %H:%M")))
                .unwrap_or_default();
            (
                detail.split(" since ").next().unwrap_or(detail).to_owned(),
                names.join(", "),
                format!("{since}these sizes made {expected} on {basis}"),
                cause["evidence"].clone(),
            )
        }
        "campaign_not_sent" => {
            let c = &cause["campaign"];
            let name = c["name"].as_str().unwrap_or("The campaign");
            values.insert("campaign", lower_first(name));
            let (title, why) = detail
                .split_once(": ")
                .map_or((detail.to_owned(), String::new()), |(t, w)| {
                    (t.to_owned(), upper_first(w))
                });
            (
                title,
                why,
                format!("{name} brings {expected} on {basis}"),
                cause["evidence"].clone(),
            )
        }
        _ => (
            upper_first(detail),
            String::new(),
            format!("Brings {expected} on {basis}"),
            cause["evidence"].clone(),
        ),
    };
    (
        json!({"cause": kind, "title": title, "detail": sub, "evidence": evidence,
               "estimated": cause["amount"], "ids": ids}),
        values,
    )
}

/// The block for one `explain_sales_change` result, with the daily revenue of a
/// `get_sales_trend` result when the turn read one, and the next steps the agent offers.
pub fn sales_explainer(
    explain: &Value,
    trend: Option<&Value>,
    next_actions: &BTreeMap<String, Vec<NextAction>>,
) -> Option<Block> {
    let period = explain.get("period")?;
    let compare = explain.get("compare_to")?;
    let (baseline_label, basis_label, basis_phrase) = basis_words(explain);
    let from = date(&period["valid_from"]);
    let through = date(&period["valid_through"]);
    let period_label = match (from, through) {
        (Some(f), Some(t)) if f == t => f.format("%a %b %-d").to_string(),
        (Some(f), Some(t)) => format!("{} – {}", f.format("%b %-d"), t.format("%b %-d")),
        _ => String::new(),
    };

    let mut causes = Vec::new();
    let mut next = Vec::new();
    for cause in explain["items"].as_array().into_iter().flatten() {
        let (r, values) = row(cause, &basis_phrase);
        let kind = r["cause"].as_str().unwrap_or_default().to_owned();
        for a in next_actions.get(&kind).into_iter().flatten() {
            if let (Some(label), Some(request)) =
                (fill(&a.label, &values), fill(&a.request, &values))
            {
                next.push(json!({"label": label, "request": request, "cause": kind}));
            }
        }
        causes.push(r);
    }
    causes.push(json!({
        "cause": "unexplained", "title": "Not explained", "detail": "",
        "evidence": "Not shown by stock or campaign data",
        "estimated": explain["unexplained"], "ids": [],
    }));

    let series: Vec<Value> = trend
        .and_then(|t| t["items"].as_array())
        .into_iter()
        .flatten()
        .filter(|i| i["acom:metric"] == "revenue" && i.get("acom:dimension").is_none())
        .map(|i| json!({"date": i["observation_date"], "revenue": i["value"]}))
        .collect();

    let data = json!({
        "period": {"valid_from": period["valid_from"], "valid_through": period["valid_through"],
                   "label": period_label},
        "summary": {
            "revenue": period["revenue"],
            "baseline": compare["revenue"],
            "baseline_label": baseline_label,
            "difference": explain["gap"],
            "change_rate": explain["change_rate"],
            "basis": compare["basis"],
            "basis_label": basis_label,
            "orders": {"count": period["order_count"], "baseline": compare["order_count"]},
        },
        "series": series,
        "causes": causes,
        "next": next,
    });
    let mut b = Block::new("sales_explainer", format!("Sales on {period_label}"), data);
    for (i, n) in next.iter().enumerate() {
        let label = n["label"].as_str().unwrap_or_default();
        b.actions.push(Action::say(
            &format!("next_{}", i + 1),
            label,
            json!({"request": n["request"]}),
        ));
        if let Some(kind) = n["cause"].as_str()
            && !b
                .suggested_intents
                .iter()
                .any(|s| s.element == format!("cause:{kind}"))
        {
            b.suggested_intents
                .push(SuggestedIntent::new(format!("cause:{kind}"), label));
        }
    }
    Some(b)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `explain_sales_change` as scene B3 returns it.
    pub(crate) fn b3_explain() -> Value {
        serde_json::from_str(
            r#"{
          "items": [
            {"cause": "out_of_stock", "detail": "3 bestsellers out of M and L since Wed Nov 4 15:00",
             "amount": {"value": -2700.00, "currency": "USD"},
             "evidence": ["prd_chore_coat", "prd_rain_parka", "prd_denim_jacket"],
             "expected": {"value": 2700.00, "currency": "USD"}, "actual": {"value": 0.00, "currency": "USD"},
             "since": "2026-11-04T15:00:00-05:00",
             "products": [
               {"product_id": "prd_chore_coat", "name": "Cotton chore coat", "sizes": ["M", "L"],
                "availability_starts": "2026-11-12T00:00:00-05:00"},
               {"product_id": "prd_rain_parka", "name": "Light rain parka", "sizes": ["M", "L"],
                "availability_starts": null},
               {"product_id": "prd_denim_jacket", "name": "Denim jacket", "sizes": ["M", "L"],
                "availability_starts": "2026-11-12T00:00:00-05:00"}]},
            {"cause": "campaign_not_sent", "detail": "Weekly email not sent: list import failed Thu 08:00",
             "amount": {"value": -1300.00, "currency": "USD"}, "evidence": ["cmp_weekly_1105"],
             "expected": {"value": 1300.00, "currency": "USD"}, "actual": {"value": 0.00, "currency": "USD"},
             "campaign": {"campaign_id": "cmp_weekly_1105", "name": "Weekly email", "channel": "email",
                          "campaign_status": "failed", "scheduled_time": "2026-11-05T08:00:00-05:00",
                          "error_message": "List import failed", "compared_with": []}}
          ],
          "total": 2,
          "acom:metric": "revenue",
          "period": {"valid_from": "2026-11-05", "valid_through": "2026-11-05",
                     "revenue": {"value": 18240.00, "currency": "USD"}, "order_count": 201},
          "compare_to": {"basis": "same_weekday_last_4_weeks",
                         "observation_dates": ["2026-10-08", "2026-10-15", "2026-10-22", "2026-10-29"],
                         "revenue": {"value": 22900.00, "currency": "USD"}, "order_count": 247},
          "gap": {"value": -4660.00, "currency": "USD"},
          "change_rate": -0.2035,
          "unexplained": {"value": -660.00, "currency": "USD"}
        }"#,
        )
        .unwrap()
    }

    pub(crate) fn b3_trend() -> Value {
        json!({"items": [
            {"observation_date": "2026-10-23", "acom:metric": "revenue", "value": {"value": 21480.00, "currency": "USD"}},
            {"observation_date": "2026-11-05", "acom:metric": "revenue", "value": {"value": 18240.00, "currency": "USD"}},
            {"observation_date": "2026-11-05", "acom:metric": "order_count", "value": 201}
        ], "total": 3})
    }

    fn actions() -> BTreeMap<String, Vec<NextAction>> {
        crate::profile::fixtures::repo()
            .get("insights")
            .unwrap()
            .next_actions
            .clone()
    }

    #[test]
    fn b3_becomes_a_sales_explainer_with_three_next_steps() {
        let b = sales_explainer(&b3_explain(), Some(&b3_trend()), &actions()).unwrap();
        assert_eq!(b.ui, "sales_explainer");
        assert_eq!(b.title, "Sales on Thu Nov 5");
        let s = &b.data["summary"];
        assert_eq!(s["revenue"]["value"].to_string(), "18240.00");
        assert_eq!(s["baseline"]["value"].to_string(), "22900.00");
        assert_eq!(s["difference"]["value"].to_string(), "-4660.00");
        assert_eq!(s["baseline_label"], "Thursday average");
        assert_eq!(s["basis_label"], "vs the last 4 Thursdays");
        assert_eq!(s["orders"], json!({"count": 201, "baseline": 247}));
        assert_eq!(
            b.data["series"].as_array().unwrap().len(),
            2,
            "revenue points only"
        );

        let causes = b.data["causes"].as_array().unwrap();
        assert_eq!(causes.len(), 3);
        assert_eq!(causes[0]["title"], "3 bestsellers out of M and L");
        assert_eq!(
            causes[0]["detail"],
            "Cotton chore coat, Light rain parka, Denim jacket"
        );
        assert_eq!(
            causes[0]["evidence"],
            "Sold out Wed 15:00; these sizes made $2,700 on an average Thursday"
        );
        assert_eq!(causes[0]["estimated"]["value"].to_string(), "-2700.00");
        assert_eq!(causes[1]["title"], "Weekly email not sent");
        assert_eq!(causes[1]["detail"], "List import failed Thu 08:00");
        assert_eq!(
            causes[1]["evidence"],
            "Weekly email brings $1,300 on an average Thursday"
        );
        assert_eq!(causes[2]["cause"], "unexplained");
        assert_eq!(causes[2]["estimated"]["value"].to_string(), "-660.00");

        let labels: Vec<&str> = b
            .actions
            .iter()
            .map(|a| match a {
                Action::Say { label, .. } => label.as_str(),
                _ => panic!("only say buttons: {a:?}"),
            })
            .collect();
        assert_eq!(
            labels,
            [
                "Reorder M and L for the 3 products",
                "Show “back Nov 12” on the 2 product pages",
                "Send the weekly email today",
            ]
        );
        assert_eq!(b.person_actions().count(), 0, "read only");
        match &b.actions[0] {
            Action::Say { args, .. } => assert_eq!(
                args["request"],
                "Draft a reorder of sizes M and L for Cotton chore coat, Light rain parka and Denim jacket."
            ),
            other => panic!("{other:?}"),
        }
        assert_eq!(b.data["next"].as_array().unwrap().len(), 3);
        assert!(
            b.suggested_intents
                .iter()
                .any(|s| s.element == "cause:campaign_not_sent")
        );
    }

    #[test]
    fn steps_without_their_values_are_left_out() {
        let mut e = b3_explain();
        for p in e["items"][0]["products"].as_array_mut().unwrap() {
            p["availability_starts"] = Value::Null;
        }
        let b = sales_explainer(&e, None, &actions()).unwrap();
        assert_eq!(b.actions.len(), 2, "no back-in-stock step without a date");
        assert!(b.data["series"].as_array().unwrap().is_empty());
        assert!(sales_explainer(&json!({"items": []}), None, &actions()).is_none());
    }

    #[test]
    fn short_money_and_templates() {
        assert_eq!(
            short_money(&json!({"value": 2700.00, "currency": "USD"})).unwrap(),
            "$2,700"
        );
        assert_eq!(
            short_money(&json!({"value": -1312.5, "currency": "USD"})).unwrap(),
            "−$1,312.50"
        );
        assert_eq!(
            short_money(&json!({"value": 5, "currency": "EUR"})).unwrap(),
            "5 EUR"
        );
        let mut v = BTreeMap::new();
        v.insert("a", "x".to_owned());
        assert_eq!(fill("{a} and {a}", &v).as_deref(), Some("x and x"));
        assert_eq!(fill("{b}", &v), None);
    }
}
