//! Back-office reads shown as they are: `product_table`, `action_log`, `order_detail`.

use serde_json::{Map, Value, json};

use super::shop::items_of;
use super::{Action, Block, SuggestedIntent, words};

/// Columns a table shows at most.
const COLUMNS: usize = 6;

/// A list read as a table: the plain fields the rows share, in first-seen order.
pub fn product_table(output: &Value) -> Option<Block> {
    let items = items_of(output);
    if items.is_empty() {
        return None;
    }
    let mut columns: Vec<String> = Vec::new();
    for i in &items {
        for (k, v) in i.as_object().into_iter().flatten() {
            let shown = matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_))
                || v.get("value").is_some();
            if shown && !columns.contains(k) && columns.len() < COLUMNS {
                columns.push(k.clone());
            }
        }
    }
    let rows: Vec<Value> = items
        .iter()
        .map(|i| {
            let row: Map<String, Value> = columns
                .iter()
                .map(|c| (c.clone(), i.get(c).cloned().unwrap_or(Value::Null)))
                .collect();
            Value::Object(row)
        })
        .collect();
    let total = output
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or(items.len() as u64);
    let cols: Vec<Value> = columns
        .iter()
        .map(|c| json!({"key": c, "label": words(c)}))
        .collect();
    let mut b = Block::new(
        "product_table",
        format!("{} of {total}", rows.len()),
        json!({"columns": cols, "rows": rows, "total": total}),
    );
    if total > items.len() as u64 {
        b.actions.push(Action::say("more", "Show more", json!({})));
    }
    for i in &items {
        if let Some(id) = i
            .get("product_id")
            .or_else(|| i.get("id"))
            .and_then(Value::as_str)
        {
            b.suggested_intents.push(SuggestedIntent::new(
                format!("row:{id}"),
                "Tell me about this one",
            ));
        }
    }
    Some(b)
}

/// Action log entries (`search_action_log`).
pub fn action_log(output: &Value) -> Option<Block> {
    let items = items_of(output);
    if items.is_empty() {
        return None;
    }
    let field = |i: &Value, keys: &[&str]| {
        keys.iter()
            .find_map(|k| i.get(*k).filter(|v| !v.is_null()).cloned())
            .unwrap_or(Value::Null)
    };
    let entries: Vec<Value> = items
        .iter()
        .map(|i| {
            json!({
                "at": field(i, &["at", "start_time", "time"]),
                "actor": field(i, &["actor", "agent_name", "agent_id"]),
                "action_type": field(i, &["action_type"]),
                "tool": field(i, &["tool", "instrument"]),
                "object": field(i, &["object", "object_id"]),
                "status": field(i, &["status", "action_status"]),
                "result": field(i, &["result"]),
            })
        })
        .collect();
    Some(Block::new(
        "action_log",
        format!("{} actions", entries.len()),
        json!({"entries": entries}),
    ))
}

/// One order (`get_order`).
pub fn order_detail(output: &Value) -> Option<Block> {
    let number = output.get("order_number")?;
    let mut data = Map::new();
    data.insert(
        "order_id".to_owned(),
        output.get("id").cloned().unwrap_or(Value::Null),
    );
    for k in ["order_number", "delivery", "payment", "totals"] {
        data.insert(k.to_owned(), output.get(k).cloned().unwrap_or(Value::Null));
    }
    // get_order says `order_status` and `ordered_item`; the block's `status` and `lines` are those,
    // each line the product it was, with the ordered color's picture.
    let status = output.get("order_status").or_else(|| output.get("status"));
    data.insert("status".to_owned(), status.cloned().unwrap_or(Value::Null));
    let lines: Vec<Value> = output
        .get("ordered_item")
        .or_else(|| output.get("lines"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|l| {
            json!({"order_item_id": l.get("id"), "product_id": l.get("product_id"),
                   "variant_id": l.get("variant_id"), "name": l.get("name"), "size": l.get("size"),
                   "color": l.get("color"), "image": l.get("image"),
                   "quantity": l.get("order_quantity").or_else(|| l.get("quantity")),
                   "price": l.get("price"),
                   "status": l.get("order_item_status").or_else(|| l.get("status"))})
        })
        .collect();
    data.insert("lines".to_owned(), Value::Array(lines));
    let title = format!(
        "Order #{}",
        number
            .as_str()
            .map_or_else(|| number.to_string(), str::to_owned)
    );
    let mut b = Block::new("order_detail", title, Value::Object(data));
    b.actions.push(Action::say(
        "return",
        "Ask about a return or exchange",
        json!({"order_number": number}),
    ));
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_show_shared_plain_fields() {
        let out = json!({"items": [
            {"product_id": "prd_chore_coat", "name": "Cotton chore coat", "inventory_level": 7, "variants": [1, 2]},
            {"product_id": "prd_rain_parka", "name": "Light rain parka", "inventory_level": 4,
             "price": {"value": 118.00, "currency": "USD"}}
        ], "total": 42});
        let b = product_table(&out).unwrap();
        assert_eq!(b.title, "2 of 42");
        let keys: Vec<&str> = b.data["columns"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["key"].as_str())
            .collect();
        assert_eq!(keys, ["product_id", "name", "inventory_level", "price"]);
        assert_eq!(b.data["rows"][0]["price"], Value::Null);
        assert!(matches!(&b.actions[0], Action::Say { action_id, .. } if action_id == "more"));
    }

    #[test]
    fn logs_and_orders() {
        let log = action_log(
            &json!({"items": [{"start_time": "2026-11-06T08:31:00-05:00",
            "agent_id": "sys_runner", "action_type": "UpdateAction", "instrument": "set_price",
            "action_status": "completed"}]}),
        )
        .unwrap();
        assert_eq!(log.data["entries"][0]["tool"], "set_price");
        assert_eq!(log.data["entries"][0]["actor"], "sys_runner");
        let order = order_detail(&json!({"order_number": "20402", "status": "delivered"})).unwrap();
        // What get_order answers: its lines and status become the block's.
        let read = order_detail(&json!({"id": "ord_20402", "order_number": "20402", "order_status": "delivered",
            "ordered_item": [{"id": "oln_20402_1", "product_id": "prd_chelsea_boots", "variant_id": "var_chelsea_boots_9_tan",
                "name": "Leather chelsea boots", "size": "9", "color": "Tan", "image": "/media/products/boots.webp",
                "order_quantity": 1, "price": {"value": 312.0, "currency": "USD"}, "order_item_status": "delivered"}]}))
        .unwrap();
        assert_eq!(read.data["status"], "delivered");
        assert_eq!(read.data["order_id"], "ord_20402");
        assert_eq!(read.data["lines"][0]["name"], "Leather chelsea boots");
        assert_eq!(read.data["lines"][0]["image"], "/media/products/boots.webp");
        assert_eq!(read.data["lines"][0]["quantity"], 1);
        assert_eq!(order.title, "Order #20402");
        assert!(order_detail(&json!({})).is_none());
    }
}
