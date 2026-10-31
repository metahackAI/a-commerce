//! Id fields take what people say (docs/design.md, principle 1; table: `acom_core::handle`).
//!
//! The manifest widens such a field's `pattern` and marks it with `x-handle`, so a model, an
//! outside agent, or a strict validator knows `"27184"` is a valid `order_id`. Before a tool runs,
//! [`resolve`] replaces the value with the record's id, so operations only ever see ids.

use acom_contract::{ErrorCode, ToolError};
use acom_core::handle::{self, Handle};
use acom_store::{Cx, StoreError};
use serde_json::{Map, Value, json};

/// The id prefix of a field schema (or of its items) that has a handle.
fn handle_of(prop: &Value) -> Option<&'static Handle> {
    let pattern = prop
        .get("pattern")
        .or_else(|| prop.pointer("/items/pattern"))
        .and_then(Value::as_str)?;
    handle::for_prefix(handle::id_prefix(pattern)?)
}

/// Widens every id field with a handle in a tool's input schema, in place.
pub(crate) fn widen(schema: &mut Value) {
    let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    for prop in props.values_mut() {
        let Some(h) = handle_of(prop) else {
            continue;
        };
        let target = match prop.get_mut("items") {
            Some(items) if items.get("pattern").is_some() => items,
            _ => &mut *prop,
        };
        if let Some(obj) = target.as_object_mut() {
            obj.insert("pattern".into(), json!(handle::widened_pattern(h)));
            obj.insert(
                "x-handle".into(),
                json!({"field": h.field, "shape": h.shape.word()}),
            );
        }
        let Some(obj) = prop.as_object_mut() else {
            continue;
        };
        let also = format!(" Also takes {} ({}).", h.says, h.example);
        let description = obj
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim_end()
            .to_owned();
        obj.insert("description".into(), json!(format!("{description}{also}")));
        if let Some(Value::Array(examples)) = obj.get_mut("examples") {
            let example = if prop_is_list(examples) {
                json!([h.example])
            } else {
                json!(h.example)
            };
            examples.push(example);
        }
    }
}

fn prop_is_list(examples: &[Value]) -> bool {
    examples.first().is_some_and(Value::is_array)
}

/// Replaces values people say with record ids in `input`, for the fields of `schema` that
/// have a handle. Values that are already ids, or that have no handle's shape, are left for the
/// operation's own validation.
pub(crate) async fn resolve(cx: &Cx, schema: &Value, input: &mut Value) -> Result<(), ToolError> {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };
    let Some(fields) = input.as_object_mut() else {
        return Ok(());
    };
    let wanted = pending(props, fields);
    if wanted.is_empty() {
        return Ok(());
    }
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    for (field, index, h, value) in wanted {
        let sql = format!(
            "SELECT id FROM {} WHERE lower({}::text) = $1 LIMIT 2",
            h.table, h.column
        );
        let ids: Vec<(String,)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(&value)
            .fetch_all(&mut *tx.tx)
            .await
            .map_err(|e| ToolError::from(StoreError::from(e)))?;
        let id = match ids.as_slice() {
            [(id,)] => id.clone(),
            [] => {
                return Err(
                    ToolError::new(ErrorCode::NOT_FOUND, format!("{} {value}.", h.none))
                        .detail(json!({"field": field, "value": value, "reason": "no_match"}))
                        .next(h.search_tool),
                );
            }
            _ => {
                return Err(ToolError::invalid_input(format!(
                    "`{value}` matches more than one record; use the id from `{}`.",
                    h.search_tool
                ))
                .detail(json!({"field": field, "value": value, "reason": "ambiguous"}))
                .next(h.search_tool));
            }
        };
        match (fields.get_mut(&field), index) {
            (Some(Value::Array(items)), Some(i)) => items[i] = json!(id),
            (Some(slot), None) => *slot = json!(id),
            _ => {}
        }
    }
    Ok(())
}

/// The values to look up: field, position in a list, handle, and stored form.
fn pending(
    props: &Map<String, Value>,
    fields: &Map<String, Value>,
) -> Vec<(String, Option<usize>, &'static Handle, String)> {
    let mut out = Vec::new();
    for (field, value) in fields {
        let Some(h) = props.get(field).and_then(handle_of) else {
            continue;
        };
        let read = |s: &str| handle::read_value(h.prefix, s).map(|(_, v)| v);
        match value {
            Value::String(s) => {
                if let Some(v) = read(s) {
                    out.push((field.clone(), None, h, v));
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    if let Some(v) = item.as_str().and_then(read) {
                        out.push((field.clone(), Some(i), h, v));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        json!({"type": "object", "additionalProperties": false, "properties": {
            "order_id": {"type": "string", "pattern": "^ord_[a-z0-9_]+$",
                         "description": "The order.", "examples": ["ord_20402"]},
            "change_set_ids": {"type": "array", "items": {"type": "string", "pattern": "^cs_[a-z0-9_]+$"},
                               "description": "Change sets.", "examples": [["cs_1042"]]},
            "product_id": {"type": "string", "pattern": "^prd_[a-z0-9_]+$", "description": "Product."}
        }})
    }

    #[test]
    fn widens_only_fields_with_a_handle() {
        let mut s = schema();
        widen(&mut s);
        let order = &s["properties"]["order_id"];
        assert_eq!(order["pattern"], "^(?:ord_[a-z0-9_]+|#?[0-9]+)$");
        assert_eq!(
            order["x-handle"],
            json!({"field": "order_number", "shape": "number"})
        );
        assert_eq!(
            order["description"],
            "The order. Also takes the order number people see (27184)."
        );
        assert_eq!(order["examples"], json!(["ord_20402", "27184"]));
        let sets = &s["properties"]["change_set_ids"];
        assert_eq!(sets["items"]["pattern"], "^(?:cs_[a-z0-9_]+|#?[0-9]+)$");
        assert_eq!(sets["examples"], json!([["cs_1042"], ["1042"]]));
        assert_eq!(
            s["properties"]["product_id"],
            schema()["properties"]["product_id"]
        );
    }

    #[test]
    fn looks_up_values_people_say_and_leaves_ids() {
        let props = schema()["properties"].as_object().unwrap().clone();
        let fields = json!({"order_id": "#27184", "change_set_ids": ["cs_1042", "1043"],
                            "product_id": "27184"});
        let wanted: Vec<_> = pending(&props, fields.as_object().unwrap())
            .into_iter()
            .map(|(f, i, h, v)| (f, i, h.prefix, v))
            .collect();
        assert_eq!(
            wanted,
            vec![
                ("order_id".to_owned(), None, "ord", "27184".to_owned()),
                (
                    "change_set_ids".to_owned(),
                    Some(1),
                    "cs",
                    "1043".to_owned()
                ),
            ]
        );
        let ids = json!({"order_id": "ord_27184"});
        assert!(pending(&props, ids.as_object().unwrap()).is_empty());
    }
}
