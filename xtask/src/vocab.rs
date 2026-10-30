//! Builds `vocab/acom.jsonld`, the machine-readable definition of every `acom:` term.
//!
//! Sources, in order of authority:
//! 1. `vocab/terms.json` — hand-written terms (comments, super-types, terms not stored yet);
//! 2. migration comments — classes from table terms (`schema:Offer/acom:Coupon` → `acom:Coupon`
//!    ⊂ `schema:Offer`) and properties from column terms, with domains and ranges taken from the
//!    tables and column types;
//! 3. `acom_core::enums::LISTS` — enumeration types and members for lists schema.org lacks.
//!
//! So every `acom:` term an agent finds in the database resolves to a definition.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::schema::{Schema, Table};

/// Word lists that are schema.org enumerations, and the schema.org type for each.
pub const SCHEMA_ENUMERATIONS: &[(&str, &str)] = &[
    ("order_status", "schema:OrderStatus"),
    ("payment_status", "schema:PaymentStatusType"),
    ("action_status", "schema:ActionStatusType"),
    ("availability", "schema:ItemAvailability"),
    ("price_type", "schema:PriceTypeEnumeration"),
    ("refund_type", "schema:RefundTypeEnumeration"),
    ("return_fees", "schema:ReturnFeesEnumeration"),
];

/// `claim_status` → `ClaimStatus`.
fn pascal(snake: &str) -> String {
    snake
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}

/// The JSON-LD id of the enumeration that stores a word list.
pub fn enumeration_iri(list: &str) -> String {
    SCHEMA_ENUMERATIONS
        .iter()
        .find(|(l, _)| *l == list)
        .map(|(_, t)| (*t).to_owned())
        .unwrap_or_else(|| format!("acom:{}", pascal(list)))
}

/// The types a table's rows are: its `@type` and, if any, its `additionalType`.
fn table_types(t: &Table) -> (Option<String>, Option<String>) {
    match t.term.split_once('/') {
        Some((schema, acom)) => (Some(schema.to_owned()), Some(acom.to_owned())),
        None if t.term.starts_with("schema:") => (Some(t.term.clone()), None),
        None if t.term.starts_with("acom:") => (None, Some(t.term.clone())),
        None => (None, None),
    }
}

/// The most specific type of a table's rows, used as a property domain or range.
fn table_class(t: &Table) -> Option<String> {
    let (schema, acom) = table_types(t);
    acom.or(schema)
}

fn sql_range(sql_type: &str) -> &'static str {
    let t = sql_type.trim_end_matches("[]");
    if t.starts_with("timestamp") {
        "schema:DateTime"
    } else if t == "date" {
        "schema:Date"
    } else if ["integer", "smallint", "bigint", "int"].contains(&t) {
        "schema:Integer"
    } else if t.starts_with("numeric") || t.starts_with("decimal") {
        "schema:Number"
    } else if t == "boolean" {
        "schema:Boolean"
    } else if t.starts_with("jsonb") || t.starts_with("json") {
        "schema:StructuredValue"
    } else {
        "schema:Text"
    }
}

fn is_lower_camel(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_pascal(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

#[derive(Default)]
struct PropertyFacts {
    domains: BTreeSet<String>,
    ranges: BTreeSet<String>,
    comment: String,
}

/// Returns the generated vocabulary document and the naming problems found on the way.
pub fn generate(
    schema: &Schema,
    terms: &Value,
    lists: &[(&str, &[&str])],
) -> (String, Vec<String>) {
    let mut problems = Vec::new();
    let mut nodes: BTreeMap<String, Value> = BTreeMap::new();
    for node in terms["@graph"].as_array().into_iter().flatten() {
        if let Some(id) = node["@id"].as_str() {
            nodes.insert(id.to_owned(), node.clone());
        }
    }

    // Classes from table terms.
    for t in schema.tables.values() {
        let (base, acom) = table_types(t);
        let Some(acom) = acom else { continue };
        let name = acom.trim_start_matches("acom:");
        if !is_pascal(name) {
            problems.push(format!(
                "{}.{}: class term `{acom}` must be PascalCase",
                t.schema, t.table
            ));
        }
        nodes.entry(acom.clone()).or_insert_with(|| {
            json!({
                "@id": acom,
                "@type": "rdfs:Class",
                "rdfs:label": name,
                "rdfs:subClassOf": {"@id": base.clone().unwrap_or_else(|| "schema:Thing".to_owned())},
                "rdfs:comment": t.description,
            })
        });
    }

    // Properties from column terms.
    let mut facts: BTreeMap<String, PropertyFacts> = BTreeMap::new();
    for t in schema.tables.values() {
        let domain = table_class(t);
        for c in &t.columns {
            if !c.term.starts_with("acom:") {
                continue;
            }
            let name = c.term.trim_start_matches("acom:");
            if !is_lower_camel(name) {
                problems.push(format!(
                    "{}.{}.{}: property term `{}` must be lowerCamelCase",
                    t.schema, t.table, c.name, c.term
                ));
                continue;
            }
            let f = facts.entry(c.term.clone()).or_default();
            if let Some(d) = &domain {
                f.domains.insert(d.clone());
            }
            let range = if let Some(list) = &c.values_list {
                enumeration_iri(list)
            } else if let Some(target) = c.references.as_deref().and_then(|r| schema.tables.get(r))
            {
                table_class(target).unwrap_or_else(|| "schema:Text".to_owned())
            } else if c.name.ends_with("_value") || c.name.ends_with("_currency") {
                let base = c
                    .name
                    .trim_end_matches("_value")
                    .trim_end_matches("_currency");
                if t.columns
                    .iter()
                    .any(|x| x.name == format!("{base}_currency"))
                {
                    "schema:MonetaryAmount".to_owned()
                } else {
                    "schema:QuantitativeValue".to_owned()
                }
            } else {
                sql_range(&c.sql_type).to_owned()
            };
            f.ranges.insert(range);
            if f.comment.is_empty() || c.description.len() < f.comment.len() {
                f.comment = c.description.clone();
            }
        }
    }
    for (iri, f) in facts {
        let ids =
            |set: &BTreeSet<String>| set.iter().map(|d| json!({"@id": d})).collect::<Vec<_>>();
        match nodes.get_mut(&iri) {
            Some(existing) => {
                // Hand-written definitions win; add any domain the database also uses it on.
                let mut domains: BTreeSet<String> = existing["schema:domainIncludes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v["@id"].as_str().map(str::to_owned))
                    .collect();
                domains.extend(f.domains);
                existing["schema:domainIncludes"] = Value::Array(ids(&domains));
            }
            None => {
                let label = iri.trim_start_matches("acom:").to_owned();
                nodes.insert(
                    iri.clone(),
                    json!({
                        "@id": iri,
                        "@type": "rdf:Property",
                        "rdfs:label": label,
                        "schema:domainIncludes": ids(&f.domains),
                        "schema:rangeIncludes": ids(&f.ranges),
                        "rdfs:comment": f.comment,
                    }),
                );
            }
        }
    }

    // Enumerations from the word lists.
    for (list, words) in lists {
        if SCHEMA_ENUMERATIONS.iter().any(|(l, _)| l == list) {
            continue;
        }
        let type_iri = enumeration_iri(list);
        let type_name = type_iri.trim_start_matches("acom:").to_owned();
        nodes.insert(
            type_iri.clone(),
            json!({
                "@id": type_iri,
                "@type": "rdfs:Class",
                "rdfs:label": type_name,
                "rdfs:subClassOf": {"@id": "schema:Enumeration"},
                "rdfs:comment": format!("Values of the {list} list (docs/api/enums.md)."),
                "acom:wireList": list,
            }),
        );
        for (i, w) in words.iter().enumerate() {
            let member = format!("{type_iri}{}", pascal(w));
            nodes.insert(
                member.clone(),
                json!({
                    "@id": member,
                    "@type": type_iri,
                    "rdfs:label": pascal(w),
                    "acom:wireWord": w,
                    "schema:position": i + 1,
                }),
            );
        }
    }

    // Classes first, then properties, then enumerations; members follow their list's order.
    let rank = |n: &Value| match n["@type"].as_str() {
        Some("rdfs:Class") if n.get("acom:wireList").is_none() => 0,
        Some("rdf:Property") => 1,
        Some("rdfs:Class") => 2,
        _ => 3,
    };
    let mut graph: Vec<Value> = nodes.into_values().collect();
    let key = |n: &Value| {
        let group = if rank(n) == 3 {
            n["@type"].as_str().unwrap_or("").to_owned()
        } else {
            n["@id"].as_str().unwrap_or("").to_owned()
        };
        (rank(n), group, n["schema:position"].as_u64().unwrap_or(0))
    };
    graph.sort_by_key(key);
    let mut doc = Map::new();
    doc.insert("@context".into(), terms["@context"].clone());
    doc.insert("@id".into(), "acom:".into());
    doc.insert(
        "rdfs:comment".into(),
        "A-Commerce extension vocabulary. Generated by `cargo xtask check-schema --write` from vocab/terms.json, migration comments, and crates/core/src/enums.rs; do not edit. Rules: docs/api/vocabulary.md."
            .into(),
    );
    doc.insert("@graph".into(), Value::Array(graph));
    let text = serde_json::to_string_pretty(&Value::Object(doc)).unwrap_or_default() + "\n";
    (text, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_enumerations_after_lists() {
        assert_eq!(enumeration_iri("claim_status"), "acom:ClaimStatus");
        assert_eq!(enumeration_iri("order_status"), "schema:OrderStatus");
        assert_eq!(pascal("true_to_size"), "TrueToSize");
    }
}
