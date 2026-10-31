//! What people say instead of an id (docs/design.md, principle 1; docs/api/conventions.md §2).
//!
//! People name an order by its number (27184), a change set by its number (1042), and a customer
//! by email. A field that takes one of these entities' ids also takes that value, and the server
//! finds the record. This table is the one place that says which entities have such a value; the
//! tool manifest, the argument repair, and the agent all read it.
//!
//! Only values that name one record are here. A product name can match several products, so it
//! goes through `search_products` instead.

/// The form of a value people say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Digits, optionally written with a leading `#` (`#27184`).
    Number,
    /// An email address.
    Email,
}

impl Shape {
    /// The word the manifest uses for this shape (`x-handle.shape`).
    pub const fn word(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Email => "email",
        }
    }

    /// The same shape as a regular expression, for the widened `pattern`.
    pub const fn pattern(self) -> &'static str {
        match self {
            Self::Number => "#?[0-9]+",
            Self::Email => "[^@\\s]+@[^@\\s]+",
        }
    }

    /// The value in its stored form when `s` has this shape: `#27184` → `27184`, an email
    /// lowercased. `None` when it does not.
    pub fn read(self, s: &str) -> Option<String> {
        let t = s.trim();
        match self {
            Self::Number => {
                let digits = t.strip_prefix('#').unwrap_or(t);
                (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| digits.to_owned())
            }
            Self::Email => {
                let (local, domain) = t.split_once('@')?;
                let ok = !local.is_empty()
                    && !domain.is_empty()
                    && !domain.contains('@')
                    && !t.chars().any(char::is_whitespace);
                ok.then(|| t.to_lowercase())
            }
        }
    }
}

/// One entity whose id can be replaced by what people say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    /// The id prefix (`ord`).
    pub prefix: &'static str,
    /// The field that holds the value in tool results (`order_number`).
    pub field: &'static str,
    pub shape: Shape,
    /// How the manifest describes it: "the order number people see".
    pub says: &'static str,
    /// The start of the error when it names no record: "No order has the number".
    pub none: &'static str,
    pub example: &'static str,
    /// Where the server looks it up. The column is compared as text.
    pub table: &'static str,
    pub column: &'static str,
    /// The tool to use when the value names no record.
    pub search_tool: &'static str,
}

pub const HANDLES: &[Handle] = &[
    Handle {
        prefix: "ord",
        field: "order_number",
        shape: Shape::Number,
        says: "the order number people see",
        none: "No order has the number",
        example: "27184",
        table: "sales.sales_order",
        column: "order_number",
        search_tool: "search_orders",
    },
    Handle {
        prefix: "cs",
        field: "number",
        shape: Shape::Number,
        says: "the change set number people see",
        none: "No change set has the number",
        example: "1042",
        table: "platform.change_set",
        column: "number",
        search_tool: "search_change_sets",
    },
    Handle {
        prefix: "cus",
        field: "email",
        shape: Shape::Email,
        says: "the customer's email",
        none: "No customer has the email",
        example: "jamie.rivera@example.com",
        table: "customer.person",
        column: "email",
        search_tool: "search_members",
    },
];

/// The handle of the entity with this id prefix.
pub fn for_prefix(prefix: &str) -> Option<&'static Handle> {
    HANDLES.iter().find(|h| h.prefix == prefix)
}

/// The id prefix of an id field's `pattern`: `^ord_[a-z0-9_]+$`, or the widened form
/// `^(?:ord_[a-z0-9_]+|#?[0-9]+)$` the manifest shows ([`widened_pattern`]).
pub fn id_prefix(pattern: &str) -> Option<&str> {
    let body = pattern.strip_prefix('^')?.strip_suffix('$')?;
    let first = match body.strip_prefix("(?:") {
        Some(alts) => alts.strip_suffix(')')?.split('|').next()?,
        None => body,
    };
    let prefix = first.strip_suffix("_[a-z0-9_]+")?;
    (!prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_lowercase())).then_some(prefix)
}

/// The id pattern widened to take the handle too.
pub fn widened_pattern(h: &Handle) -> String {
    format!("^(?:{}_[a-z0-9_]+|{})$", h.prefix, h.shape.pattern())
}

/// The handle a value stands for in a field of this id prefix: `Some((handle, stored form))`
/// when the value is not an id of that prefix but has the handle's shape.
pub fn read_value(prefix: &str, s: &str) -> Option<(&'static Handle, String)> {
    let h = for_prefix(prefix)?;
    if s.trim().starts_with(&format!("{prefix}_")) {
        return None;
    }
    h.shape.read(s).map(|v| (h, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_pattern_forms() {
        assert_eq!(id_prefix("^ord_[a-z0-9_]+$"), Some("ord"));
        let h = for_prefix("ord").unwrap();
        assert_eq!(widened_pattern(h), "^(?:ord_[a-z0-9_]+|#?[0-9]+)$");
        assert_eq!(id_prefix(&widened_pattern(h)), Some("ord"));
        assert_eq!(id_prefix("^[A-Z]{3}$"), None);
        assert_eq!(id_prefix("^[a-z][a-z0-9_]*$"), None);
    }

    #[test]
    fn reads_what_people_say() {
        assert_eq!(Shape::Number.read("#27184"), Some("27184".into()));
        assert_eq!(Shape::Number.read(" 27184 "), Some("27184".into()));
        assert_eq!(Shape::Number.read("order 27184"), None);
        assert_eq!(Shape::Number.read("#"), None);
        assert_eq!(
            Shape::Email.read("Jamie.Rivera@Example.com"),
            Some("jamie.rivera@example.com".into())
        );
        assert_eq!(Shape::Email.read("jamie rivera@example.com"), None);
        assert_eq!(Shape::Email.read("@example.com"), None);
    }

    #[test]
    fn an_id_is_not_a_handle() {
        assert_eq!(read_value("ord", "ord_27184"), None);
        assert_eq!(
            read_value("ord", "27184").map(|(_, v)| v),
            Some("27184".into())
        );
        assert_eq!(read_value("prd", "27184"), None, "products have no handle");
    }

    #[test]
    fn every_handle_has_a_search_tool_and_a_readable_example() {
        for h in HANDLES {
            assert!(h.search_tool.starts_with("search_"), "{}", h.prefix);
            assert!(h.shape.read(h.example).is_some(), "{}", h.prefix);
        }
    }
}
