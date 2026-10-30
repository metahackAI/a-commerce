//! Storefront blocks: product results in three layouts, the cart, the order confirmation card,
//! return options, and remembered preferences. Data is copied from tool output; prices and
//! totals are never computed here.

use acom_contract::Preview;
use serde_json::{Map, Value, json};

use super::{Action, Block, SuggestedIntent, money_label, price_of, words};
use crate::profile::LayoutRule;

/// Fields that are not "facts" about a product.
const NOT_FACTS: &[&str] = &[
    "id",
    "product_id",
    "name",
    "price",
    "price_currency",
    "list_price",
    "list_price_currency",
    "image",
    "image_url",
    "images",
    "url",
    "description",
    "caption",
    "facts",
];

/// Items of a list read (`{items: [...]}`, or a screen grid's `{list: [...]}`) or a bare list.
pub fn items_of(output: &Value) -> Vec<Value> {
    output
        .get("items")
        .or_else(|| output.get("products"))
        // A screen-shaped grid (the evaluation's facade) answers its rows as `list`.
        .or_else(|| output.get("list"))
        .and_then(Value::as_array)
        .or_else(|| output.as_array())
        .cloned()
        .unwrap_or_default()
}

fn product_id(item: &Value) -> String {
    item.get("product_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn name(item: &Value) -> String {
    item.get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn image(item: &Value) -> Value {
    item.get("image")
        .or_else(|| item.get("image_url"))
        .or_else(|| item.pointer("/images/0"))
        .cloned()
        .unwrap_or(Value::Null)
}

/// Short `label: value` facts from an item's plain fields, or its own `facts` list.
fn facts(item: &Value, max: usize) -> Vec<Value> {
    if let Some(Value::Array(f)) = item.get("facts") {
        return f.iter().take(max).cloned().collect();
    }
    item.as_object()
        .map(|o| {
            o.iter()
                .filter(|(k, v)| {
                    !NOT_FACTS.contains(&k.as_str())
                        && !k.ends_with("_id")
                        && matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_))
                })
                .take(max)
                .map(|(k, v)| {
                    let text = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
                    json!(format!("{}: {text}", words(k)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a profile fact matches a rule's `when_profile` entry: a fact key (`compares_specs`),
/// or `key=words` for a remembered value (`layout_preference=photos` matches the memory
/// `layout_preference=photos over details`). Facts are `key` and `key=value`.
fn profile_matches(pattern: &str, fact: &str) -> bool {
    match pattern.split_once('=') {
        None => fact == pattern,
        Some((key, words)) => fact.split_once('=').is_some_and(|(k, v)| {
            k == key && v.to_lowercase().contains(&words.trim().to_lowercase())
        }),
    }
}

/// Which layout shows product results (docs/api/chat.md §3.4): what the shopper says now,
/// then their profile, then a first visit, then the rule with no condition.
pub fn layout<'a>(
    rules: &'a [LayoutRule],
    said: &str,
    profile_facts: &[String],
    first_visit: bool,
) -> Option<&'a LayoutRule> {
    let said = said.to_lowercase();
    rules
        .iter()
        .find(|r| r.when_said.iter().any(|p| said.contains(&p.to_lowercase())))
        .or_else(|| {
            rules.iter().find(|r| {
                r.when_profile
                    .iter()
                    .any(|p| profile_facts.iter().any(|f| profile_matches(p, f)))
            })
        })
        .or_else(|| rules.iter().find(|r| first_visit && r.first_visit))
        .or_else(|| {
            rules
                .iter()
                .find(|r| r.when_said.is_empty() && r.when_profile.is_empty() && !r.first_visit)
        })
}

/// The products an earlier results block showed, as rows to lay out again (F6: the same
/// results, another layout). A comparison keeps its prices beside its products.
pub fn results_of(data: &Value) -> Value {
    let items: Vec<Value> = match (data.get("products"), data.get("prices")) {
        (Some(Value::Array(products)), prices) => products
            .iter()
            .enumerate()
            .map(|(n, p)| {
                let mut row = p.clone();
                if let Some(price) = prices.and_then(|ps| ps.get(n))
                    && let Some(o) = row.as_object_mut()
                {
                    for k in ["price", "list_price"] {
                        o.insert(k.into(), price.get(k).cloned().unwrap_or(Value::Null));
                    }
                }
                row
            })
            .collect(),
        // A gift picker's picks are products too.
        _ => match data.get("picks") {
            Some(Value::Array(picks)) => picks.clone(),
            _ => items_of(data),
        },
    };
    json!({"items": items, "total": items.len()})
}

/// Product results as `compare_table`, `look_gallery`, or `product_list`.
pub fn products(ui: &str, output: &Value, why: Option<&str>) -> Option<Block> {
    let items = items_of(output);
    if items.is_empty() {
        return None;
    }
    let shown: Vec<&Value> = items.iter().take(6).collect();
    let data = match ui {
        "compare_table" => compare_data(output, &shown),
        "look_gallery" => json!({"items": shown.iter().map(|i| json!({
            "product_id": product_id(i), "name": name(i), "price": price_of(i, "price"),
            "list_price": price_of(i, "list_price"), "image": image(i),
            "caption": i.get("caption").or_else(|| i.get("summary")).cloned().unwrap_or(Value::Null),
        })).collect::<Vec<_>>()}),
        "product_list" => json!({"items": shown.iter().map(|i| json!({
            "product_id": product_id(i), "name": name(i), "price": price_of(i, "price"),
            "list_price": price_of(i, "list_price"), "image": image(i),
            "facts": facts(i, 4),
        })).collect::<Vec<_>>()}),
        _ => return None,
    };
    let total = output
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or(items.len() as u64);
    let title = format!("{} of {total} shown", shown.len());
    let mut b = Block::new(ui, title, data);
    b.why = why.map(str::to_owned);
    if ui != "compare_table" {
        b.actions.push(Action::say(
            "compare",
            "Compare them",
            json!({"layout": "compare_table"}),
        ));
    }
    if ui != "look_gallery" {
        b.actions.push(Action::say(
            "photos",
            "Show photos",
            json!({"layout": "look_gallery"}),
        ));
    }
    for i in &shown {
        let id = product_id(i);
        if id.is_empty() {
            continue;
        }
        let element = if ui == "compare_table" {
            format!("compare_column:{id}")
        } else {
            format!("product_card:{id}")
        };
        b.suggested_intents
            .push(SuggestedIntent::new(element, "Tell me more about this one"));
    }
    Some(b)
}

/// A comparison: the tool's own rows when it has them, else the fields the items share.
fn compare_data(output: &Value, shown: &[&Value]) -> Value {
    let products: Vec<Value> = shown
        .iter()
        .map(|i| {
            json!({"product_id": product_id(i), "name": name(i),
                   "color": i.get("color").cloned().unwrap_or(Value::Null),
                   "material": i.get("material").cloned().unwrap_or(Value::Null),
                   "badge": i.get("badge").cloned().unwrap_or(Value::Null)})
        })
        .collect();
    let rows = match output.get("rows") {
        Some(r @ Value::Array(_)) => r.clone(),
        _ => {
            let mut keys: Vec<String> = Vec::new();
            for i in shown {
                for (k, v) in i.as_object().into_iter().flatten() {
                    let plain = matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_));
                    let skip = NOT_FACTS.contains(&k.as_str())
                        || k.ends_with("_id")
                        || ["color", "material", "badge"].contains(&k.as_str());
                    if plain && !skip && !keys.contains(k) {
                        keys.push(k.clone());
                    }
                }
            }
            Value::Array(
                keys.iter()
                    .take(8)
                    .map(|k| {
                        json!({"label": words(k),
                               "values": shown.iter().map(|i| i.get(k).cloned().unwrap_or(Value::Null)).collect::<Vec<_>>()})
                    })
                    .collect(),
            )
        }
    };
    let prices: Vec<Value> = shown
        .iter()
        .map(|i| {
            json!({"price": price_of(i, "price"), "list_price": price_of(i, "list_price"),
                         "label": i.get("price_label").cloned().unwrap_or(Value::Null)})
        })
        .collect();
    json!({"products": products, "rows": rows, "prices": prices,
           "sorted_by": output.get("sorted_by").cloned().unwrap_or(Value::Null)})
}

const CART_FIELDS: &[&str] = &[
    "lines",
    "subtotal",
    "discounts",
    "shipping",
    "total",
    "delivery",
    "payment",
];

fn pick(from: &Value, fields: &[&str]) -> Map<String, Value> {
    fields
        .iter()
        .map(|f| {
            (
                (*f).to_owned(),
                from.get(*f).cloned().unwrap_or(Value::Null),
            )
        })
        .collect()
}

/// The cart, from `get_cart`'s output.
pub fn cart(output: &Value) -> Option<Block> {
    output.get("lines")?;
    let data = Value::Object(pick(output, CART_FIELDS));
    let title = match output.get("total").and_then(money_label) {
        Some(t) => format!("Your cart · {t}"),
        None => "Your cart".to_owned(),
    };
    let mut b = Block::new("cart", title, data);
    b.actions.push(Action::say(
        "checkout",
        "Check out",
        json!({"intent": "prepare_order"}),
    ));
    for line in output["lines"].as_array().into_iter().flatten() {
        let id = line
            .get("cart_line_id")
            .or_else(|| line.get("id"))
            .or_else(|| line.get("product_id"))
            .and_then(Value::as_str);
        if let Some(id) = id {
            b.suggested_intents.push(SuggestedIntent::new(
                format!("cart_line:{id}"),
                "Remove this",
            ));
        }
    }
    Some(b)
}

/// The order confirmation card from a prepared order's preview. Only the shopper places it.
pub fn checkout_card(preview: &Preview) -> Block {
    let order = preview
        .changes
        .iter()
        .map(|c| &c.to)
        .find(|to| to.get("total").is_some())
        .cloned()
        .unwrap_or(Value::Null);
    let mut data = pick(&order, CART_FIELDS);
    // A gift's card also shows the wrap, the card and receipt, and the recipient.
    for f in ["order_type", "gift_wrap", "gift_options", "recipient"] {
        if let Some(v) = order.get(f).filter(|v| !v.is_null()) {
            data.insert(f.into(), v.clone());
        }
    }
    data.insert("preview_id".into(), json!(preview.preview_id));
    data.insert("expires_at".into(), json!(preview.expires_at));
    data.insert("checks".into(), json!(preview.checks));
    data.insert("note".into(), json!("Only you can place it."));
    let total = order.get("total").and_then(money_label);
    let mut b = Block::new(
        "checkout_card",
        total
            .as_ref()
            .map_or_else(|| "Your order".to_owned(), |t| format!("Your order · {t}")),
        Value::Object(data),
    );
    let ready = preview.checks.iter().all(|c| c.passed);
    if ready {
        let label = total.as_ref().map_or_else(
            || "Place order".to_owned(),
            |t| format!("Place order · {t}"),
        );
        let confirm = total.as_ref().map_or_else(
            || "Place this order?".to_owned(),
            |t| format!("Place this order for {t}?"),
        );
        b.actions.push(Action::person(
            "place_order",
            label,
            "/v1/sales/orders",
            json!({"preview_id": preview.preview_id}),
            confirm,
        ));
    } else {
        b.why = Some("A check failed; fix it before the order can be placed.".into());
    }
    b
}

/// Return options from `get_return_options`, with the input it was read with. Sending the
/// request is the shopper's button.
pub fn return_options(output: &Value, input: &Value) -> Option<Block> {
    let options = items_of(output);
    if options.is_empty() {
        return None;
    }
    let order = json!({
        "order_id": input.get("order_id"), "order_item_id": input.get("order_item_id"),
        "reason": input.get("reason"),
        "order_number": output.get("order_number"), "item": output.get("item"),
        "delivered_at": output.get("delivered_at"),
    });
    let mut b = Block::new(
        "return_options",
        "Your options",
        json!({"order": order, "options": options, "selected": null,
               "steps": output.get("steps").cloned().unwrap_or(json!([]))}),
    );
    for o in &options {
        let kind = o.get("kind").and_then(Value::as_str).unwrap_or("refund");
        let title = o
            .get("title")
            .and_then(Value::as_str)
            .map_or_else(|| words(kind), str::to_owned);
        let mut body = json!({"order_id": input.get("order_id"), "order_item_id": input.get("order_item_id"),
                              "reason": input.get("reason"), "kind": kind});
        if let Some(q) = input.get("quantity") {
            body["quantity"] = q.clone();
        }
        if let Some(r) = o.get("refund_amount") {
            body["expected_refund"] = r.clone();
        }
        if let Some(c) = o.get("you_pay") {
            body["expected_charge"] = c.clone();
        }
        b.actions.push(Action::person(
            &format!("send_{kind}"),
            format!("Send request: {title}"),
            "/v1/fulfillment/returns",
            body,
            format!("Send this request ({title})?"),
        ));
    }
    Some(b)
}

/// A remembered preference, with the shopper's undo button.
pub fn memory_note(memory_id: &str, text: &str) -> Block {
    let mut b = Block::new(
        "memory_note",
        "Remembered",
        json!({"memory_id": memory_id, "text": text}),
    );
    b.actions.push(Action::person(
        "undo",
        "Undo",
        format!("/v1/customer/me/memories/{memory_id}/undo"),
        json!({}),
        "Forget this?",
    ));
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::fixtures::repo;

    fn jackets() -> Value {
        json!({"items": [
            {"product_id": "prd_windbreaker", "name": "Packable windbreaker", "price": 71.20,
             "price_currency": "USD", "list_price": 89.00, "weight_g": 190, "packed_size": "14 × 18 cm", "image": "w.jpg"},
            {"product_id": "prd_field_jacket", "name": "Linen blend field jacket", "price": 102.40,
             "price_currency": "USD", "list_price": 128.00, "weight_g": 340, "material": "linen, cotton"}
        ], "total": 3})
    }

    #[test]
    fn layout_follows_words_then_profile_then_first_visit() {
        let p = repo();
        let rules = &p.get("shopper").unwrap().layout_rules;
        assert_eq!(
            layout(rules, "Which one packs smallest?", &[], false)
                .unwrap()
                .ui,
            "compare_table"
        );
        assert_eq!(
            layout(
                rules,
                "I need a light jacket",
                &["compares_specs".into()],
                false
            )
            .unwrap()
            .ui,
            "compare_table"
        );
        assert_eq!(
            layout(rules, "I need a light jacket", &[], true)
                .unwrap()
                .ui,
            "look_gallery"
        );
        assert_eq!(
            layout(
                rules,
                "Less text. Just show me bigger photos.",
                &["compares_specs".into()],
                false
            )
            .unwrap()
            .ui,
            "look_gallery"
        );
        assert_eq!(
            layout(rules, "A jacket", &[], false).unwrap().ui,
            "product_list"
        );
        // A remembered layout (F6) is a key and its value.
        let photos = [
            "layout_preference".to_owned(),
            "layout_preference=photos over details".to_owned(),
        ];
        assert_eq!(
            layout(rules, "Any rain jackets?", &photos, false)
                .unwrap()
                .ui,
            "look_gallery"
        );
        assert!(!profile_matches(
            "layout_preference=photos",
            "layout_preference"
        ));
    }

    #[test]
    fn product_layouts_copy_tool_values() {
        let b = products(
            "compare_table",
            &jackets(),
            Some("Because you compare specs."),
        )
        .unwrap();
        assert_eq!(b.title, "2 of 3 shown");
        assert_eq!(b.data["products"][1]["material"], "linen, cotton");
        let weight = b.data["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["label"] == "weight g")
            .unwrap();
        assert_eq!(weight["values"], json!([190, 340]));
        assert_eq!(b.data["prices"][0]["price"]["value"].to_string(), "71.2");
        assert!(
            b.suggested_intents[0]
                .element
                .starts_with("compare_column:prd_")
        );

        let g = products("look_gallery", &jackets(), None).unwrap();
        assert_eq!(g.data["items"][0]["image"], "w.jpg");
        let l = products("product_list", &jackets(), None).unwrap();
        assert!(
            l.data["items"][0]["facts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f == "weight g: 190")
        );
        assert!(products("product_list", &json!({"items": []}), None).is_none());
    }

    #[test]
    fn earlier_results_come_back_with_their_prices() {
        let list = products("product_list", &jackets(), None).unwrap();
        let again = results_of(&list.data);
        let g = products("look_gallery", &again, Some("Shown as photos.")).unwrap();
        assert_eq!(g.data["items"][0]["product_id"], "prd_windbreaker");
        assert_eq!(g.data["items"][0]["image"], "w.jpg");
        assert_eq!(g.data["items"][0]["price"]["value"].to_string(), "71.2");
        let table = products("compare_table", &jackets(), None).unwrap();
        let rows = results_of(&table.data);
        assert_eq!(rows["items"][1]["price"]["value"].to_string(), "102.4");
    }

    #[test]
    fn checkout_card_has_the_only_place_order_button() {
        let p: Preview = serde_json::from_value(json!({
            "preview_id": "pv_7731", "expires_at": "2026-11-07T10:42:00-05:00",
            "changes": [{"op": "create", "entity": "order", "id": "", "to": {
                "lines": [{"name": "Packable windbreaker", "size": "M", "quantity": 1}],
                "subtotal": {"value": 89.00, "currency": "USD"},
                "discounts": [{"label": "VIP weekend", "amount": {"value": 17.80, "currency": "USD"}}],
                "shipping": {"value": 0.00, "currency": "USD"},
                "total": {"value": 71.20, "currency": "USD"}}}],
            "checks": [{"rule": "checkout_validation.stock_available", "passed": true, "detail": null, "fix_tool": null}],
            "needs_approval": false, "approval_reason": null
        }))
        .unwrap();
        let b = checkout_card(&p);
        assert_eq!(b.title, "Your order · $71.20");
        assert_eq!(b.data["note"], "Only you can place it.");
        match &b.actions[..] {
            [
                Action::Person {
                    label, path, body, ..
                },
            ] => {
                assert_eq!(label, "Place order · $71.20");
                assert_eq!(path, "/v1/sales/orders");
                assert_eq!(body, &json!({"preview_id": "pv_7731"}));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn cart_returns_and_memory() {
        let c = cart(
            &json!({"lines": [{"cart_line_id": "cln_1", "name": "Packable windbreaker"}],
                             "total": {"value": 71.20, "currency": "USD"}}),
        )
        .unwrap();
        assert_eq!(c.title, "Your cart · $71.20");
        assert_eq!(c.suggested_intents[0].element, "cart_line:cln_1");
        assert!(cart(&json!({"items": []})).is_none());

        let r = return_options(
            &json!({"items": [{"kind": "refund", "title": "Full refund", "refund_amount": {"value": 312.00, "currency": "USD"}},
                              {"kind": "replacement", "title": "New pair, same size"}]}),
            &json!({"order_id": "ord_20402", "order_item_id": "oln_1", "reason": "defect"}),
        )
        .unwrap();
        assert_eq!(r.person_actions().count(), 2);
        assert!(r.actions.iter().all(
            |a| matches!(a, Action::Person { path, .. } if path == "/v1/fulfillment/returns")
        ));

        let m = memory_note(
            "mem_12",
            "Weight and packed size matter to you when traveling",
        );
        assert!(
            matches!(&m.actions[0], Action::Person { path, .. } if path == "/v1/customer/me/memories/mem_12/undo")
        );
    }
}
