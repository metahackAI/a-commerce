//! Back-office reads shown as they are: `product_table`, `action_log`, `order_detail`.

use serde_json::{Map, Value, json};

use super::insights::{and_list, upper_first};
use super::shop::items_of;
use super::{Action, Block, SuggestedIntent, words};

/// Columns a table shows at most.
const COLUMNS: usize = 6;

/// Figures read as a table (`get_sales_overview`, `get_sales_trend`): one item per figure
/// (`acom:metric`, `value`, maybe a day and a split), laid out one row per day and split with a
/// column per figure. A split by something named shows its name. With a split and no days, the
/// largest first figure comes first ("which promotion sold most"). Titled by what it shows
/// ("Revenue by observation date and promotion"). `None` when the items are not figures.
fn figures(items: &[Value]) -> Option<(String, Vec<String>, Vec<Value>)> {
    let is_figure =
        |i: &Value| i.get("acom:metric").is_some_and(Value::is_string) && i.get("value").is_some();
    if items.is_empty() || !items.iter().all(is_figure) {
        return None;
    }
    let mut keys: Vec<String> = Vec::new();
    let mut metrics: Vec<String> = Vec::new();
    let mut rows: Vec<(String, Map<String, Value>)> = Vec::new();
    let mut dated = false;
    for i in items {
        let mut at = Map::new();
        if let Some(d) = i.get("observation_date").filter(|d| !d.is_null()) {
            at.insert("observation_date".into(), d.clone());
            dated = true;
        }
        for (k, v) in i
            .get("acom:dimension")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if k == "name" {
                continue;
            }
            // `promotion_id` shows as the promotion's name where the split gives one.
            let shown = i["acom:dimension"]
                .get("name")
                .filter(|n| !n.is_null())
                .unwrap_or(v);
            let key = k.strip_suffix("_id").unwrap_or(k).to_owned();
            at.insert(key, shown.clone());
        }
        let id = Value::Object(at.clone()).to_string();
        let metric = i["acom:metric"].as_str().unwrap_or_default().to_owned();
        if !metrics.contains(&metric) {
            metrics.push(metric.clone());
        }
        for k in at.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        match rows.iter_mut().find(|(r, _)| *r == id) {
            Some((_, row)) => {
                row.insert(metric, i["value"].clone());
            }
            None => {
                at.insert(metric, i["value"].clone());
                rows.push((id, at));
            }
        }
    }
    let first = metrics.first().cloned().unwrap_or_default();
    if !dated && !keys.is_empty() {
        let size = |r: &Map<String, Value>| {
            r.get(&first)
                .map(|v| v.get("value").unwrap_or(v))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
        };
        rows.sort_by(|a, b| size(&b.1).total_cmp(&size(&a.1)));
    }
    let named = |names: &[String]| and_list(&names.iter().map(|n| words(n)).collect::<Vec<_>>());
    let mut title = upper_first(&named(&metrics));
    if !keys.is_empty() {
        title = format!("{title} by {}", named(&keys));
    }
    let mut columns = keys;
    columns.extend(metrics);
    columns.truncate(COLUMNS);
    let rows = rows
        .into_iter()
        .map(|(_, r)| {
            Value::Object(
                columns
                    .iter()
                    .map(|c| (c.clone(), r.get(c).cloned().unwrap_or(Value::Null)))
                    .collect(),
            )
        })
        .collect();
    Some((title, columns, rows))
}

/// A list read as a table: the plain fields the rows share, in first-seen order.
pub fn product_table(output: &Value) -> Option<Block> {
    let items = items_of(output);
    if let Some((title, columns, rows)) = figures(&items) {
        let cols: Vec<Value> = columns
            .iter()
            .map(|c| json!({"key": c, "label": words(c)}))
            .collect();
        let n = rows.len();
        return Some(Block::new(
            "product_table",
            title,
            json!({"columns": cols, "rows": rows, "total": n}),
        ));
    }
    if items.is_empty() {
        // A search that found nothing still answers: an empty table says so.
        return output.get("items").and_then(Value::as_array).map(|_| {
            Block::new(
                "product_table",
                "Nothing found",
                json!({"columns": [], "rows": [], "total": 0}),
            )
        });
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
    fn figures_become_one_row_per_split_with_a_column_per_figure() {
        let money = |v: f64| json!({"value": v, "currency": "USD"});
        let out = json!({"items": [
            {"acom:metric": "revenue", "value": money(105.0), "acom:dimension": {"promotion_id": "promo_birthday", "name": "Birthday coupon"}},
            {"acom:metric": "order_count", "value": 3, "acom:dimension": {"promotion_id": "promo_birthday", "name": "Birthday coupon"}},
            {"acom:metric": "revenue", "value": money(5831.0), "acom:dimension": {"promotion_id": "promo_autumn_knit", "name": "Autumn knitwear 15%"}},
            {"acom:metric": "order_count", "value": 42, "acom:dimension": {"promotion_id": "promo_autumn_knit", "name": "Autumn knitwear 15%"}}
        ], "total": 4});
        let b = product_table(&out).unwrap_or_else(|| panic!("a table"));
        assert_eq!(b.title, "Revenue and order count by promotion");
        assert_eq!(b.data["columns"][0]["key"], "promotion");
        assert_eq!(b.data["rows"].as_array().map(Vec::len), Some(2));
        // The largest first: the knitwear sale before the birthday coupon.
        assert_eq!(b.data["rows"][0]["promotion"], "Autumn knitwear 15%");
        assert_eq!(b.data["rows"][0]["order_count"], 42);
        assert_eq!(b.data["rows"][1]["revenue"]["value"], 105.0);

        // Days stay in their order, one row a day.
        let trend = json!({"items": [
            {"observation_date": "2026-11-04", "acom:metric": "revenue", "value": money(10.0)},
            {"observation_date": "2026-11-05", "acom:metric": "revenue", "value": money(30.0)}
        ]});
        let b = product_table(&trend).unwrap_or_else(|| panic!("a table"));
        assert_eq!(b.title, "Revenue by observation date");
        assert_eq!(b.data["rows"][0]["observation_date"], "2026-11-04");
        assert_eq!(b.data["columns"][1]["key"], "revenue");
    }

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
