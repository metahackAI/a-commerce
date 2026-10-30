//! Closed, flat JSON Schemas for tool inputs.
//!
//! Small models and strict tool callers do best with schemas that have no `$ref`, no
//! `anyOf`/`oneOf`/`allOf`, closed objects, and a description on every field
//! (docs/api/conventions.md, "Tool schemas"). `close` turns what schemars generates into that
//! shape where it safely can; `violations` reports what is still wrong so a test can fail.

use schemars::{JsonSchema, generate::SchemaSettings};
use serde_json::{Map, Value};

/// Generates the closed input schema for `T`.
pub fn closed_schema_for<T: JsonSchema>() -> Value {
    let generator = SchemaSettings::draft2020_12()
        .with(|s| s.inline_subschemas = true)
        .into_generator();
    let mut schema = generator.into_root_schema_for::<T>().to_value();
    if let Some(obj) = schema.as_object_mut() {
        obj.remove("$schema");
        obj.remove("title");
    }
    close(&mut schema);
    schema
}

/// Normalizes in place:
/// - `anyOf: [X, {"type": "null"}]` and `type: [T, "null"]` become `X` / `T` (optional is
///   expressed by leaving the field out of `required`, not by null);
/// - `oneOf` of documented constants becomes `enum` with the value descriptions folded into
///   the field description ("vip: ...; gold: ...").
pub fn close(schema: &mut Value) {
    let Some(obj) = schema.as_object_mut() else {
        return;
    };

    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(alts)) = obj.get(key) {
            let non_null: Vec<&Value> = alts.iter().filter(|a| !is_null_schema(a)).collect();
            if non_null.len() == 1 && alts.len() == 2 {
                let inner = non_null[0].clone();
                obj.remove(key);
                merge_into(obj, inner);
            }
        }
    }

    if let Some(Value::Array(types)) = obj.get("type") {
        let non_null: Vec<Value> = types
            .iter()
            .filter(|t| t.as_str() != Some("null"))
            .cloned()
            .collect();
        if non_null.len() == 1 {
            obj.insert("type".into(), non_null[0].clone());
        }
    }

    // An optional word field lists `null` among its words; optional is expressed by leaving the
    // field out of `required`.
    if let Some(Value::Array(words)) = obj.get_mut("enum") {
        words.retain(|w| !w.is_null());
    }

    if let Some(Value::Array(alts)) = obj.get("oneOf")
        && !alts.is_empty()
        && alts.iter().all(|a| a.get("const").is_some())
    {
        let values: Vec<Value> = alts
            .iter()
            .filter_map(|a| a.get("const").cloned())
            .collect();
        let notes: Vec<String> = alts
            .iter()
            .filter_map(|a| {
                let v = a.get("const")?.as_str()?;
                let d = a.get("description")?.as_str()?;
                Some(format!("{v}: {d}"))
            })
            .collect();
        obj.remove("oneOf");
        obj.insert("type".into(), Value::String("string".into()));
        obj.insert("enum".into(), Value::Array(values));
        if !notes.is_empty() {
            let base = obj
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let joined = notes.join("; ");
            let text = if base.is_empty() {
                joined
            } else {
                format!("{base} ({joined})")
            };
            obj.insert("description".into(), Value::String(text));
        }
    }

    for (_, child) in obj.iter_mut() {
        match child {
            Value::Object(_) => close(child),
            Value::Array(items) => items.iter_mut().for_each(close),
            _ => {}
        }
    }
}

fn is_null_schema(v: &Value) -> bool {
    v.get("type").and_then(Value::as_str) == Some("null")
}

/// Copies keys from `inner` that the outer schema does not set (keeps the outer description).
fn merge_into(outer: &mut Map<String, Value>, inner: Value) {
    if let Value::Object(inner) = inner {
        for (k, v) in inner {
            outer.entry(k).or_insert(v);
        }
    }
}

/// Everything that keeps a schema from being closed and flat. Empty means it is fine.
pub fn violations(schema: &Value) -> Vec<String> {
    let mut out = Vec::new();
    walk(schema, "$", &mut out);
    out
}

fn walk(schema: &Value, path: &str, out: &mut Vec<String>) {
    let Some(obj) = schema.as_object() else {
        return;
    };
    for key in ["$ref", "anyOf", "oneOf", "allOf", "not", "$defs"] {
        if obj.contains_key(key) {
            out.push(format!("{path}: uses `{key}`"));
        }
    }
    if obj.get("type").and_then(Value::as_str) == Some("object") {
        if obj.get("additionalProperties") != Some(&Value::Bool(false)) {
            out.push(format!(
                "{path}: object is open (add #[serde(deny_unknown_fields)])"
            ));
        }
        if let Some(Value::Object(props)) = obj.get("properties") {
            for (name, prop) in props {
                let p = format!("{path}.{name}");
                if prop
                    .get("description")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    out.push(format!("{p}: missing description"));
                }
                walk(prop, &p, out);
            }
        }
    }
    if let Some(items) = obj.get("items") {
        walk(items, &format!("{path}[]"), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::Deserialize;

    /// Who a sale price is for.
    #[derive(Deserialize, JsonSchema)]
    #[serde(rename_all = "snake_case")]
    #[allow(dead_code)]
    enum Audience {
        /// Every shopper.
        Everyone,
        /// Members of one tier.
        Tier,
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct Item {
        /// Product to put on sale.
        product_id: String,
        /// Percent off the list price, 0-100.
        percent_off: Option<u8>,
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    #[allow(dead_code)]
    struct Input {
        /// Who the price is for.
        audience: Audience,
        /// Products and their discounts.
        items: Vec<Item>,
        /// Shown to people only.
        note: Option<String>,
    }

    #[derive(Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct Open {
        x: Option<Item>,
    }

    #[test]
    fn closes_typical_input() {
        let s = closed_schema_for::<Input>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            s["properties"]["audience"]["enum"],
            serde_json::json!(["everyone", "tier"])
        );
        assert_eq!(s["properties"]["note"]["type"], "string");
        assert_eq!(s["required"], serde_json::json!(["audience", "items"]));
    }

    #[test]
    fn optional_words_do_not_allow_null() {
        let mut v = serde_json::json!({"type": ["string", "null"], "enum": ["vip", "gold", null]});
        close(&mut v);
        assert_eq!(
            v,
            serde_json::json!({"type": "string", "enum": ["vip", "gold"]})
        );
    }

    #[test]
    fn reports_open_objects_and_missing_descriptions() {
        let s = closed_schema_for::<Open>();
        let v = violations(&s);
        assert!(v.iter().any(|m| m.contains("object is open")), "{v:?}");
        assert!(
            v.iter().any(|m| m.contains("$.x: missing description")),
            "{v:?}"
        );
    }
}
