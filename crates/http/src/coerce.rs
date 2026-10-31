//! Query strings and path segments arrive as text; the input schema says what each field is.
//! Tool schemas are flat (docs/api/conventions.md §8), so the field's `type` is enough, except for
//! a money filter, which the query names by its parts (`min_amount.value=200&min_amount.currency=USD`).

use acom_contract::ToolError;
use serde_json::{Map, Number, Value, json};

fn field_error(field: &str, expected: &str, got: &str) -> ToolError {
    ToolError::invalid_input(format!("`{field}` must be {expected}; got `{got}`."))
        .detail(json!({"field": field, "reason": "format"}))
}

fn scalar(field: &str, ty: &str, s: &str) -> Result<Value, ToolError> {
    Ok(match ty {
        "integer" => s
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| field_error(field, "a whole number", s))?,
        "number" => s
            .parse::<Number>()
            .map(Value::Number)
            .map_err(|_| field_error(field, "a number", s))?,
        "boolean" => match s {
            "true" | "1" => Value::Bool(true),
            "false" | "0" => Value::Bool(false),
            _ => return Err(field_error(field, "true or false", s)),
        },
        "object" => serde_json::from_str(s).map_err(|_| field_error(field, "a JSON object", s))?,
        _ => Value::String(s.to_owned()),
    })
}

fn type_of(schema: &Value) -> &str {
    schema
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("string")
}

/// Turns `key=value` pairs into a JSON object shaped by `schema`. Repeated keys and
/// comma-separated values both make arrays; `field.part` sets one part of an object field (a
/// MonetaryAmount's `value` and `currency`). Unknown keys stay text; the operation's closed
/// input type rejects them.
pub fn coerce(schema: &Value, pairs: &[(String, String)]) -> Result<Map<String, Value>, ToolError> {
    let props = schema.get("properties");
    let mut out = Map::new();
    for (key, raw) in pairs {
        if let Some((field, part)) = key.split_once('.') {
            let prop = props.and_then(|p| p.get(field));
            if prop.is_some_and(|p| type_of(p) == "object") {
                let ty = prop
                    .and_then(|p| p.get("properties"))
                    .and_then(|p| p.get(part))
                    .map_or("string", type_of);
                let parts = out
                    .entry(field.to_owned())
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Value::Object(parts) = parts {
                    parts.insert(part.to_owned(), scalar(key, ty, raw)?);
                }
                continue;
            }
        }
        let prop = props.and_then(|p| p.get(key));
        let ty = prop.map_or("string", type_of);
        if ty == "array" {
            let item_ty = prop.and_then(|p| p.get("items")).map_or("string", type_of);
            let list = out
                .entry(key.clone())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(items) = list {
                for part in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    items.push(scalar(key, item_ty, part)?);
                }
            }
        } else {
            out.insert(key.clone(), scalar(key, ty, raw)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_schema() {
        let schema = json!({"type": "object", "properties": {
            "limit": {"type": "integer"}, "in_stock": {"type": "boolean"},
            "max_price": {"type": "number"}, "state": {"type": "array", "items": {"type": "string"}},
            "query": {"type": "string"}}});
        let pairs: Vec<(String, String)> = [
            ("limit", "20"),
            ("in_stock", "true"),
            ("max_price", "89.00"),
            ("state", "live,drafting"),
            ("state", "failed"),
            ("query", "linen, shirt"),
        ]
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect();
        let v = Value::Object(coerce(&schema, &pairs).unwrap());
        assert_eq!(v["limit"], json!(20));
        assert_eq!(v["in_stock"], json!(true));
        assert_eq!(v["max_price"].to_string(), "89.00");
        assert_eq!(v["state"], json!(["live", "drafting", "failed"]));
        assert_eq!(v["query"], json!("linen, shirt"));
        let bad = [("limit".to_owned(), "lots".to_owned())];
        assert!(coerce(&schema, &bad).is_err());
    }

    #[test]
    fn a_money_filter_is_named_by_its_parts() {
        let schema = json!({"type": "object", "properties": {
            "min_amount": {"type": "object", "properties": {
                "value": {"type": "number"}, "currency": {"type": "string"}}},
            "note": {"type": "string"}}});
        let pairs: Vec<(String, String)> = [
            ("min_amount.value", "200"),
            ("min_amount.currency", "USD"),
            ("note.part", "x"),
        ]
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect();
        let v = Value::Object(coerce(&schema, &pairs).unwrap());
        assert_eq!(v["min_amount"], json!({"value": 200, "currency": "USD"}));
        // Not an object field: the key stays as sent, for the closed input type to reject.
        assert_eq!(v["note.part"], json!("x"));
        let bad = [("min_amount.value".to_owned(), "lots".to_owned())];
        assert!(coerce(&schema, &bad).is_err());
    }
}
