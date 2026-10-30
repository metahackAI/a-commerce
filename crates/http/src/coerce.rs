//! Query strings and path segments arrive as text; the input schema says what each field is.
//! Tool schemas are flat (docs/api/conventions.md §8), so the field's `type` is enough.

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
/// comma-separated values both make arrays. Unknown keys stay text; the operation's closed
/// input type rejects them.
pub fn coerce(schema: &Value, pairs: &[(String, String)]) -> Result<Map<String, Value>, ToolError> {
    let props = schema.get("properties");
    let mut out = Map::new();
    for (key, raw) in pairs {
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
}
