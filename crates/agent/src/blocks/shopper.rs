//! Storefront blocks for fit, gifts, what the assistant understood, and what the store knows
//! about the shopper (docs/api/chat.md §3.3, scenes F3, F4, F6). Every value is copied from
//! tool output; the model adds only short wording (the plan's notes and understood lines), which
//! the runtime has already checked for numbers the tools did not return.

use acom_contract::Preview;
use serde_json::{Value, json};

use super::{Action, Block, SuggestedIntent, money_label, words};
use crate::executor::rows;

/// One read of this turn, as the builders look at it.
#[derive(Debug, Clone, Copy)]
pub struct Read<'a> {
    pub tool: &'a str,
    pub output: &'a Value,
}

fn text(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn id_of(item: &Value) -> Option<&str> {
    item.get("product_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
}

/// What the shopper pays for a product and its list price: the member price a read returned
/// (`member_price`), else the price itself. Copied, never computed.
fn prices(item: &Value) -> (Value, Value, Value) {
    let currency = item.get("price_currency").cloned().unwrap_or(Value::Null);
    let list = super::price_of(item, "price");
    match item.get("member_price").filter(|m| m.is_object()) {
        Some(m) => {
            let value = m.get("price").or_else(|| m.get("value")).cloned();
            let pays = match value {
                Some(v) => json!({"value": v, "currency": m.get("price_currency")
                    .or_else(|| m.get("currency")).cloned().unwrap_or(currency)}),
                None => list.clone(),
            };
            (pays, list, m.get("name").cloned().unwrap_or(Value::Null))
        }
        None => (list, Value::Null, Value::Null),
    }
}

// ---------------------------------------------------------------- looking_for

/// The conditions understood from the conversation, as chips the shopper can remove.
/// `chips` are `(label, source)` with source `said`, `profile`, `his` or `hers`.
pub fn looking_for(chips: &[(String, String)]) -> Option<Block> {
    if chips.is_empty() {
        return None;
    }
    let data = json!({"chips": chips.iter().map(|(label, source)| json!({
        "label": label, "source": source, "removable": true,
    })).collect::<Vec<_>>()});
    let mut b = Block::new("looking_for", "Looking for", data);
    for (label, _) in chips {
        b.suggested_intents
            .push(SuggestedIntent::new(format!("chip:{label}"), "Drop this"));
    }
    Some(b)
}

// ---------------------------------------------------------------- profile_card

/// "What we know about you", from `get_my_profile`: remembered preferences (each with the
/// shopper's Forget button) and saved sizes.
pub fn profile_card(profile: &Value) -> Option<Block> {
    let prefs = profile.get("preferences").and_then(Value::as_array)?;
    let mut facts = Vec::new();
    let mut actions = Vec::new();
    for p in prefs {
        let Some(key) = text(p.get("key")) else {
            continue;
        };
        let value = text(p.get("text")).or_else(|| text(p.get("value")));
        let memory_id = text(p.get("memory_id")).or_else(|| text(p.get("id")));
        facts.push(
            json!({"key": key, "value": value, "source": p.get("source"),
                          "memory_id": memory_id}),
        );
        if let Some(id) = &memory_id {
            actions.push(Action::person(
                &format!("forget_{id}"),
                "Forget",
                format!("/v1/customer/me/memories/{id}/undo"),
                json!({}),
                format!("Forget “{}”?", value.as_deref().unwrap_or(&key)),
            ));
        }
    }
    for (kind, size) in profile
        .get("sizes")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        if let Some(size) = size.as_str() {
            facts.push(json!({"key": format!("size_{kind}"), "value": size,
                              "source": "profile", "memory_id": null}));
        }
    }
    let mut b = Block::new(
        "profile_card",
        "What we know about you",
        json!({"facts": facts}),
    );
    b.actions = actions;
    b.actions.push(Action::say(
        "correct",
        "Correct something",
        json!({"intent": "correct_profile"}),
    ));
    for f in &facts {
        let id = f["memory_id"]
            .as_str()
            .or_else(|| f["key"].as_str())
            .unwrap_or_default();
        b.suggested_intents.push(SuggestedIntent::new(
            format!("profile_fact:{id}"),
            "Why do you know this?",
        ));
    }
    Some(b)
}

// ---------------------------------------------------------------- fit_check

/// A size the shopper owns: from their orders.
#[derive(Debug, Clone)]
struct Owned {
    product_id: String,
    name: String,
    size: Option<String>,
    status: String,
}

fn owned(reads: &[Read<'_>]) -> Vec<Owned> {
    let mut out = Vec::new();
    for r in reads.iter().filter(|r| r.tool == "get_my_orders") {
        for order in rows(r.output) {
            for line in order
                .get("ordered_item")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(pid) = text(line.get("product_id")) {
                    out.push(Owned {
                        product_id: pid,
                        name: text(line.get("name")).unwrap_or_default(),
                        size: text(line.get("size")),
                        status: text(line.get("order_item_status")).unwrap_or_default(),
                    });
                }
            }
        }
    }
    out
}

fn kept(o: &Owned) -> bool {
    !matches!(o.status.as_str(), "returned" | "cancelled")
}

/// `56 cm`, `65.5 cm`: the tool's number and unit.
fn measure(v: Option<&Value>, unit: &str) -> Value {
    match v {
        Some(Value::Number(n)) => json!(format!("{n} {unit}").trim().to_owned()),
        Some(Value::String(s)) => json!(s),
        _ => Value::Null,
    }
}

/// The product a read describes: an item of a list read, or a single product read.
fn product_facts(reads: &[Read<'_>], product_id: &str) -> Option<Value> {
    reads.iter().find_map(|r| {
        rows(r.output)
            .into_iter()
            .find(|i| id_of(i) == Some(product_id) && i.get("name").is_some())
    })
}

/// Which size, why, and what the answer rests on (F3): the asked product's sizes side by side
/// with something the shopper owns and kept, what reviews say, and what was and was not used.
/// `notes` are the plan's checked wording by element (`size:L` → "Over a hoodie"), whose
/// order is the recommendation; `remembered` are memory lines saved this turn.
pub fn fit_check(
    reads: &[Read<'_>],
    notes: &[(String, String)],
    remembered: &[String],
) -> Option<Block> {
    let measured: Vec<&Value> = reads
        .iter()
        .filter(|r| r.tool == "get_product_measurements" && r.output.get("sizes").is_some())
        .map(|r| r.output)
        .collect();
    let mine = owned(reads);
    let owns = |pid: &str| mine.iter().any(|o| o.product_id == pid && kept(o));
    let pid = |m: &Value| text(m.get("product_id")).unwrap_or_default();
    let asked = measured
        .iter()
        .find(|m| !owns(&pid(m)))
        .or_else(|| measured.first())
        .copied()?;
    let asked_id = pid(asked);
    let reference = measured
        .iter()
        .find(|m| pid(m) != asked_id && owns(&pid(m)))
        .copied();
    let ref_line = reference.and_then(|r| {
        mine.iter()
            .find(|o| o.product_id == pid(r) && kept(o))
            .cloned()
    });
    let ref_row = reference.and_then(|r| {
        let size = ref_line.as_ref().and_then(|l| l.size.clone());
        let sizes = r.get("sizes").and_then(Value::as_array)?;
        sizes
            .iter()
            .find(|s| size.is_some() && text(s.get("size")) == size)
            .or_else(|| sizes.first())
            .cloned()
    });

    let product = product_facts(reads, &asked_id).unwrap_or(Value::Null);
    let name = text(product.get("name")).unwrap_or_else(|| words(&asked_id));
    let (price, list_price, _) = prices(&product);
    let unit = text(asked.get("unit")).unwrap_or_default();
    let names: Vec<String> = asked
        .get("measurements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    let sizes: Vec<Value> = asked
        .get("sizes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let size_of = |s: &Value| text(s.get("size")).unwrap_or_default();
    let ref_name = ref_line.as_ref().map(|l| {
        format!(
            "{}{}",
            l.name.to_lowercase(),
            l.size
                .as_deref()
                .map(|s| format!(" {s}"))
                .unwrap_or_default()
        )
    });

    // Measurements: this product's sizes, then the one the shopper owns.
    let mut columns = vec![json!("")];
    columns.extend(
        sizes
            .iter()
            .map(|s| json!(format!("{name}, {}", size_of(s)))),
    );
    if let Some(r) = &ref_name {
        columns.push(json!(format!("Your {r}")));
    }
    let rows_out: Vec<Value> = names
        .iter()
        .map(|m| {
            let mut values: Vec<Value> = sizes.iter().map(|s| measure(s.get(m), &unit)).collect();
            if ref_name.is_some() {
                values.push(measure(ref_row.as_ref().and_then(|r| r.get(m)), &unit));
            }
            json!({"label": capitalized(&words(m)), "values": values})
        })
        .collect();

    // Options: the sizes the model wrote about, in its order (the first is its pick), else all.
    let noted: Vec<(&Value, Option<&str>)> = notes
        .iter()
        .filter_map(|(element, note)| {
            let size = element.strip_prefix("size:")?;
            sizes
                .iter()
                .find(|s| size_of(s) == size)
                .map(|s| (s, Some(note.as_str())))
        })
        .collect();
    let picked = !noted.is_empty();
    let chosen: Vec<(&Value, Option<&str>)> = if picked {
        noted
    } else {
        sizes.iter().map(|s| (s, None)).collect()
    };
    let first = names.first().cloned().unwrap_or_default();
    let options: Vec<Value> = chosen
        .iter()
        .enumerate()
        .map(|(n, (s, label))| {
            let mut reason = format!(
                "{} {}",
                capitalized(&words(&first)),
                measure(s.get(&first), &unit).as_str().unwrap_or("")
            );
            if let (Some(r), Some(row)) = (&ref_name, &ref_row)
                && let Some(v) = measure(row.get(&first), &unit).as_str()
            {
                reason.push_str(&format!("; your {r}, which you kept, is {v}"));
            }
            json!({"label": label, "size": size_of(s), "reason": reason.trim(),
                   "price": price, "recommended": picked && n == 0})
        })
        .collect();

    // Reviews.
    let review = reads
        .iter()
        .find(|r| r.tool == "get_review_summary")
        .map(|r| r.output);
    let reviews = review.map_or(json!({}), |r| {
        let count = r.get("review_count").cloned().unwrap_or(Value::Null);
        let filter = r.get("filter").cloned().unwrap_or(Value::Null);
        let heights = match (
            filter.get("height_min_cm").filter(|v| v.is_number()),
            filter.get("height_max_cm").filter(|v| v.is_number()),
        ) {
            (Some(a), Some(b)) => format!(" from people {a}–{b} cm"),
            _ => String::new(),
        };
        let fit: Vec<Value> = r
            .get("fit_votes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|v| v.get("count").and_then(Value::as_u64).unwrap_or(0) > 0)
            .map(|v| {
                json!(format!(
                    "{} say {}",
                    v["count"],
                    v.get("label")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_lowercase()
                ))
            })
            .collect();
        json!({"sample": if count.is_null() { Value::Null } else { json!(format!("{count} reviews{heights}")) },
               "fit": fit,
               "mentions": r.get("mentions").cloned().unwrap_or(json!([]))})
    });

    // What the answer used, and what it did not have.
    let mut used = Vec::new();
    if let Some(l) = &ref_line {
        used.push(json!({"label": "Your orders",
            "value": format!("{} {}, kept", l.name, l.size.as_deref().unwrap_or("")).replace("  ", " ")}));
    }
    let mut profile: Vec<String> = Vec::new();
    for r in reads.iter().filter(|r| r.tool == "get_my_profile") {
        for p in r
            .output
            .get("preferences")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if matches!(
                p.get("key").and_then(Value::as_str),
                Some("size" | "fit_note")
            ) && let Some(t) = text(p.get("text")).or_else(|| text(p.get("value")))
            {
                profile.push(t);
            }
        }
    }
    for m in remembered {
        let line = m.trim_start_matches("Remembered:").trim();
        if !line.is_empty() {
            profile.push(capitalized(line));
        }
    }
    if !profile.is_empty() {
        used.push(json!({"label": "Your profile", "value": profile.join(", ")}));
    }
    let knows_body = reads.iter().any(|r| {
        ["height", "height_cm", "weight", "weight_kg"]
            .iter()
            .any(|k| r.output.get(*k).is_some())
    });
    let not_used: Vec<Value> = if knows_body {
        Vec::new()
    } else {
        vec![json!({"label": "Height and weight", "value": "You haven't shared them"})]
    };

    let mut facts: Vec<Value> = Vec::new();
    for f in ["color", "material"] {
        if let Some(t) = text(product.get(f)) {
            facts.push(json!(t));
        }
    }
    let data = json!({
        "product": {"product_id": asked_id, "name": name, "image": product.get("image").cloned()
                        .or_else(|| product.get("image_url").cloned()).unwrap_or(Value::Null),
                    "facts": facts, "price": price, "list_price": list_price,
                    "note": product.get("summary").or_else(|| product.get("description")).cloned()
                        .unwrap_or(Value::Null)},
        "options": options,
        "measurements": {"columns": columns, "rows": rows_out,
                         "caption": asked.get("how_measured").cloned().unwrap_or(Value::Null)},
        "reviews": reviews,
        "used": used,
        "not_used": not_used,
    });
    let mut b = Block::new("fit_check", format!("{name} · which size"), data);
    let price_text = money_label(&price);
    for o in &options {
        let size = o["size"].as_str().unwrap_or_default();
        let label = match &price_text {
            Some(p) => format!("Add {size} · {p}"),
            None => format!("Add {size}"),
        };
        b.actions.push(Action::say(
            &format!("add_{size}"),
            label,
            json!({"product_id": asked_id, "size": size}),
        ));
        b.suggested_intents.push(SuggestedIntent::new(
            format!("fit_option:{size}"),
            format!("Why {size}?"),
        ));
    }
    Some(b)
}

// ---------------------------------------------------------------- gift_picker

/// The size chosen for each product, from the cart or the prepared gift.
fn chosen_size(pid: &str, name: &str, cart: Option<&Value>, gift: Option<&Value>) -> Value {
    let from_cart = cart.and_then(|c| {
        rows(c)
            .into_iter()
            .find(|l| l.get("product_id").and_then(Value::as_str) == Some(pid))
            .and_then(|l| l.get("size").cloned())
    });
    let from_gift = || {
        gift.and_then(|g| {
            g.get("lines")?
                .as_array()?
                .iter()
                .find(|l| {
                    l.get("product_id").and_then(Value::as_str) == Some(pid)
                        || l.get("name").and_then(Value::as_str) == Some(name)
                })
                .and_then(|l| l.get("size").cloned())
        })
    };
    from_cart.or_else(from_gift).unwrap_or(Value::Null)
}

/// A few picks for someone else (F4), the gift options and where it goes, from the products
/// read (or shown earlier), the cart, and the `prepare_gift_order` preview. Nothing here is
/// saved about the recipient. `review_target` is the block (or page) the shopper confirms on.
pub fn gift_picker(
    picks: &[Value],
    cart: Option<&Value>,
    gift: Option<&Preview>,
    review_target: Option<&str>,
) -> Option<Block> {
    let order = gift.and_then(|p| {
        p.changes
            .iter()
            .map(|c| &c.to)
            .find(|to| to.get("total").is_some())
    });
    if picks.is_empty() && order.is_none() {
        return None;
    }
    let shown: Vec<Value> = picks
        .iter()
        .take(3)
        .map(|i| {
            let pid = id_of(i).unwrap_or_default().to_owned();
            let name = text(i.get("name")).unwrap_or_default();
            let (price, list_price, label) = if i.get("member_price").is_some()
                || !i.get("price").is_some_and(Value::is_object)
            {
                prices(i)
            } else {
                // A pick shown earlier keeps the prices it had.
                (
                    i.get("price").cloned().unwrap_or(Value::Null),
                    i.get("list_price").cloned().unwrap_or(Value::Null),
                    i.get("price_label").cloned().unwrap_or(Value::Null),
                )
            };
            json!({"product_id": pid, "name": name, "price": price, "list_price": list_price,
                   "price_label": label,
                   "why": i.get("why").or_else(|| i.get("caption")).or_else(|| i.get("summary"))
                       .or_else(|| i.get("description")).cloned().unwrap_or(Value::Null),
                   "image": i.get("image").or_else(|| i.get("image_url")).cloned().unwrap_or(Value::Null),
                   "chosen_size": chosen_size(&pid, &name, cart, order)})
        })
        .collect();

    let options = order.and_then(|o| o.get("gift_options")).map(|g| {
        let wrap = match (
            g.get("wrap").and_then(Value::as_bool),
            g.get("wrap_fee").and_then(money_label),
        ) {
            (Some(true), Some(fee)) => json!(format!("Gift wrap, +{fee}")),
            (Some(true), None) => json!("Gift wrap"),
            (Some(false), _) => json!("No wrap"),
            _ => Value::Null,
        };
        let receipt = match (
            g.get("gift_receipt").and_then(Value::as_bool),
            g.get("receipt_hides_prices").and_then(Value::as_bool),
        ) {
            (Some(true), Some(true)) => json!("Gift receipt, no prices shown"),
            (Some(true), _) => json!("Gift receipt"),
            (Some(false), _) => json!("Regular receipt"),
            _ => Value::Null,
        };
        json!({"wrap": wrap, "card_message": g.get("card_message").cloned().unwrap_or(Value::Null),
               "receipt": receipt,
               "exchange_until": g.get("exchange_until").cloned().unwrap_or(Value::Null)})
    });
    let delivery = order.and_then(|o| o.get("delivery")).map(|d| {
        json!({"to": d.get("address_label").filter(|v| !v.is_null()).or_else(|| d.get("to"))
                    .cloned().unwrap_or(Value::Null),
               "arrives": d.get("arrives").cloned().unwrap_or(Value::Null),
               "address_saved": false})
    });
    let total = order
        .and_then(|o| o.get("total"))
        .cloned()
        .unwrap_or(Value::Null);
    let data = json!({
        "picks": shown,
        "gift_options": options.unwrap_or(json!({})),
        "delivery": delivery.unwrap_or(json!({"address_saved": false})),
        "total": total,
    });
    let title = match shown.len() {
        0 => "Your gift".to_owned(),
        n => format!("{n} picks"),
    };
    let mut b = Block::new("gift_picker", title, data);
    b.why = Some("It's for someone else, so none of it is saved to your profile.".into());
    for p in &shown {
        let pid = p["product_id"].as_str().unwrap_or_default();
        if pid.is_empty() {
            continue;
        }
        if p["chosen_size"].is_null() {
            b.actions.push(Action::say(
                &format!("choose_{pid}"),
                format!("Choose {}", p["name"].as_str().unwrap_or("this")),
                json!({"product_id": pid}),
            ));
        }
        b.suggested_intents.push(SuggestedIntent::new(
            format!("product_card:{pid}"),
            "Why this one?",
        ));
    }
    let ready = gift.is_some_and(|p| p.checks.iter().all(|c| c.passed));
    if let (true, Some(target)) = (ready, review_target) {
        let label = money_label(&total).map_or_else(
            || "Review gift order".to_owned(),
            |t| format!("Review gift order · {t}"),
        );
        b.actions.push(Action::open("review", label, target));
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read<'a>(tool: &'a str, output: &'a Value) -> Read<'a> {
        Read { tool, output }
    }

    #[test]
    fn fit_check_sets_the_coat_beside_the_windbreaker_she_kept() {
        let coat = json!({"items": [{"product_id": "prd_wool_overcoat", "name": "Wool overcoat",
            "price": 289.00, "price_currency": "USD", "color": "Charcoal", "material": "70% wool"}], "total": 1});
        let coat_m = json!({"product_id": "prd_wool_overcoat", "unit": "cm",
            "measurements": ["chest", "shoulder", "sleeve", "length"],
            "sizes": [{"size": "M", "chest": 56, "shoulder": 46, "sleeve": 64, "length": 98},
                      {"size": "L", "chest": 59, "shoulder": 48, "sleeve": 65.5, "length": 100}]});
        let orders = json!({"items": [
            {"id": "ord_20381", "ordered_item": [{"product_id": "prd_merino_tee", "name": "Merino trail tee", "size": "M", "order_item_status": "delivered"}]},
            {"id": "ord_20268", "ordered_item": [{"product_id": "prd_windbreaker", "name": "Packable windbreaker", "size": "M", "order_item_status": "delivered"}]}],
            "total": 2});
        let wb = json!({"product_id": "prd_windbreaker", "unit": "cm",
            "measurements": ["chest", "shoulder", "sleeve", "length"],
            "sizes": [{"size": "M", "chest": 57, "shoulder": 47, "sleeve": 64, "length": 71}]});
        let reviews = json!({"product_id": "prd_wool_overcoat", "review_count": 41,
            "filter": {"height_min_cm": 175, "height_max_cm": 182, "size": null},
            "fit_votes": [{"fit": "true_to_size", "label": "True to size", "count": 29},
                          {"fit": "runs_small", "label": "A bit snug", "count": 12},
                          {"fit": "runs_large", "label": "Runs large", "count": 0}],
            "mentions": [{"text": "size up for a hoodie", "count": 9}]});
        let reads = [
            read("search_products", &coat),
            read("get_product_measurements", &coat_m),
            read("get_review_summary", &reviews),
            read("get_my_orders", &orders),
            read("get_product_measurements", &wb),
        ];
        let notes = [
            ("size:L".to_owned(), "Over a hoodie".to_owned()),
            ("size:M".to_owned(), "Over a shirt or sweater".to_owned()),
        ];
        let b = fit_check(
            &reads,
            &notes,
            &["Remembered: you layer hoodies under coats".to_owned()],
        )
        .unwrap();
        let d = &b.data;
        assert_eq!(d["product"]["name"], "Wool overcoat");
        assert_eq!(d["options"][0]["size"], "L");
        assert_eq!(d["options"][0]["label"], "Over a hoodie");
        assert_eq!(d["options"][0]["recommended"], true);
        assert_eq!(
            d["options"][0]["reason"],
            "Chest 59 cm; your packable windbreaker M, which you kept, is 57 cm"
        );
        assert_eq!(d["options"][1]["size"], "M");
        assert_eq!(
            d["measurements"]["columns"],
            json!([
                "",
                "Wool overcoat, M",
                "Wool overcoat, L",
                "Your packable windbreaker M"
            ])
        );
        assert_eq!(
            d["measurements"]["rows"][0],
            json!({"label": "Chest", "values": ["56 cm", "59 cm", "57 cm"]})
        );
        assert_eq!(d["measurements"]["rows"][2]["values"][1], "65.5 cm");
        assert_eq!(d["reviews"]["sample"], "41 reviews from people 175–182 cm");
        assert_eq!(
            d["reviews"]["fit"],
            json!(["29 say true to size", "12 say a bit snug"])
        );
        assert_eq!(d["used"][0]["value"], "Packable windbreaker M, kept");
        assert_eq!(d["used"][1]["value"], "You layer hoodies under coats");
        assert_eq!(d["not_used"][0]["label"], "Height and weight");
        assert!(matches!(&b.actions[0], Action::Say { label, args, .. }
            if label == "Add L · $289.00" && args["size"] == "L"));
        assert!(fit_check(&[read("search_products", &coat)], &[], &[]).is_none());
    }

    fn gift_preview() -> Preview {
        serde_json::from_value(json!({
            "preview_id": "pv_g118", "expires_at": null, "needs_approval": false, "approval_reason": null,
            "changes": [{"op": "create", "entity": "order", "id": "", "to": {
                "order_type": "gift",
                "lines": [{"variant_id": "var_windbreaker_l_slate", "name": "Packable windbreaker", "size": "L", "quantity": 1}],
                "gift_wrap": {"value": 5.00, "currency": "USD"},
                "total": {"value": 76.20, "currency": "USD"},
                "delivery": {"to": "Dad", "address_label": "Dad · 48 Pine Rd, Lakeview", "arrives": "2026-11-10", "address_saved": false},
                "gift_options": {"wrap": true, "wrap_fee": {"value": 5.00, "currency": "USD"},
                                 "card_message": "For the next trail. Love, J.", "gift_receipt": true,
                                 "receipt_hides_prices": true, "exchange_until": "2026-12-31"}}}],
            "checks": [{"rule": "gift.recipient_per_order", "passed": true, "detail": "One recipient: Dad", "fix_tool": null}]
        }))
        .unwrap()
    }

    #[test]
    fn gift_picker_shows_the_vip_price_the_chosen_size_and_the_prepared_gift() {
        let picks = rows(&json!({"items": [
            {"product_id": "prd_windbreaker", "name": "Packable windbreaker", "price": 89.00, "price_currency": "USD",
             "member_price": {"price": 71.20, "name": "VIP weekend"}, "description": "Packs into its own pocket."},
            {"product_id": "prd_fleece_half_zip", "name": "Fleece half-zip", "price": 84.00, "price_currency": "USD"},
            {"product_id": "prd_merino_tee", "name": "Merino trail tee", "price": 58.00, "price_currency": "USD"},
            {"product_id": "prd_extra", "name": "Fourth", "price": 10.00, "price_currency": "USD"}]}));
        let cart = json!({"lines": [{"product_id": "prd_windbreaker", "size": "L"}]});
        let p = gift_preview();
        let b = gift_picker(&picks, Some(&cart), Some(&p), Some("block:blk_9")).unwrap();
        let d = &b.data;
        assert_eq!(d["picks"].as_array().unwrap().len(), 3);
        assert_eq!(d["picks"][0]["price"]["value"].to_string(), "71.2");
        assert_eq!(d["picks"][0]["list_price"]["value"].to_string(), "89.0");
        assert_eq!(d["picks"][0]["price_label"], "VIP weekend");
        assert_eq!(d["picks"][0]["chosen_size"], "L");
        assert!(d["picks"][1]["chosen_size"].is_null());
        assert_eq!(d["gift_options"]["wrap"], "Gift wrap, +$5.00");
        assert_eq!(
            d["gift_options"]["receipt"],
            "Gift receipt, no prices shown"
        );
        assert_eq!(d["delivery"]["to"], "Dad · 48 Pine Rd, Lakeview");
        assert_eq!(d["delivery"]["address_saved"], false);
        assert_eq!(d["total"]["value"].to_string(), "76.2");
        assert!(
            b.actions
                .iter()
                .any(|a| matches!(a, Action::Open { label, target, .. }
            if label == "Review gift order · $76.20" && target == "block:blk_9"))
        );
        assert!(
            b.actions
                .iter()
                .any(|a| matches!(a, Action::Say { args, .. }
            if args["product_id"] == "prd_fleece_half_zip"))
        );
        assert_eq!(
            b.person_actions().count(),
            0,
            "the shopper places it on the card"
        );

        // An earlier block's picks keep their prices.
        let again = gift_picker(&rows(&d.clone()), None, None, None).unwrap();
        assert_eq!(again.data["picks"][0]["price"]["value"].to_string(), "71.2");
    }

    #[test]
    fn chips_and_the_profile_card() {
        let l = looking_for(&[
            ("Gift".into(), "said".into()),
            ("Size L".into(), "his".into()),
        ])
        .unwrap();
        assert_eq!(l.data["chips"][1]["source"], "his");
        assert!(looking_for(&[]).is_none());

        let p = profile_card(&json!({"sizes": {"tops": "M", "bottoms": null},
            "preferences": [{"memory_id": "mem_03", "key": "size", "value": "M", "text": "Size M", "source": "conversation"}]}))
        .unwrap();
        assert_eq!(p.data["facts"][0]["memory_id"], "mem_03");
        assert_eq!(p.data["facts"][1]["key"], "size_tops");
        assert!(matches!(&p.actions[0], Action::Person { path, .. }
            if path == "/v1/customer/me/memories/mem_03/undo"));
        assert!(profile_card(&json!({"name": "Jamie"})).is_none());
    }
}
