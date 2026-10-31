//! The `enum` resource: every word list with readable labels. The lists themselves are
//! `acom_core::enums::LISTS`, the same ones the database `CHECK`s and the input schemas use.

pub mod list_enums;

use acom_contract::{ErrorClass, ErrorCode};

pub const ENUM_NOT_FOUND: ErrorCode = ErrorCode::new("enum_not_found", ErrorClass::NotFound);

/// Lists whose last values end the life of a record.
const TERMINAL: &[(&str, &[&str])] = &[
    ("order_status", &["returned", "cancelled"]),
    ("availability", &["discontinued"]),
    ("change_set_state", &["failed", "rejected", "undone"]),
    ("preview_state", &["committed", "expired", "superseded"]),
    ("task_state", &["done", "dismissed"]),
];

/// Labels that are not just the word with spaces.
const LABELS: &[(&str, &str)] = &[
    ("problem", "Needs attention"),
    ("needs_you", "Needs you"),
    ("waiting_for_approval", "Waiting for approval"),
    ("you_asked", "You asked"),
    ("agent_noticed", "Agent noticed"),
    ("bo", "Back office"),
    ("fo", "Storefront"),
];

fn label(word: &str) -> String {
    if let Some((_, l)) = LABELS.iter().find(|(w, _)| *w == word) {
        return (*l).to_owned();
    }
    let spaced = word.replace('_', " ");
    let mut c = spaced.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// The tables whose columns use a list, from the data dictionary.
fn applies_to(list: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in crate::schema::describe(&crate::schema::describe_data::DescribeDataInput {
        term: None,
        schema: None,
        table: None,
        api_name: None,
    }) {
        let uses = t["columns"]
            .as_array()
            .is_some_and(|cs| cs.iter().any(|c| c["values_list"] == list));
        if uses && let Some(name) = t["table"].as_str() {
            out.push(name.to_owned());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_read_like_words() {
        assert_eq!(label("in_transit"), "In transit");
        assert_eq!(label("problem"), "Needs attention");
    }
}
