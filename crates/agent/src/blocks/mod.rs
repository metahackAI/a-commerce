//! UI blocks (docs/api/chat.md §3): the agent never writes HTML; it picks blocks from a fixed
//! library and fills them from tool output. The model chooses which block and short wording;
//! every number in `data` comes from a tool result, never from the model.
//!
//! Elements inside a block are named `<kind>:<id>` (`change_row:prd_windbreaker`); the web
//! prefixes the block id to make the `element_ref`.

pub mod change_set;
pub mod insights;
pub mod shop;
pub mod table;
pub mod tasks;

use std::str::FromStr;

use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Value, json};

/// Blocks this runtime can build. Agent definitions may only list these.
pub const KNOWN_BLOCKS: &[&str] = &[
    "change_set",
    "product_table",
    "action_log",
    "order_detail",
    "compare_table",
    "look_gallery",
    "product_list",
    "cart",
    "checkout_card",
    "return_options",
    "memory_note",
    "sales_explainer",
    "task_list",
];

/// Block data format version (`schema`).
pub const SCHEMA: u32 = 1;

/// One block.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Block {
    pub block_id: String,
    pub ui: String,
    pub schema: u32,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    pub data: Value,
    pub actions: Vec<Action>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suggested_intents: Vec<SuggestedIntent>,
}

impl Block {
    pub fn new(ui: &str, title: impl Into<String>, data: Value) -> Self {
        Self {
            block_id: format!("blk_{}", acom_core::id::new_suffix()),
            ui: ui.to_owned(),
            schema: SCHEMA,
            title: title.into(),
            why: None,
            data,
            actions: Vec::new(),
            suggested_intents: Vec::new(),
        }
    }

    /// `<block_id>#<element>`, for `ui.command` targets.
    pub fn element_ref(&self, element: &str) -> String {
        format!("{}#{element}", self.block_id)
    }

    /// Person buttons in this block; no screen command may target them.
    pub fn person_actions(&self) -> impl Iterator<Item = &Action> {
        self.actions
            .iter()
            .filter(|a| matches!(a, Action::Person { .. }))
    }
}

/// A button (docs/api/chat.md §3.2). Only `person` buttons change state, and the web calls
/// them directly with the person's session; the agent never calls those endpoints.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Comes back to the agent as a `ui_event`.
    Say {
        action_id: String,
        label: String,
        args: Value,
    },
    /// A person-only endpoint the web calls with the person's session and CSRF token.
    Person {
        action_id: String,
        label: String,
        method: String,
        path: String,
        body: Value,
        confirm: String,
    },
    /// Opens another block or a page.
    Open {
        action_id: String,
        label: String,
        target: String,
    },
}

impl Action {
    pub fn say(id: &str, label: impl Into<String>, args: Value) -> Self {
        Self::Say {
            action_id: id.to_owned(),
            label: label.into(),
            args,
        }
    }

    pub fn person(
        id: &str,
        label: impl Into<String>,
        path: impl Into<String>,
        body: Value,
        confirm: impl Into<String>,
    ) -> Self {
        Self::Person {
            action_id: id.to_owned(),
            label: label.into(),
            method: "POST".into(),
            path: path.into(),
            body,
            confirm: confirm.into(),
        }
    }

    pub fn open(id: &str, label: impl Into<String>, target: impl Into<String>) -> Self {
        Self::Open {
            action_id: id.to_owned(),
            label: label.into(),
            target: target.into(),
        }
    }
}

/// Something a person may say about one element, shown next to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuggestedIntent {
    pub element: String,
    pub say: String,
}

impl SuggestedIntent {
    pub fn new(element: impl Into<String>, say: impl Into<String>) -> Self {
        Self {
            element: element.into(),
            say: say.into(),
        }
    }
}

/// `{"value": 71.2, "currency": "USD"}` (or `price` + `price_currency`) as "$71.20". Only
/// formats an amount a tool returned; never computes one.
pub fn money_label(v: &Value) -> Option<String> {
    let (value, currency) = match v {
        Value::Object(o) if o.contains_key("value") => (
            o.get("value")?,
            o.get("currency").and_then(Value::as_str).unwrap_or(""),
        ),
        Value::Object(o) if o.contains_key("price") => (
            o.get("price")?,
            o.get("price_currency")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
        _ => return None,
    };
    let text = match value {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    let amount = Decimal::from_str(&text).ok()?.round_dp(2);
    let digits = format!("{amount:.2}");
    Some(match currency {
        "USD" | "" => format!("${digits}"),
        c => format!("{digits} {c}"),
    })
}

/// A product's price as a money object: `price` when it is one, or `price` + `price_currency`.
pub fn price_of(item: &Value, key: &str) -> Value {
    match item.get(key) {
        Some(p @ Value::Object(_)) => p.clone(),
        Some(p @ (Value::Number(_) | Value::String(_))) => {
            let currency = item
                .get(format!("{key}_currency"))
                .or_else(|| item.get("price_currency"))
                .cloned()
                .unwrap_or(Value::Null);
            json!({"value": p, "currency": currency})
        }
        _ => Value::Null,
    }
}

/// `snake_case` as words: `price_floor` → "price floor".
pub fn words(s: &str) -> String {
    s.replace(['_', '.'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_labels_format_tool_amounts_exactly() {
        let v: Value = serde_json::from_str(r#"{"value": 71.2, "currency": "USD"}"#).unwrap();
        assert_eq!(money_label(&v).as_deref(), Some("$71.20"));
        let v = json!({"price": "1284", "price_currency": "EUR"});
        assert_eq!(money_label(&v).as_deref(), Some("1284.00 EUR"));
        assert_eq!(money_label(&json!(5)), None);
        let item = json!({"price": 89.0, "price_currency": "USD"});
        assert_eq!(price_of(&item, "price")["currency"], "USD");
    }

    #[test]
    fn blocks_serialize_in_the_documented_shape() {
        let mut b = Block::new("memory_note", "Remembered", json!({"memory_id": "mem_1"}));
        b.actions.push(Action::person(
            "undo",
            "Undo",
            "/v1/customer/me/memories/mem_1/undo",
            json!({}),
            "Forget this?",
        ));
        let v = serde_json::to_value(&b).unwrap();
        assert!(v["block_id"].as_str().unwrap().starts_with("blk_"));
        assert_eq!(v["schema"], 1);
        assert_eq!(v["actions"][0]["kind"], "person");
        assert_eq!(v["actions"][0]["method"], "POST");
        assert!(v.get("why").is_none() && v.get("suggested_intents").is_none());
        assert_eq!(b.person_actions().count(), 1);
        assert_eq!(
            b.element_ref("memory_note:mem_1"),
            format!("{}#memory_note:mem_1", b.block_id)
        );
    }
}
