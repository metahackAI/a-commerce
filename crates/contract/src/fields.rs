//! The `fields` input of read tools (docs/api/conventions.md §6): which top-level fields of an
//! entity come back. One rule for every tool: `id` always stays, and all fields come back when
//! `fields` is empty.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::ToolError;

/// Keeps only the named top-level fields of an entity. A name the entity does not have is
/// `invalid_input` naming the ones it has: a guessed name would otherwise come back as an empty
/// object, which reads as "there is no data".
pub fn keep(v: Value, fields: &[String]) -> Result<Value, ToolError> {
    known(std::slice::from_ref(&v), fields)?;
    Ok(pick(v, fields))
}

/// [`keep`] for each entity of a list. A name is known when some entity has it; an empty list has
/// nothing to check against.
pub fn keep_each(items: Vec<Value>, fields: &[String]) -> Result<Vec<Value>, ToolError> {
    known(&items, fields)?;
    Ok(items.into_iter().map(|v| pick(v, fields)).collect())
}

/// Keeps only the named top-level fields, without checking the names. For inputs whose schema
/// already lists the names a caller may give.
pub fn pick(v: Value, fields: &[String]) -> Value {
    match v {
        Value::Object(m) if !fields.is_empty() => Value::Object(
            m.into_iter()
                .filter(|(k, _)| k == "id" || fields.iter().any(|f| f == k))
                .collect(),
        ),
        other => other,
    }
}

fn known(items: &[Value], fields: &[String]) -> Result<(), ToolError> {
    let has: BTreeSet<&str> = items
        .iter()
        .filter_map(Value::as_object)
        .flat_map(|m| m.keys().map(String::as_str))
        .collect();
    if has.is_empty() {
        return Ok(());
    }
    let unknown: Vec<&str> = fields
        .iter()
        .map(String::as_str)
        .filter(|f| !has.contains(f))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let has: Vec<&str> = has.into_iter().collect();
    Err(ToolError::invalid_input(format!(
        "`fields` names {} this result does not have. Its top-level fields are {}. Name some of \
         those, or leave `fields` out to get all of them.",
        unknown
            .iter()
            .map(|f| format!("`{f}`"))
            .collect::<Vec<_>>()
            .join(", "),
        has.join(", "),
    ))
    .detail(json!({"field": "fields", "unknown": unknown, "fields": has})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn keeps_the_named_fields_and_the_id() {
        let v = json!({"id": "prd_windbreaker", "name": "Packable windbreaker", "price": 89.00});
        assert_eq!(
            keep(v.clone(), &names(&["price"])).unwrap_or_else(|e| panic!("{e}")),
            json!({"id": "prd_windbreaker", "price": 89.00})
        );
        assert_eq!(keep(v.clone(), &[]).unwrap_or_else(|e| panic!("{e}")), v);
    }

    #[test]
    fn a_name_the_entity_does_not_have_says_which_it_has() {
        let v = json!({"product_id": "prd_hiking_socks", "variants": [], "total": 78});
        let e = keep(v, &names(&["variant_id[]", "total"]))
            .err()
            .unwrap_or_else(|| panic!("an unknown name must not narrow to nothing"));
        assert_eq!(e.code, "invalid_input");
        assert!(e.message.contains("`variant_id[]`"), "{}", e.message);
        assert!(
            e.message.contains("product_id, total, variants"),
            "{}",
            e.message
        );
        let d = e.detail.unwrap_or_else(|| panic!("detail"));
        assert_eq!(d["unknown"], json!(["variant_id[]"]));
    }

    #[test]
    fn a_list_knows_a_name_any_of_its_entities_has() {
        let items = vec![
            json!({"id": "a", "note": "x"}),
            json!({"id": "b", "photo": "p.jpg"}),
        ];
        let kept = keep_each(items, &names(&["photo"])).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            kept,
            vec![json!({"id": "a"}), json!({"id": "b", "photo": "p.jpg"})]
        );
        assert!(keep_each(Vec::new(), &names(&["anything"])).is_ok());
        assert!(keep_each(vec![json!({"id": "a"})], &names(&["nope"])).is_err());
    }
}
