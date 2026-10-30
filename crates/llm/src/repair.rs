//! Deterministic argument repair against closed tool schemas (docs/backend/agent.md §8.3).
//!
//! Tool input schemas are closed and flat, every field has a description, and enums are words
//! with `x-aliases` for what people say instead. That makes it possible to fix what small
//! models usually get wrong without asking them again:
//!
//! | model wrote | becomes |
//! |---|---|
//! | `"20%"`, `"20"` for a number | `20` |
//! | `"VIP members"`, `"vips"`, `"VIP"` for `vip` | `vip` (normalized word, plural, or `x-aliases`) |
//! | `"Saturday"`, `"2026-11-07"` for a date-time | RFC 3339 in the store's zone; an end of a period gets 23:59 |
//! | `"$89"` or `89` for money | `{"value": 89, "currency": "USD"}` (store currency) |
//! | `"summer_outerwear"` for a `^cat_…` id | `cat_summer_outerwear` |
//! | `"a, b"` or `"a"` for a list | `["a", "b"]`, `["a"]` |
//! | a field the schema does not have | dropped, and noted |
//!
//! What cannot be fixed (an unknown enum word, a number out of range) is reported, never
//! guessed. `x-aliases` may be `{"vip": ["VIP members", "vips"]}` (value → phrases) or
//! `{"VIP members": "vip"}` (phrase → value).

use acom_core::StoreTime;
use chrono::{DateTime, Datelike, Duration, NaiveTime, TimeZone, Utc, Weekday};
use serde_json::{Map, Number, Value};

/// What repair needs to know about the store.
#[derive(Debug, Clone)]
pub struct RepairEnv {
    pub time: StoreTime,
    /// The store's clock (the demo clock in demo mode).
    pub now: DateTime<Utc>,
    /// ISO 4217 code of the store currency; empty when unknown (amounts then stay unrepaired).
    pub currency: String,
}

/// Repaired arguments and what was done to them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Repaired {
    pub value: Value,
    /// One line per change made, for the step log.
    pub notes: Vec<String>,
    /// Fields that were dropped because the schema does not have them.
    pub dropped: Vec<String>,
    /// Required fields that are still missing (dotted paths).
    pub missing: Vec<String>,
    /// Values that could not be made valid, as sentences.
    pub errors: Vec<String>,
}

impl Repaired {
    pub fn is_valid(&self) -> bool {
        self.missing.is_empty() && self.errors.is_empty()
    }
}

/// Fits `args` to `schema` (a closed object schema). Never fails: problems are reported.
pub fn repair(args: &Value, schema: &Value, env: &RepairEnv) -> Repaired {
    let mut out = Repaired::default();
    let args = match args {
        Value::Null => Value::Object(Map::new()),
        Value::String(s) => crate::parse::extract_json(s).unwrap_or(Value::Object(Map::new())),
        other => other.clone(),
    };
    out.value = fix(&args, schema, "", env, &mut out).unwrap_or(Value::Object(Map::new()));
    out
}

fn field_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}.{name}")
    }
}

fn label(path: &str) -> &str {
    if path.is_empty() { "the input" } else { path }
}

fn types(schema: &Value) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(t)) => vec![t.as_str()],
        Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect(),
        _ if schema.get("properties").is_some() => vec!["object"],
        _ if schema.get("enum").is_some() => vec!["string"],
        _ => vec![],
    }
}

fn is_money(schema: &Value) -> bool {
    let props = schema.get("properties");
    props.and_then(|p| p.get("value")).is_some() && props.and_then(|p| p.get("currency")).is_some()
}

/// Repairs one value against its schema; `None` drops it (and records why).
fn fix(
    v: &Value,
    schema: &Value,
    path: &str,
    env: &RepairEnv,
    out: &mut Repaired,
) -> Option<Value> {
    let ts = types(schema);
    if ts.is_empty() {
        return Some(v.clone());
    }
    if ts.contains(&"object") {
        return fix_object(v, schema, path, env, out);
    }
    if ts.contains(&"array") {
        return fix_array(v, schema, path, env, out);
    }
    if ts.contains(&"integer") || ts.contains(&"number") {
        return fix_number(v, schema, path, ts.contains(&"integer"), out);
    }
    if ts.contains(&"boolean") {
        return fix_bool(v, path, out);
    }
    if ts.contains(&"string") {
        return fix_string(v, schema, path, env, out);
    }
    Some(v.clone())
}

fn fix_object(
    v: &Value,
    schema: &Value,
    path: &str,
    env: &RepairEnv,
    out: &mut Repaired,
) -> Option<Value> {
    if is_money(schema) && !v.is_object() {
        return money(v, schema, path, env, out);
    }
    let parsed;
    let obj = match v {
        Value::Object(o) => o,
        Value::String(s) => match crate::parse::extract_json(s) {
            Some(Value::Object(o)) => {
                parsed = o;
                out.notes
                    .push(format!("{}: read the JSON text", label(path)));
                &parsed
            }
            _ => {
                out.errors
                    .push(format!("{} must be an object.", label(path)));
                return None;
            }
        },
        _ => {
            out.errors
                .push(format!("{} must be an object.", label(path)));
            return None;
        }
    };
    let empty = Map::new();
    let props = schema
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    let mut fixed = Map::new();
    for (key, value) in obj {
        let p = field_path(path, key);
        match props
            .get_key_value(key)
            .or_else(|| matching_key(props, key))
        {
            Some((name, prop)) => {
                let name = name.clone();
                if name != *key {
                    out.notes.push(format!("{p}: renamed to {name}"));
                }
                if value.is_null() {
                    out.notes.push(format!("{p}: null left out"));
                    continue;
                }
                let child = field_path(path, &name);
                if let Some(v) = fix(value, prop, &child, env, out) {
                    fixed.insert(name, v);
                }
            }
            None if closed => {
                out.dropped.push(p.clone());
                out.notes
                    .push(format!("{p}: not a field of this tool; dropped"));
            }
            None => {
                fixed.insert(key.clone(), value.clone());
            }
        }
    }
    if let Some(Value::Array(required)) = schema.get("required") {
        for r in required.iter().filter_map(Value::as_str) {
            if !fixed.contains_key(r) {
                out.missing.push(field_path(path, r));
            }
        }
    }
    Some(Value::Object(fixed))
}

/// A field written with different case or separators (`percentOff`, `Percent Off`).
fn matching_key<'a>(props: &'a Map<String, Value>, key: &str) -> Option<(&'a String, &'a Value)> {
    let want = word(&camel_to_snake(key));
    props.iter().find(|(k, _)| word(k) == want)
}

fn camel_to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

fn fix_array(
    v: &Value,
    schema: &Value,
    path: &str,
    env: &RepairEnv,
    out: &mut Repaired,
) -> Option<Value> {
    let item_schema = schema.get("items").cloned().unwrap_or(Value::Null);
    let items: Vec<Value> = match v {
        Value::Array(a) => a.clone(),
        Value::String(s) => {
            let item_is_string = types(&item_schema).contains(&"string");
            if s.trim_start().starts_with('[')
                && let Ok(Value::Array(a)) = serde_json::from_str::<Value>(s)
            {
                a
            } else if item_is_string && s.contains(',') {
                out.notes.push(format!("{path}: split the list text"));
                s.split(',')
                    .map(|p| Value::String(p.trim().to_owned()))
                    .filter(|p| p.as_str().is_some_and(|s| !s.is_empty()))
                    .collect()
            } else {
                out.notes.push(format!("{path}: made a one-item list"));
                vec![v.clone()]
            }
        }
        Value::Null => vec![],
        other => {
            out.notes.push(format!("{path}: made a one-item list"));
            vec![other.clone()]
        }
    };
    let mut fixed = Vec::with_capacity(items.len());
    for (n, item) in items.iter().enumerate() {
        if let Some(v) = fix(item, &item_schema, &format!("{path}[{n}]"), env, out) {
            fixed.push(v);
        }
    }
    let len = fixed.len() as u64;
    if let Some(min) = schema.get("minItems").and_then(Value::as_u64)
        && len < min
    {
        out.errors.push(format!(
            "{path} needs at least {min} item(s); it has {len}."
        ));
    }
    if let Some(max) = schema.get("maxItems").and_then(Value::as_u64)
        && len > max
    {
        out.errors
            .push(format!("{path} takes at most {max} item(s); it has {len}."));
    }
    Some(Value::Array(fixed))
}

/// The number in `"20%"`, `"$89.00"`, `"1,284"`, `"20 percent"`.
fn number_in(s: &str) -> Option<Number> {
    let cleaned: String = s
        .trim()
        .trim_end_matches(|c: char| c.is_alphabetic() || c.is_whitespace() || c == '%')
        .chars()
        .filter(|c| !matches!(c, '$' | '€' | '£' | '₩' | '¥' | ',' | ' ' | '%'))
        .collect();
    let cleaned = cleaned.trim_start_matches('+');
    if cleaned.is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(cleaned) {
        Ok(Value::Number(n)) => Some(n),
        _ => None,
    }
}

fn fix_number(
    v: &Value,
    schema: &Value,
    path: &str,
    integer: bool,
    out: &mut Repaired,
) -> Option<Value> {
    let n = match v {
        Value::Number(n) => n.clone(),
        Value::String(s) => match number_in(s) {
            Some(n) => {
                out.notes.push(format!("{path}: read {s:?} as {n}"));
                n
            }
            None => {
                out.errors
                    .push(format!("{path} must be a number; got {s:?}."));
                return None;
            }
        },
        other => {
            out.errors
                .push(format!("{path} must be a number; got {other}."));
            return None;
        }
    };
    let f = n.as_f64().unwrap_or(f64::NAN);
    let n = if integer {
        if f.fract() != 0.0 {
            out.errors
                .push(format!("{path} must be a whole number; got {n}."));
            return None;
        }
        // `20.0` → `20`.
        serde_json::from_str::<Number>(&format!("{}", f as i64)).unwrap_or(n)
    } else {
        n
    };
    let bound = |key: &str| schema.get(key).and_then(Value::as_f64);
    let range_error = match (
        bound("minimum"),
        bound("exclusiveMinimum"),
        bound("maximum"),
        bound("exclusiveMaximum"),
    ) {
        (Some(min), _, _, _) if f < min => Some(format!("at least {min}")),
        (_, Some(min), _, _) if f <= min => Some(format!("more than {min}")),
        (_, _, Some(max), _) if f > max => Some(format!("at most {max}")),
        (_, _, _, Some(max)) if f >= max => Some(format!("less than {max}")),
        _ => None,
    };
    if let Some(r) = range_error {
        out.errors.push(format!("{path} must be {r}; got {n}."));
        return None;
    }
    Some(Value::Number(n))
}

fn fix_bool(v: &Value, path: &str, out: &mut Repaired) -> Option<Value> {
    match v {
        Value::Bool(b) => Some(Value::Bool(*b)),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "y" | "1" | "on" => Some(Value::Bool(true)),
            "false" | "no" | "n" | "0" | "off" => Some(Value::Bool(false)),
            _ => {
                out.errors
                    .push(format!("{path} must be true or false; got {s:?}."));
                None
            }
        },
        Value::Number(n) if n.as_f64() == Some(1.0) => Some(Value::Bool(true)),
        Value::Number(n) if n.as_f64() == Some(0.0) => Some(Value::Bool(false)),
        other => {
            out.errors
                .push(format!("{path} must be true or false; got {other}."));
            None
        }
    }
}

/// Lowercase, separators to `_`, trimmed: the shape of our enum words.
pub fn word(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_sep = true;
    for c in s.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            last_sep = false;
        } else if !last_sep {
            out.push('_');
            last_sep = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

/// `(phrase, value)` pairs from `x-aliases`, in either declared shape.
fn aliases(schema: &Value) -> Vec<(String, String)> {
    let Some(Value::Object(map)) = schema.get("x-aliases") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, v) in map {
        match v {
            Value::String(value) => out.push((k.clone(), value.clone())),
            Value::Array(phrases) => out.extend(
                phrases
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|p| (p.to_owned(), k.clone())),
            ),
            _ => {}
        }
    }
    out
}

/// The enum word `s` means: exact, normalized, singular, or an alias. `None` when unknown.
fn enum_value(s: &str, allowed: &[&str], schema: &Value) -> Option<String> {
    if allowed.contains(&s) {
        return Some(s.to_owned());
    }
    let w = word(s);
    if let Some(a) = allowed.iter().find(|a| word(a) == w) {
        return Some((*a).to_owned());
    }
    if let Some(stem) = w.strip_suffix('s')
        && let Some(a) = allowed.iter().find(|a| word(a) == stem)
    {
        return Some((*a).to_owned());
    }
    alias(s, schema).filter(|v| allowed.contains(&v.as_str()))
}

fn alias(s: &str, schema: &Value) -> Option<String> {
    let w = word(s);
    aliases(schema)
        .into_iter()
        .find(|(phrase, _)| word(phrase) == w)
        .map(|(_, value)| value)
}

/// `Some(prefix)` for the id pattern `^prefix_[a-z0-9_]+$`.
fn id_prefix(pattern: &str) -> Option<&str> {
    let rest = pattern.strip_prefix('^')?.strip_suffix("_[a-z0-9_]+$")?;
    rest.bytes()
        .all(|b| b.is_ascii_lowercase())
        .then_some(rest)
        .filter(|r| !r.is_empty())
}

fn is_suffix_word(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn fix_string(
    v: &Value,
    schema: &Value,
    path: &str,
    env: &RepairEnv,
    out: &mut Repaired,
) -> Option<Value> {
    let raw = match v {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(a) if a.len() == 1 => {
            out.notes.push(format!("{path}: took the only list item"));
            return fix_string(&a[0], schema, path, env, out);
        }
        other => {
            out.errors
                .push(format!("{path} must be text; got {other}."));
            return None;
        }
    };

    // `$item[n].slot`: what an earlier change-set item will produce, bound when the set runs.
    if raw.starts_with("$item[") {
        return Some(Value::String(raw));
    }

    if let Some(Value::Array(allowed)) = schema.get("enum") {
        let allowed: Vec<&str> = allowed.iter().filter_map(Value::as_str).collect();
        return match enum_value(&raw, &allowed, schema) {
            Some(value) => {
                if value != raw {
                    out.notes.push(format!("{path}: {raw:?} means {value:?}"));
                }
                Some(Value::String(value))
            }
            None => {
                out.errors.push(format!(
                    "{path} must be one of {}; {raw:?} is not one of them.",
                    allowed.join(", ")
                ));
                None
            }
        };
    }

    let mut s = raw.clone();
    if let Some(value) = alias(&s, schema) {
        out.notes.push(format!("{path}: {s:?} means {value:?}"));
        s = value;
    }

    let format = schema.get("format").and_then(Value::as_str);
    let leaf = path.rsplit('.').next().unwrap_or(path);
    let time_like = matches!(format, Some("date-time" | "date"))
        || ["_from", "_through", "_until", "_at"]
            .iter()
            .any(|suffix| leaf.ends_with(suffix));
    if time_like {
        let end = ["through", "until", "end", "_to"]
            .iter()
            .any(|w| leaf.contains(w));
        match when(&s, end, env) {
            Some(t) => {
                let text = if format == Some("date") {
                    t.with_timezone(&env.time.zone)
                        .format("%Y-%m-%d")
                        .to_string()
                } else {
                    env.time.format(t)
                };
                if text != s {
                    out.notes.push(format!("{path}: {s:?} is {text}"));
                }
                return Some(Value::String(text));
            }
            None if matches!(format, Some("date-time" | "date")) => {
                out.errors.push(format!(
                    "{path} must be a date or date-time (2026-11-07T00:00); got {s:?}."
                ));
                return None;
            }
            None => {}
        }
    }

    if let Some(pattern) = schema.get("pattern").and_then(Value::as_str) {
        if let Some(prefix) = id_prefix(pattern) {
            let w = word(&s);
            let fixed = if w.starts_with(&format!("{prefix}_")) {
                w
            } else {
                format!("{prefix}_{w}")
            };
            if fixed != s && is_suffix_word(&fixed[prefix.len() + 1..]) {
                out.notes.push(format!("{path}: {s:?} written as {fixed}"));
                s = fixed;
            }
        } else if pattern == "^[a-z][a-z0-9_]*$" && !is_suffix_word(&s) {
            let w = word(&s);
            if !w.is_empty() {
                out.notes.push(format!("{path}: {s:?} written as {w}"));
                s = w;
            }
        }
    }

    let len = s.chars().count() as u64;
    if let Some(min) = schema.get("minLength").and_then(Value::as_u64)
        && len < min
    {
        out.errors
            .push(format!("{path} needs at least {min} character(s)."));
        return None;
    }
    if let Some(max) = schema.get("maxLength").and_then(Value::as_u64)
        && len > max
    {
        out.errors.push(format!(
            "{path} takes at most {max} characters; it has {len}."
        ));
        return None;
    }
    Some(Value::String(s))
}

/// `"$89"` or `89` as `{"value": 89, "currency": <store currency>}`.
fn money(
    v: &Value,
    schema: &Value,
    path: &str,
    env: &RepairEnv,
    out: &mut Repaired,
) -> Option<Value> {
    let value = match v {
        Value::Number(n) => Some(n.clone()),
        Value::String(s) => number_in(s),
        _ => None,
    };
    let Some(value) = value.filter(|_| !env.currency.is_empty()) else {
        out.errors.push(format!(
            "{path} must be an amount with its currency, like {{\"value\": 89.00, \"currency\": \"USD\"}}."
        ));
        return None;
    };
    out.notes
        .push(format!("{path}: {v} as {value} {}", env.currency));
    let mut m = Map::new();
    m.insert("value".into(), Value::Number(value));
    m.insert("currency".into(), Value::String(env.currency.clone()));
    // Check the parts like any other object.
    fix_object(&Value::Object(m), schema, path, env, out)
}

/// A date-time from what people write: RFC 3339, a local date-time, a date, `today`,
/// `tomorrow`, or a weekday (`Saturday`, `this sat`, `next Sunday`). A date alone starts the
/// day, or ends it (23:59) when `end` is true, because periods include their last minute.
fn when(s: &str, end: bool, env: &RepairEnv) -> Option<DateTime<Utc>> {
    let trimmed = s.trim();
    let date_only = trimmed.len() == 10 && trimmed.as_bytes().get(4) == Some(&b'-');
    if let Ok(t) = env.time.parse(trimmed) {
        return Some(if date_only && end { end_of(t, env) } else { t });
    }
    let w = word(trimmed);
    let today = env.now.with_timezone(&env.time.zone).date_naive();
    let day = match w.as_str() {
        "today" | "now" => Some(today),
        "tomorrow" => today.succ_opt(),
        _ => {
            let (next, name) = match w.split_once('_') {
                Some(("next", rest)) => (true, rest),
                Some(("this" | "on", rest)) => (false, rest),
                _ => (false, w.as_str()),
            };
            name.parse::<Weekday>().ok().map(|wd| {
                let ahead = (7 + wd.num_days_from_monday() as i64
                    - today.weekday().num_days_from_monday() as i64)
                    % 7;
                let ahead = if next && ahead == 0 { 7 } else { ahead };
                today + Duration::days(ahead)
            })
        }
    }?;
    let time = if end {
        NaiveTime::from_hms_opt(23, 59, 0)?
    } else {
        NaiveTime::MIN
    };
    env.time
        .zone
        .from_local_datetime(&day.and_time(time))
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

fn end_of(start_of_day: DateTime<Utc>, env: &RepairEnv) -> DateTime<Utc> {
    let local = start_of_day.with_timezone(&env.time.zone).date_naive();
    NaiveTime::from_hms_opt(23, 59, 0)
        .and_then(|t| {
            env.time
                .zone
                .from_local_datetime(&local.and_time(t))
                .earliest()
        })
        .map_or(start_of_day, |t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env() -> RepairEnv {
        let time = StoreTime::new(chrono_tz::America::New_York);
        RepairEnv {
            now: time.parse("2026-11-06T11:02").unwrap(),
            time,
            currency: "USD".into(),
        }
    }

    /// A trimmed `set_sale_prices` input schema, as the manifest serves it.
    fn sale_prices() -> Value {
        json!({
            "type": "object", "additionalProperties": false,
            "required": ["name", "valid_for_member_tier", "valid_from", "valid_through", "scope_mode", "percent_off"],
            "properties": {
                "name": {"type": "string", "description": "Promotion name.", "minLength": 1},
                "valid_for_member_tier": {"type": "string", "pattern": "^[a-z][a-z0-9_]*$",
                    "description": "Tier id or everyone.", "x-aliases": {"vip": ["VIP members", "vips"]}},
                "valid_from": {"type": "string", "format": "date-time", "description": "Start."},
                "valid_through": {"type": "string", "format": "date-time", "description": "End."},
                "scope_mode": {"type": "string", "enum": ["category", "products"], "description": "Scope."},
                "category_id": {"type": "string", "pattern": "^cat_[a-z0-9_]+$", "description": "Category."},
                "product_ids": {"type": "array", "items": {"type": "string", "pattern": "^prd_[a-z0-9_]+$"}, "description": "Products.", "maxItems": 500},
                "percent_off": {"type": "number", "exclusiveMinimum": 0, "exclusiveMaximum": 100, "description": "Percent off."},
                "exclude_low_stock": {"type": "boolean", "description": "Leave out low stock."},
                "overrides": {"type": "array", "description": "Per product.", "items": {
                    "type": "object", "additionalProperties": false, "required": ["product_id", "percent_off"],
                    "properties": {
                        "product_id": {"type": "string", "pattern": "^prd_[a-z0-9_]+$", "description": "Product."},
                        "percent_off": {"type": "number", "description": "Percent off."}}}}
            }
        })
    }

    #[test]
    fn fixes_what_a_small_model_writes_for_the_vip_sale() {
        let args = json!({
            "name": "VIP weekend", "valid_for_member_tier": "VIP members",
            "valid_from": "Saturday", "valid_through": "Sunday",
            "scope_mode": "Category", "category_id": "summer outerwear",
            "percent_off": "20%", "exclude_low_stock": "yes",
            "overrides": [{"productId": "seersucker_overshirt", "percent_off": "15"}],
            "banner": "home"
        });
        let r = repair(&args, &sale_prices(), &env());
        assert!(r.is_valid(), "{r:#?}");
        assert_eq!(
            r.value,
            json!({
                "name": "VIP weekend", "valid_for_member_tier": "vip",
                "valid_from": "2026-11-07T00:00:00-05:00", "valid_through": "2026-11-08T23:59:00-05:00",
                "scope_mode": "category", "category_id": "cat_summer_outerwear",
                "percent_off": 20, "exclude_low_stock": true,
                "overrides": [{"product_id": "prd_seersucker_overshirt", "percent_off": 15}]
            })
        );
        assert_eq!(r.dropped, ["banner"]);
    }

    #[test]
    fn dates_keep_explicit_times_and_close_periods_at_the_last_minute() {
        let r = repair(
            &json!({"name": "x", "valid_for_member_tier": "vips", "valid_from": "2026-11-07",
                    "valid_through": "2026-11-08", "scope_mode": "products",
                    "product_ids": "prd_windbreaker, prd_field_jacket", "percent_off": 20}),
            &sale_prices(),
            &env(),
        );
        assert!(r.is_valid(), "{r:#?}");
        assert_eq!(r.value["valid_from"], "2026-11-07T00:00:00-05:00");
        assert_eq!(r.value["valid_through"], "2026-11-08T23:59:00-05:00");
        assert_eq!(r.value["valid_for_member_tier"], "vip");
        assert_eq!(
            r.value["product_ids"],
            json!(["prd_windbreaker", "prd_field_jacket"])
        );

        let e = env();
        let next = when("next friday", false, &e).unwrap();
        assert_eq!(e.time.format(next), "2026-11-13T00:00:00-05:00");
        let today = when("today", true, &e).unwrap();
        assert_eq!(e.time.format(today), "2026-11-06T23:59:00-05:00");
        let exact = when("2026-11-07T10:30:00Z", true, &e).unwrap();
        assert_eq!(e.time.format(exact), "2026-11-07T05:30:00-05:00");
    }

    #[test]
    fn reports_what_it_cannot_fix() {
        let r = repair(
            &json!({"valid_for_member_tier": "vip", "scope_mode": "everything",
                    "percent_off": 120, "valid_from": "someday"}),
            &sale_prices(),
            &env(),
        );
        assert!(!r.is_valid());
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("scope_mode must be one of category, products")),
            "{r:#?}"
        );
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("percent_off must be less than 100")),
            "{r:#?}"
        );
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("valid_from must be a date")),
            "{r:#?}"
        );
        assert!(r.missing.contains(&"name".to_owned()));
        assert!(r.missing.contains(&"valid_through".to_owned()));
    }

    #[test]
    fn money_integers_and_aliases_in_both_shapes() {
        let schema = json!({
            "type": "object", "additionalProperties": false, "required": ["client_total"],
            "properties": {
                "client_total": {"type": "object", "additionalProperties": false, "required": ["value", "currency"],
                    "properties": {"value": {"type": "number", "description": "Amount."},
                                   "currency": {"type": "string", "pattern": "^[A-Z]{3}$", "description": "Code."}},
                    "description": "Total."},
                "quantity": {"type": "integer", "minimum": 1, "description": "How many."},
                "reason": {"type": "string", "enum": ["defect", "changed_mind"], "description": "Why.",
                           "x-aliases": {"broken": "defect", "coming apart": "defect"}}
            }
        });
        let r = repair(
            &json!({"client_total": "$71.20", "quantity": "2.0", "reason": "Coming apart"}),
            &schema,
            &env(),
        );
        assert!(r.is_valid(), "{r:#?}");
        assert_eq!(r.value["client_total"]["value"].to_string(), "71.20");
        assert_eq!(r.value["client_total"]["currency"], "USD");
        assert_eq!(r.value["quantity"], 2);
        assert_eq!(r.value["reason"], "defect");
        let r = repair(&json!({"client_total": 10, "quantity": 0}), &schema, &env());
        assert!(
            r.errors
                .iter()
                .any(|e| e.contains("quantity must be at least 1"))
        );
    }

    #[test]
    fn words_and_arguments_sent_as_text() {
        assert_eq!(word("  VIP Members "), "vip_members");
        assert_eq!(word("light-jacket!"), "light_jacket");
        let schema = json!({"type": "object", "additionalProperties": false,
            "properties": {"limit": {"type": "integer", "description": "Max."}}});
        let r = repair(&json!("{\"limit\": \"5\"}"), &schema, &env());
        assert_eq!(r.value, json!({"limit": 5}));
        assert_eq!(id_prefix("^prd_[a-z0-9_]+$"), Some("prd"));
        assert_eq!(id_prefix("^[A-Z]{3}$"), None);
    }

    #[test]
    fn change_set_placeholders_pass_untouched() {
        let schema = json!({"type": "object", "additionalProperties": false,
            "properties": {"promotion_id": {"type": "string", "pattern": "^promo_[a-z0-9_]+$", "description": "Promotion."}}});
        let r = repair(
            &json!({"promotion_id": "$item[1].promotion_id"}),
            &schema,
            &env(),
        );
        assert_eq!(r.value["promotion_id"], "$item[1].promotion_id");
        assert!(r.notes.is_empty());
    }
}
