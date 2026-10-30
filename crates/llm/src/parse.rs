//! Reading JSON and tool calls out of text, for models and providers that cannot do guided
//! decoding or native tool calls (docs/backend/agent.md §8.2).
//!
//! Small models write the same intent in several shapes. The parsers accept, in order: a
//! `<tool_call>` tag (also unclosed), a `<|python_tag|>` prefix, a fenced code block, a bare
//! JSON object or list (`{"name", "arguments"}`, `{"tool", "args"}`, `{"tool_calls": [...]}`),
//! and the call form `name({...})` or `name(key="value", n=3)` for known tool names.

use serde_json::{Map, Value};

use crate::chat::ToolCall;

/// The first JSON object in `text`: the whole text, a fenced block, or the first balanced
/// `{…}` that parses. Tolerates trailing commas and Python's `True`/`False`/`None`.
pub fn extract_json(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Some(v) = lenient(trimmed).filter(Value::is_object) {
        return Some(v);
    }
    for block in fenced_blocks(text) {
        if let Some(v) = lenient(block.trim()).filter(Value::is_object) {
            return Some(v);
        }
    }
    balanced_values(text, b'{', b'}').into_iter().next()
}

/// Tool calls written as text. `known_tools` gates the `name(args)` form so prose such as
/// "call me (soon)" is never read as a call.
pub fn tool_calls_in_text(text: &str, known_tools: &[&str]) -> Vec<ToolCall> {
    let mut found = Vec::new();

    let tagged = tagged_sections(text, "<tool_call>", "</tool_call>");
    for section in &tagged {
        collect_json_calls(section, known_tools, &mut found);
    }
    if found.is_empty()
        && let Some(rest) = text.split_once("<|python_tag|>").map(|(_, r)| r)
    {
        collect_json_calls(rest, known_tools, &mut found);
    }
    if found.is_empty() {
        for block in fenced_blocks(text) {
            collect_json_calls(block, known_tools, &mut found);
        }
    }
    if found.is_empty() {
        collect_json_calls(text, known_tools, &mut found);
    }

    found
        .into_iter()
        .enumerate()
        .map(|(n, (name, arguments))| ToolCall {
            id: format!("text_{}", n + 1),
            name,
            arguments,
        })
        .collect()
}

/// JSON calls in `text`; when there are none, the call form for known tools.
fn collect_json_calls(text: &str, known: &[&str], out: &mut Vec<(String, Value)>) {
    let before = out.len();
    let whole = lenient(text.trim());
    let values = match whole {
        Some(v) => vec![v],
        None => {
            let mut v = balanced_values(text, b'[', b']');
            v.retain(Value::is_array);
            if v.is_empty() {
                balanced_values(text, b'{', b'}')
            } else {
                v
            }
        }
    };
    for v in values {
        calls_in_value(&v, out);
    }
    if out.len() == before {
        out.extend(call_forms(text, known));
    }
}

fn calls_in_value(v: &Value, out: &mut Vec<(String, Value)>) {
    match v {
        Value::Array(items) => items.iter().for_each(|i| calls_in_value(i, out)),
        Value::Object(o) => {
            if let Some(Value::Array(calls)) = o.get("tool_calls") {
                calls.iter().for_each(|c| calls_in_value(c, out));
                return;
            }
            if let Some(f @ Value::Object(_)) = o.get("function") {
                calls_in_value(f, out);
                return;
            }
            let name = ["name", "tool", "tool_name", "function_name"]
                .iter()
                .find_map(|k| o.get(*k).and_then(Value::as_str));
            let args = ["arguments", "parameters", "args", "input"]
                .iter()
                .find_map(|k| o.get(*k));
            if let Some(name) = name {
                let args = match args {
                    Some(Value::String(s)) => lenient(s).unwrap_or(Value::Object(Map::new())),
                    Some(a @ Value::Object(_)) => a.clone(),
                    _ => Value::Object(Map::new()),
                };
                out.push((name.to_owned(), args));
            }
        }
        _ => {}
    }
}

/// `name({...})` and `name(key=value, …)` for each known tool name, in text order.
fn call_forms(text: &str, known: &[&str]) -> Vec<(String, Value)> {
    let mut hits: Vec<(usize, String, Value)> = Vec::new();
    for name in known {
        let pattern = format!("{name}(");
        let mut from = 0;
        while let Some(i) = text[from..].find(&pattern) {
            let start = from + i;
            from = start + pattern.len();
            // A longer identifier ending in this name (`my_search_products(`) is not a call.
            let boundary = text[..start]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
            if !boundary {
                continue;
            }
            let Some(inner) = balanced_from(text, start + pattern.len() - 1, b'(', b')') else {
                continue;
            };
            let inner = inner.trim();
            let args = if inner.is_empty() {
                Some(Value::Object(Map::new()))
            } else if inner.starts_with('{') {
                lenient(inner).filter(Value::is_object)
            } else {
                keyword_args(inner)
            };
            if let Some(args) = args {
                hits.push((start, (*name).to_owned(), args));
            }
        }
    }
    hits.sort_by_key(|(at, _, _)| *at);
    hits.into_iter().map(|(_, n, a)| (n, a)).collect()
}

/// `key="value", n=3, ids=["a", "b"]` as a JSON object.
fn keyword_args(s: &str) -> Option<Value> {
    let mut out = Map::new();
    for part in split_top_level(s, b',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (k, v) = part.split_once('=').or_else(|| part.split_once(':'))?;
        let key = k.trim().trim_matches(|c| c == '"' || c == '\'');
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        out.insert(key.to_owned(), literal(v.trim()));
    }
    Some(Value::Object(out))
}

/// A Python-ish literal: quoted strings, JSON, `True`/`False`/`None`, or a bare word.
fn literal(s: &str) -> Value {
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        return Value::String(s[1..s.len() - 1].replace("\\'", "'"));
    }
    if let Some(v) = lenient(s) {
        return v;
    }
    if s.starts_with('[') && s.ends_with(']') {
        let items = split_top_level(&s[1..s.len() - 1], b',')
            .into_iter()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(literal)
            .collect();
        return Value::Array(items);
    }
    Value::String(s.to_owned())
}

/// Parses JSON, then retries without trailing commas and with Python constants replaced.
fn lenient(s: &str) -> Option<Value> {
    if s.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        return Some(v);
    }
    let fixed = outside_strings(s, |word| match word {
        "True" => Some("true"),
        "False" => Some("false"),
        "None" => Some("null"),
        _ => None,
    });
    serde_json::from_str::<Value>(&fixed).ok()
}

/// Rewrites `s` outside string literals: drops commas before `}`/`]` and replaces bare words.
fn outside_strings(s: &str, replace: impl Fn(&str) -> Option<&'static str>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                quote = Some(c);
                out.push(c);
            }
            ',' => {
                let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
                if !matches!(next, Some('}' | ']')) {
                    out.push(c);
                }
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                out.push_str(replace(&word).unwrap_or(&word));
                continue;
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// The contents of ``` fenced blocks, in order.
fn fenced_blocks(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("```") {
        let after = &rest[open + 3..];
        // Skip the info string (`json`, `tool_code`) up to the end of the line.
        let body_start = after.find('\n').map_or(after.len(), |n| n + 1);
        let body = &after[body_start..];
        match body.find("```") {
            Some(close) => {
                out.push(&body[..close]);
                rest = &body[close + 3..];
            }
            None => {
                out.push(body);
                break;
            }
        }
    }
    out
}

/// Text between `open` and `close` tags; an unclosed last tag runs to the end.
fn tagged_sections<'a>(text: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(open) {
        let after = &rest[i + open.len()..];
        match after.find(close) {
            Some(j) => {
                out.push(&after[..j]);
                rest = &after[j + close.len()..];
            }
            None => {
                out.push(after);
                break;
            }
        }
    }
    out
}

/// Every balanced `open…close` span in `text` that parses as JSON, outermost first.
fn balanced_values(text: &str, open: u8, close: u8) -> Vec<Value> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == open
            && let Some(inner) = balanced_from(text, i, open, close)
        {
            let span = &text[i..i + inner.len() + 2];
            if let Some(v) = lenient(span) {
                out.push(v);
                i += span.len();
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The text inside the bracket at byte `at` up to its match, skipping quoted strings.
fn balanced_from(text: &str, at: usize, open: u8, close: u8) -> Option<&str> {
    let bytes = text.as_bytes();
    if bytes.get(at) != Some(&open) {
        return None;
    }
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut i = at;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
        } else if b == b'"' || (b == b'\'' && open == b'(') {
            quote = Some(b);
        } else if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return text.get(at + 1..i);
            }
        }
        i += 1;
    }
    None
}

/// Splits on `sep` where it is not inside brackets or quotes.
fn split_top_level(s: &str, sep: u8) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
        } else {
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'[' | b'{' | b'(' => depth += 1,
                b']' | b'}' | b')' => depth -= 1,
                _ if b == sep && depth == 0 => {
                    parts.push(&s[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    parts.push(&s[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TOOLS: &[&str] = &["search_products", "list_tiers", "set_sale_prices"];

    fn names(calls: &[ToolCall]) -> Vec<&str> {
        calls.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn json_from_whole_text_fence_or_prose() {
        assert_eq!(extract_json(r#" {"a": 1} "#), Some(json!({"a": 1})));
        assert_eq!(
            extract_json("Sure.\n```json\n{\"agent\": \"shopper\", \"confidence\": 0.9}\n```"),
            Some(json!({"agent": "shopper", "confidence": 0.9}))
        );
        assert_eq!(
            extract_json("The plan is {\"steps\": [{\"id\": \"s1\"}],} as asked."),
            Some(json!({"steps": [{"id": "s1"}]}))
        );
        assert_eq!(
            extract_json("{\"exclude\": True, \"note\": \"True story\"}"),
            Some(json!({"exclude": true, "note": "True story"}))
        );
        assert_eq!(extract_json("no json here"), None);
        assert_eq!(extract_json("[1, 2]"), None);
    }

    #[test]
    fn hermes_style_tags_including_unclosed() {
        let text = "<tool_call>\n{\"name\": \"search_products\", \"arguments\": {\"query\": \"jacket\"}}\n</tool_call>\n<tool_call>{\"name\": \"list_tiers\", \"arguments\": \"{}\"}";
        let calls = tool_calls_in_text(text, TOOLS);
        assert_eq!(names(&calls), ["search_products", "list_tiers"]);
        assert_eq!(calls[0].arguments, json!({"query": "jacket"}));
        assert_eq!(calls[1].arguments, json!({}));
        assert_eq!(calls[0].id, "text_1");
    }

    #[test]
    fn python_tag_and_fenced_lists() {
        let calls = tool_calls_in_text(
            "<|python_tag|>{\"name\": \"list_tiers\", \"parameters\": {}}",
            TOOLS,
        );
        assert_eq!(names(&calls), ["list_tiers"]);

        let calls = tool_calls_in_text(
            "```json\n[{\"tool\": \"search_products\", \"args\": {\"category\": \"summer_outerwear\"}},\n {\"tool\": \"list_tiers\", \"args\": {}}]\n```",
            TOOLS,
        );
        assert_eq!(names(&calls), ["search_products", "list_tiers"]);
        assert_eq!(calls[0].arguments["category"], "summer_outerwear");
    }

    #[test]
    fn openai_shaped_json_in_text() {
        let calls = tool_calls_in_text(
            r#"{"tool_calls": [{"id": "x", "type": "function", "function": {"name": "list_tiers", "arguments": "{\"limit\": 5}"}}]}"#,
            TOOLS,
        );
        assert_eq!(names(&calls), ["list_tiers"]);
        assert_eq!(calls[0].arguments, json!({"limit": 5}));
    }

    #[test]
    fn call_forms_for_known_tools_only() {
        let calls = tool_calls_in_text(
            "I'll call search_products(query=\"light jacket\", limit=5, in_stock=True) then list_tiers().",
            TOOLS,
        );
        assert_eq!(names(&calls), ["search_products", "list_tiers"]);
        assert_eq!(
            calls[0].arguments,
            json!({"query": "light jacket", "limit": 5, "in_stock": true})
        );

        let calls = tool_calls_in_text(
            "[set_sale_prices({\"percent_off\": 20, \"valid_for_member_tier\": \"vip\"})]",
            TOOLS,
        );
        assert_eq!(names(&calls), ["set_sale_prices"]);
        assert_eq!(calls[0].arguments["percent_off"], 20);

        let calls = tool_calls_in_text(
            "search_products(product_ids=['prd_a', 'prd_b'], note='it''s')",
            TOOLS,
        );
        assert_eq!(calls[0].arguments["product_ids"], json!(["prd_a", "prd_b"]));

        assert!(tool_calls_in_text("call me (soon) or delete_everything()", TOOLS).is_empty());
        assert!(tool_calls_in_text("my_search_products(x=1)", TOOLS).is_empty());
    }
}
