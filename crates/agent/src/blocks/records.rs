//! `record_list` (chat.md §3.3): what a shopper's read returned that is not a list of products,
//! as rows they can read: their orders, subscriptions, returns, coupons and points, reviews and
//! questions, notices and FAQs, sales running now, collections, alerts, addresses, messages, a
//! delivery, a return policy, their tier. Each tool has a shape that names what matters; any
//! other read is shown by a general rule, so whatever a turn read can be shown.
//!
//! Every value comes from the tool's output as it is (money as `{value, currency}`, times as
//! the store's own strings); the web only lays it out.

use serde_json::{Map, Value, json};

use super::{Action, Block, SuggestedIntent, words};

/// Reads that only tell who is asking or set up another read; a turn that read anything else is
/// not answered with them.
pub const SUPPORTING: &[&str] = &[
    "get_my_profile",
    "get_payment_methods",
    "autocomplete",
    "get_facets",
];

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

/// A word value as people read it (`return_fees_customer_responsibility` → "Return fees
/// customer responsibility").
fn said(v: Option<&Value>) -> Option<String> {
    text(v).map(|s| capitalized(&words(&s)))
}

fn is_money(v: &Value) -> bool {
    v.get("value").is_some_and(Value::is_number) && v.get("currency").is_some_and(Value::is_string)
}

fn items(output: &Value) -> Vec<Value> {
    output
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// One row of the list. Only the fields a row has are written.
#[derive(Default)]
struct Row {
    id: String,
    title: String,
    subtitle: Option<String>,
    image: Option<String>,
    href: Option<String>,
    status: Option<String>,
    amount: Option<Value>,
    rating: Option<Value>,
    dates: Vec<(String, String)>,
    meta: Vec<String>,
    body: Option<String>,
}

impl Row {
    fn json(self) -> Value {
        let mut v = Map::new();
        v.insert("id".into(), json!(self.id));
        v.insert("title".into(), json!(self.title));
        for (k, x) in [
            ("subtitle", self.subtitle),
            ("image", self.image),
            ("href", self.href),
            ("status", self.status),
            ("body", self.body),
        ] {
            if let Some(x) = x {
                v.insert(k.into(), json!(x));
            }
        }
        if let Some(a) = self.amount {
            v.insert("amount".into(), a);
        }
        if let Some(r) = self.rating {
            v.insert("rating".into(), r);
        }
        if !self.dates.is_empty() {
            let d: Vec<Value> = self
                .dates
                .into_iter()
                .map(|(label, at)| json!({"label": label, "at": at}))
                .collect();
            v.insert("dates".into(), Value::Array(d));
        }
        if !self.meta.is_empty() {
            v.insert("meta".into(), json!(self.meta));
        }
        Value::Object(v)
    }
}

fn date(label: &str, v: Option<&Value>) -> Option<(String, String)> {
    text(v).map(|at| (label.to_owned(), at))
}

/// `/p/prd_x?color=Black`.
fn product_href(product_id: Option<&Value>, color: Option<&Value>) -> Option<String> {
    let id = text(product_id)?;
    Some(match text(color) {
        Some(c) => format!("/p/{id}?color={}", c.replace(' ', "%20")),
        None => format!("/p/{id}"),
    })
}

/// "Packable windbreaker (Black, M) × 2".
fn line_label(l: &Value) -> String {
    let name = text(l.get("name")).unwrap_or_default();
    let opts: Vec<String> = [text(l.get("color")), text(l.get("size"))]
        .into_iter()
        .flatten()
        .collect();
    let qty = l
        .get("order_quantity")
        .or_else(|| l.get("quantity"))
        .and_then(Value::as_i64)
        .unwrap_or(1);
    let mut s = name;
    if !opts.is_empty() {
        s.push_str(&format!(" ({})", opts.join(", ")));
    }
    if qty > 1 {
        s.push_str(&format!(" × {qty}"));
    }
    s
}

fn orders(output: &Value) -> Vec<Row> {
    items(output)
        .iter()
        .map(|o| {
            let lines: Vec<Value> = o
                .get("ordered_item")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let first = lines.first();
            let id = text(o.get("id")).unwrap_or_default();
            Row {
                title: format!("Order #{}", text(o.get("order_number")).unwrap_or_default()),
                subtitle: Some(lines.iter().map(line_label).collect::<Vec<_>>().join(", "))
                    .filter(|s| !s.is_empty()),
                image: first.and_then(|l| text(l.get("image"))),
                href: Some(format!("/orders/{id}")),
                status: text(o.get("order_status")),
                amount: o.get("total").cloned(),
                dates: [
                    date("Ordered", o.get("order_date")),
                    date("Delivered", o.get("delivered_at")),
                ]
                .into_iter()
                .flatten()
                .collect(),
                id,
                ..Row::default()
            }
        })
        .collect()
}

fn subscriptions(output: &Value) -> Vec<Row> {
    items(output)
        .iter()
        .map(|s| {
            let lines: Vec<Value> = s
                .get("lines")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let first = lines.first();
            Row {
                id: text(s.get("id")).unwrap_or_default(),
                title: lines.iter().map(line_label).collect::<Vec<_>>().join(", "),
                subtitle: said(s.get("billing_cycle")),
                image: first.and_then(|l| text(l.get("image"))),
                href: first.and_then(|l| product_href(l.get("product_id"), l.get("color"))),
                status: text(s.get("status")),
                amount: s.get("amount").cloned(),
                dates: date("Next", s.get("next_charge_date"))
                    .into_iter()
                    .collect(),
                meta: [
                    text(s.pointer("/payment/label")),
                    text(s.pointer("/delivery/address_label")),
                ]
                .into_iter()
                .flatten()
                .collect(),
                ..Row::default()
            }
        })
        .collect()
}

fn claim_row(c: &Value) -> Row {
    let kind = said(c.get("claim_type")).unwrap_or_else(|| "Request".into());
    let order = text(c.get("order_id")).unwrap_or_default();
    Row {
        id: text(c.get("id")).unwrap_or_default(),
        title: format!(
            "{kind} · order #{}",
            order.strip_prefix("ord_").unwrap_or(&order)
        ),
        subtitle: said(c.get("reason")),
        href: (!order.is_empty()).then(|| format!("/orders/{order}")),
        status: text(c.get("status")),
        amount: c
            .get("refund_estimate")
            .or_else(|| c.get("amount"))
            .cloned(),
        dates: date("Requested", c.get("created_at")).into_iter().collect(),
        body: text(c.get("reason_text")),
        ..Row::default()
    }
}

fn coupons(output: &Value) -> Vec<Row> {
    items(output)
        .iter()
        .map(|c| Row {
            id: text(c.get("id")).unwrap_or_default(),
            title: text(c.get("code"))
                .or_else(|| text(c.get("name")))
                .unwrap_or_else(|| "Coupon".into()),
            subtitle: text(c.get("code")).and(text(c.get("name"))),
            status: text(c.get("status")),
            dates: date("Until", c.get("valid_through")).into_iter().collect(),
            meta: text(c.get("order_id"))
                .map(|o| format!("Used on order #{}", o.trim_start_matches("ord_")))
                .into_iter()
                .collect(),
            ..Row::default()
        })
        .collect()
}

fn points_rows(output: &Value) -> Vec<Row> {
    // A balance: what expires soon. A ledger: each entry.
    if let Some(soon) = output.get("expiring_soon").and_then(Value::as_array) {
        return soon
            .iter()
            .enumerate()
            .map(|(n, e)| Row {
                id: format!("expiring_{n}"),
                title: "Expiring soon".into(),
                amount: e.get("amount").cloned(),
                dates: date("Expires", e.get("expires")).into_iter().collect(),
                ..Row::default()
            })
            .collect();
    }
    items(output)
        .iter()
        .map(|t| Row {
            id: text(t.get("id")).unwrap_or_default(),
            title: text(t.get("description"))
                .or_else(|| said(t.get("type")))
                .unwrap_or_default(),
            status: text(t.get("type")),
            amount: t.get("amount").cloned(),
            dates: [date("On", t.get("at")), date("Expires", t.get("expires"))]
                .into_iter()
                .flatten()
                .collect(),
            ..Row::default()
        })
        .collect()
}

fn reviews(output: &Value) -> Vec<Row> {
    items(output)
        .iter()
        .map(|r| {
            let author = r
                .get("author")
                .and_then(|a| text(a.get("name")).or_else(|| text(Some(a))))
                .unwrap_or_else(|| "A shopper".into());
            Row {
                id: text(r.get("id")).unwrap_or_default(),
                title: author,
                rating: r.get("review_rating").cloned(),
                body: text(r.get("review_body")),
                dates: date("", r.get("date_published")).into_iter().collect(),
                meta: [
                    text(r.get("size_bought")).map(|s| format!("Bought {s}")),
                    said(r.get("fit")),
                    text(r.get("height_cm")).map(|h| format!("{h} cm tall")),
                ]
                .into_iter()
                .flatten()
                .collect(),
                ..Row::default()
            }
        })
        .collect()
}

/// "4.6 out of 5 · 41 reviews", with the fit votes and what reviewers mention.
fn review_summary(s: &Value) -> Option<Value> {
    let count = s.get("review_count")?;
    let rating = s.get("rating_value").cloned().unwrap_or(Value::Null);
    let mut detail: Vec<String> = s
        .get("fit_votes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|f| f.get("count").and_then(Value::as_i64).unwrap_or(0) > 0)
        .filter_map(|f| {
            Some(format!(
                "{} {}",
                text(f.get("count"))?,
                text(f.get("label"))?.to_lowercase()
            ))
        })
        .collect();
    detail.extend(
        s.get("mentions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(format!(
                    "“{}” {} times",
                    text(m.get("text"))?,
                    text(m.get("count"))?
                ))
            }),
    );
    Some(json!({"label": format!("{count} reviews"), "rating": rating, "detail": detail}))
}

fn questions(output: &Value) -> Vec<Row> {
    items(output)
        .iter()
        .map(|q| Row {
            id: text(q.get("id")).unwrap_or_default(),
            title: text(q.get("text")).unwrap_or_default(),
            subtitle: text(q.get("product_name")),
            href: product_href(q.get("product_id"), None),
            body: Some(
                q.pointer("/accepted_answer/text")
                    .and_then(|t| text(Some(t)))
                    .map_or_else(
                        || "Not answered yet.".to_owned(),
                        |a| format!("The store: {a}"),
                    ),
            ),
            meta: q
                .pointer("/author/name")
                .and_then(|a| text(Some(a)))
                .into_iter()
                .collect(),
            dates: date("Asked", q.get("date_created")).into_iter().collect(),
            ..Row::default()
        })
        .collect()
}

fn notice_row(n: &Value) -> Row {
    let id = text(n.get("id")).unwrap_or_default();
    Row {
        title: text(n.get("headline")).unwrap_or_default(),
        subtitle: said(n.get("category")),
        href: Some(format!("/help/notices/{id}")),
        status: (n.get("pinned") == Some(&Value::Bool(true))).then(|| "pinned".into()),
        dates: date("", n.get("date_published")).into_iter().collect(),
        body: text(n.get("article_body")),
        id,
        ..Row::default()
    }
}

fn faq_row(f: &Value) -> Row {
    let topic = text(f.get("category"));
    Row {
        id: text(f.get("id")).unwrap_or_default(),
        title: text(f.get("name")).unwrap_or_default(),
        subtitle: said(f.get("category")),
        href: Some(topic.map_or_else(|| "/help".into(), |t| format!("/help?topic={t}"))),
        body: text(f.get("accepted_answer")),
        ..Row::default()
    }
}

fn promotions(output: &Value) -> Vec<Row> {
    items(output)
        .iter()
        .map(|p| {
            let scope = text(p.pointer("/applies_to/scope"));
            let category = p
                .pointer("/applies_to/category_ids/0")
                .and_then(|c| text(Some(c)));
            let reach = match (
                scope.as_deref(),
                p.pointer("/applies_to/product_count")
                    .and_then(Value::as_i64),
            ) {
                (Some("order"), _) => Some("On the whole order".to_owned()),
                (_, Some(n)) => Some(format!("{n} products")),
                _ => None,
            };
            let who = text(p.get("valid_for_member_tier")).map(|t| {
                if t == "everyone" {
                    "For everyone".to_owned()
                } else {
                    format!("For {} members", words(&t))
                }
            });
            Row {
                id: text(p.get("id")).unwrap_or_default(),
                title: text(p.get("name")).unwrap_or_default(),
                subtitle: reach,
                href: category.map(|c| format!("/c/{c}")),
                status: text(p.get("status")),
                dates: date("Until", p.get("valid_through")).into_iter().collect(),
                meta: who.into_iter().collect(),
                ..Row::default()
            }
        })
        .collect()
}

fn collections(output: &Value) -> Vec<Row> {
    items(output)
        .iter()
        .map(|c| {
            let id = text(c.get("id")).unwrap_or_default();
            Row {
                title: text(c.get("name")).unwrap_or_default(),
                subtitle: text(c.get("product_count")).map(|n| format!("{n} products")),
                href: Some(format!("/collections/{id}")),
                dates: date("Until", c.get("valid_through")).into_iter().collect(),
                meta: c
                    .get("keywords")
                    .and_then(Value::as_array)
                    .map(|k| {
                        k.iter()
                            .filter_map(|w| text(Some(w)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .filter(|s| !s.is_empty())
                    .into_iter()
                    .collect(),
                id,
                ..Row::default()
            }
        })
        .collect()
}

/// One collection: its name and reach, then a row per section, each opening the collection.
fn collection(c: &Value) -> (Option<Value>, Vec<Row>) {
    let id = text(c.get("id")).unwrap_or_default();
    let href = format!("/collections/{id}");
    let reach = text(c.get("product_count")).map(|n| format!("{n} products"));
    let summary = json!({"label": text(c.get("name")).unwrap_or_default(),
                         "value": reach, "href": href});
    let rows = c
        .get("has_part")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(n, part)| {
            let count = part
                .get("item_list_element")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Row {
                id: format!("{id}_{n}"),
                title: text(part.get("name")).unwrap_or_else(|| format!("Section {}", n + 1)),
                subtitle: Some(format!("{count} products")),
                href: Some(href.clone()),
                ..Row::default()
            }
        })
        .collect();
    (Some(summary), rows)
}

fn alerts(output: &Value) -> Vec<Row> {
    items(output)
        .iter()
        .map(|a| {
            let back = text(a.get("availability")).as_deref() == Some("in_stock");
            Row {
                id: text(a.get("id")).unwrap_or_default(),
                title: text(a.get("product_name")).unwrap_or_default(),
                subtitle: Some(
                    [
                        text(a.get("color")),
                        text(a.get("size")).map(|s| format!("size {s}")),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(", "),
                )
                .filter(|s| !s.is_empty()),
                image: text(a.get("image")),
                href: product_href(a.get("product_id"), a.get("color")),
                status: Some(if back { "back_in_stock" } else { "waiting" }.into()),
                dates: [
                    date("Since", a.get("start_time")),
                    date("Until", a.get("expires")),
                ]
                .into_iter()
                .flatten()
                .collect(),
                meta: said(a.get("channel"))
                    .map(|c| format!("By {}", c.to_lowercase()))
                    .into_iter()
                    .collect(),
                ..Row::default()
            }
        })
        .collect()
}

fn addresses(output: &Value) -> Vec<Row> {
    items(output)
        .iter()
        .map(|a| Row {
            id: text(a.get("id")).unwrap_or_default(),
            title: text(a.get("label")).unwrap_or_else(|| "Address".into()),
            subtitle: Some(
                [
                    text(a.get("street_address")),
                    text(a.get("address_locality")),
                    text(a.get("address_region")),
                    text(a.get("postal_code")),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", "),
            ),
            status: (a.get("is_primary") == Some(&Value::Bool(true))).then(|| "primary".into()),
            ..Row::default()
        })
        .collect()
}

fn messages(output: &Value) -> Vec<Row> {
    items(output)
        .iter()
        .map(|m| Row {
            id: text(m.get("id")).unwrap_or_default(),
            title: text(m.get("headline")).unwrap_or_default(),
            subtitle: said(m.get("channel")),
            body: text(m.get("text")),
            dates: date("Sent", m.get("date_sent")).into_iter().collect(),
            ..Row::default()
        })
        .collect()
}

fn memories(output: &Value) -> Vec<Row> {
    items(output)
        .iter()
        .map(|m| Row {
            id: text(m.get("id")).unwrap_or_default(),
            title: text(m.get("text"))
                .or_else(|| text(m.get("value")))
                .unwrap_or_default(),
            subtitle: said(m.get("key")),
            dates: date("Remembered", m.get("created_at"))
                .into_iter()
                .collect(),
            meta: said(m.get("source")).into_iter().collect(),
            ..Row::default()
        })
        .collect()
}

/// A delivery: where it is, then each scan.
fn delivery(d: &Value) -> (Option<Value>, Vec<Row>) {
    let label = [text(d.get("carrier")), text(d.get("tracking_number"))]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let href = text(d.get("order_id")).map(|o| format!("/orders/{o}"));
    // `get_delivery` has its scans as `events`; `track_shipment` lists them as `items`.
    let events: Vec<Value> = d
        .get("events")
        .or_else(|| d.get("items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let state = said(d.get("delivery_state"))
        .or_else(|| events.last().and_then(|e| said(e.get("delivery_state"))));
    let summary = json!({"label": label, "value": state, "href": href});
    let rows = events
        .iter()
        .enumerate()
        .map(|(n, e)| Row {
            id: format!("event_{n}"),
            title: said(e.get("delivery_state")).unwrap_or_default(),
            subtitle: text(e.get("location")),
            dates: date("", e.get("at")).into_iter().collect(),
            ..Row::default()
        })
        .collect();
    (Some(summary), rows)
}

/// A return policy: how long, who pays, how it goes back.
fn policy(p: &Value) -> (Option<Value>, Vec<Row>) {
    let summary = json!({
        "label": text(p.get("name")).unwrap_or_else(|| "Return policy".into()),
        "value": text(p.get("merchant_return_days")).map(|d| format!("{d} days to return")),
    });
    let mut meta: Vec<String> = [said(p.get("return_fees")), said(p.get("refund_type"))]
        .into_iter()
        .flatten()
        .collect();
    if let Some(m) = p.get("return_method").and_then(Value::as_array) {
        let ways: Vec<String> = m.iter().filter_map(|w| said(Some(w))).collect();
        if !ways.is_empty() {
            meta.push(format!("Send back by {}", ways.join(" or ").to_lowercase()));
        }
    }
    if let Some(d) = text(p.get("acom:size_exchange_days")) {
        meta.push(format!("Size exchange within {d} days"));
    }
    let rows = vec![Row {
        id: text(p.get("merchant_return_policy_id")).unwrap_or_else(|| "policy".into()),
        title: "How returns work".into(),
        meta,
        ..Row::default()
    }];
    (Some(summary), rows)
}

/// A member's tier, or its history.
fn tier(output: &Value) -> (Option<Value>, Vec<Row>) {
    if let Some(t) = text(output.get("tier_id")) {
        let summary = json!({"label": "Member tier", "value": capitalized(&words(&t))});
        let rows = vec![Row {
            id: "tier".into(),
            title: text(output.pointer("/why/rule")).unwrap_or_else(|| "Your tier".into()),
            dates: [
                date("Since", output.get("since")),
                date("Next review", output.get("next_evaluation")),
            ]
            .into_iter()
            .flatten()
            .collect(),
            ..Row::default()
        }];
        return (Some(summary), rows);
    }
    let rows = items(output)
        .iter()
        .enumerate()
        .map(|(n, h)| Row {
            id: format!("tier_{n}"),
            title: format!(
                "{} → {}",
                capitalized(&words(&text(h.get("from_tier_id")).unwrap_or_default())),
                capitalized(&words(&text(h.get("to_tier_id")).unwrap_or_default()))
            ),
            subtitle: said(h.get("cause")),
            amount: h.pointer("/snapshot/value").cloned().filter(is_money),
            dates: date("", h.get("at")).into_iter().collect(),
            ..Row::default()
        })
        .collect();
    (None, rows)
}

/// Any other read: a row per item (or one for an entity), titled by its name and keeping its
/// status, money, times and a few other plain fields.
fn general(output: &Value) -> Vec<Row> {
    const TITLE: &[&str] = &[
        "name",
        "headline",
        "title",
        "label",
        "code",
        "description",
        "text",
        "product_name",
    ];
    let entities: Vec<Value> = match output.get("items").and_then(Value::as_array) {
        Some(list) => list.clone(),
        None if output.is_object() => vec![output.clone()],
        None => Vec::new(),
    };
    entities
        .iter()
        .enumerate()
        .map(|(n, e)| {
            let o = e.as_object().cloned().unwrap_or_default();
            let title_key = TITLE.iter().find(|k| text(o.get(**k)).is_some());
            let id = text(o.get("id")).unwrap_or_else(|| format!("row_{n}"));
            let mut row = Row {
                title: title_key
                    .and_then(|k| text(o.get(*k)))
                    .unwrap_or_else(|| id.clone()),
                id,
                ..Row::default()
            };
            for (k, v) in &o {
                if Some(&k.as_str()) == title_key || k == "id" || k.starts_with('@') {
                    continue;
                }
                if row.status.is_none() && (k == "status" || k.ends_with("_status")) {
                    row.status = text(Some(v));
                } else if row.amount.is_none() && is_money(v) {
                    row.amount = Some(v.clone());
                } else if (k.ends_with("_at")
                    || k.ends_with("_date")
                    || k == "expires"
                    || k == "at")
                    && text(Some(v)).is_some()
                {
                    row.dates.push((
                        capitalized(&words(k.trim_end_matches("_at"))),
                        text(Some(v)).unwrap_or_default(),
                    ));
                } else if k == "image" {
                    row.image = text(Some(v));
                } else if row.meta.len() < 3
                    && !k.ends_with("_id")
                    && let Some(t) = said(Some(v))
                {
                    row.meta.push(format!("{}: {t}", capitalized(&words(k))));
                }
            }
            row
        })
        .collect()
}

/// What kind of record a tool returns, and the block's heading for it.
fn kind_of(tool: &str) -> (&'static str, &'static str) {
    match tool {
        "get_my_orders" | "search_orders" => ("orders", "Your orders"),
        "list_subscriptions" | "get_subscription" => ("subscriptions", "Your subscriptions"),
        "search_claims" | "get_claim" => ("returns", "Returns and exchanges"),
        "list_member_coupons" => ("coupons", "Your coupons"),
        "get_loyalty_balance" | "list_loyalty_transactions" => ("points", "Your reward points"),
        "list_reviews" | "get_review_summary" => ("reviews", "Reviews"),
        "list_questions" => ("questions", "Questions and answers"),
        "list_notices" | "get_notice" => ("notices", "Store notices"),
        "list_faqs" | "get_faq" => ("faqs", "Questions people ask"),
        "list_active_promotions" => ("promotions", "On sale now"),
        "list_curations" | "get_curation" => ("collections", "Collections"),
        "list_restock_subscriptions" => ("alerts", "Back-in-stock alerts"),
        "list_addresses" => ("addresses", "Your addresses"),
        "list_notifications" => ("messages", "Messages from the store"),
        "get_delivery" | "track_shipment" => ("delivery", "Delivery"),
        "get_return_policy" => ("policy", "Returns"),
        "get_member_tier" | "list_tier_history" => ("tier", "Your membership"),
        "get_preferences" => ("memories", "What I remember about you"),
        _ => ("records", ""),
    }
}

/// Whether a tool has a shape of its own (not only the general rule).
pub fn shaped(tool: &str) -> bool {
    kind_of(tool).0 != "records"
}

/// The block for one read, or `None` when it returned nothing to show.
pub fn record_list(tool: &str, output: &Value) -> Option<Block> {
    record_list_with(tool, output, None)
}

/// Reviews with their summary (both reads of one product), as one block.
pub fn reviews_with_summary(list: Option<&Value>, summary: Option<&Value>) -> Option<Block> {
    match (list, summary) {
        (Some(l), s) => record_list_with("list_reviews", l, s.and_then(review_summary)),
        (None, Some(s)) => record_list_with("get_review_summary", s, review_summary(s)),
        (None, None) => None,
    }
}

fn record_list_with(tool: &str, output: &Value, summary: Option<Value>) -> Option<Block> {
    let (kind, heading) = kind_of(tool);
    let (summary, rows): (Option<Value>, Vec<Row>) = match tool {
        "get_my_orders" | "search_orders" => (None, orders(output)),
        "list_subscriptions" => (None, subscriptions(output)),
        "search_claims" => (None, items(output).iter().map(claim_row).collect()),
        "get_claim" => (None, vec![claim_row(output)]),
        "list_member_coupons" => (None, coupons(output)),
        "get_loyalty_balance" => (
            output
                .get("balance")
                .filter(|b| is_money(b))
                .map(|b| json!({"label": "Reward balance", "amount": b})),
            points_rows(output),
        ),
        "list_loyalty_transactions" => (None, points_rows(output)),
        "list_reviews" => (summary, reviews(output)),
        "get_review_summary" => (summary.or_else(|| review_summary(output)), Vec::new()),
        "list_questions" => (None, questions(output)),
        "list_notices" => (None, items(output).iter().map(notice_row).collect()),
        "get_notice" => (None, vec![notice_row(output)]),
        "list_faqs" => (None, items(output).iter().map(faq_row).collect()),
        "get_faq" => (None, vec![faq_row(output)]),
        "list_active_promotions" => (None, promotions(output)),
        "list_curations" => (None, collections(output)),
        "get_curation" => collection(output),
        "list_restock_subscriptions" => (None, alerts(output)),
        "list_addresses" => (None, addresses(output)),
        "list_notifications" => (None, messages(output)),
        "get_delivery" | "track_shipment" => delivery(output),
        "get_return_policy" => policy(output),
        "get_member_tier" | "list_tier_history" => tier(output),
        "get_preferences" => (None, memories(output)),
        _ => (None, general(output)),
    };
    let listed = output.get("items").is_some();
    if rows.is_empty() && summary.is_none() && !listed {
        return None;
    }
    let total = output
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or(rows.len() as u64);
    let shown = rows.len() as u64;
    let heading = if heading.is_empty() {
        capitalized(&words(
            tool.trim_start_matches("get_")
                .trim_start_matches("list_")
                .trim_start_matches("search_"),
        ))
    } else {
        heading.to_owned()
    };
    let title = if listed {
        format!("{heading} · {total}")
    } else {
        heading
    };
    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let rows: Vec<Value> = rows.into_iter().map(Row::json).collect();
    let mut data = json!({"kind": kind, "tool": tool, "rows": rows, "total": total});
    if let Some(s) = summary {
        data["summary"] = s;
    }
    let mut b = Block::new("record_list", title, data);
    if total > shown {
        b.actions.push(Action::say("more", "Show more", json!({})));
    }
    let intent = match kind {
        "orders" => Some("Where is this order?"),
        "subscriptions" => Some("Skip the next one"),
        "returns" => Some("What happens next with this?"),
        "promotions" | "collections" => Some("Show me what is in it"),
        "alerts" => Some("Is it back yet?"),
        "questions" | "faqs" | "notices" => None,
        _ => None,
    };
    if let Some(say) = intent {
        for id in ids.iter().take(10) {
            b.suggested_intents
                .push(SuggestedIntent::new(format!("record:{id}"), say));
        }
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_name_their_products_with_the_ordered_picture() {
        let out = json!({"items": [{"id": "ord_20268", "order_number": "20268", "order_status": "delivered",
            "order_date": "2026-09-08T09:14:00-04:00", "delivered_at": "2026-09-11T15:05:00-04:00",
            "ordered_item": [{"name": "Packable windbreaker", "size": "M", "color": "Black",
                              "image": "/media/w.webp", "order_quantity": 1}],
            "total": {"value": 89.0, "currency": "USD"}}], "total": 6});
        let b = record_list("get_my_orders", &out).unwrap();
        assert_eq!(b.ui, "record_list");
        assert_eq!(b.title, "Your orders · 6");
        let r = &b.data["rows"][0];
        assert_eq!(r["title"], "Order #20268");
        assert_eq!(r["subtitle"], "Packable windbreaker (Black, M)");
        assert_eq!(r["image"], "/media/w.webp");
        assert_eq!(r["href"], "/orders/ord_20268");
        assert_eq!(r["amount"]["value"], 89.0);
        assert_eq!(r["dates"][1]["label"], "Delivered");
        assert_eq!(b.actions.len(), 1, "5 more to show");
        assert_eq!(b.suggested_intents[0].element, "record:ord_20268");
    }

    #[test]
    fn a_balance_is_the_summary_and_what_expires_is_the_rows() {
        let out = json!({"customer_id": "cus_jamie", "balance": {"value": 13.94, "currency": "USD"},
            "expiring_soon": [{"amount": {"value": 2.0, "currency": "USD"}, "expires": "2026-11-14"}]});
        let b = record_list("get_loyalty_balance", &out).unwrap();
        assert_eq!(b.data["summary"]["amount"]["value"], 13.94);
        assert_eq!(b.data["rows"][0]["title"], "Expiring soon");
        assert_eq!(b.data["rows"][0]["dates"][0]["at"], "2026-11-14");
    }

    #[test]
    fn reviews_come_with_their_summary_and_questions_with_the_answer() {
        let list = json!({"items": [{"id": "rev_1", "review_rating": 5, "author": {"name": "Jordan P."},
            "review_body": "Warm.", "fit": "true_to_size", "size_bought": "L", "date_published": "2026-10-12"}], "total": 41});
        let summary = json!({"review_count": 41, "rating_value": 4.6,
            "fit_votes": [{"label": "True to size", "count": 29}, {"label": "Runs large", "count": 0}],
            "mentions": [{"text": "size up for a hoodie", "count": 9}]});
        let b = reviews_with_summary(Some(&list), Some(&summary)).unwrap();
        assert_eq!(b.data["summary"]["label"], "41 reviews");
        assert_eq!(b.data["summary"]["rating"], 4.6);
        assert_eq!(
            b.data["summary"]["detail"],
            json!(["29 true to size", "“size up for a hoodie” 9 times"])
        );
        assert_eq!(
            b.data["rows"][0]["meta"],
            json!(["Bought L", "True to size"])
        );
        let q = json!({"items": [{"id": "qst_313", "text": "Does the hood roll in?", "product_id": "prd_windbreaker",
            "author": {"name": "Da**"}, "accepted_answer": null}], "total": 1});
        let b = record_list("list_questions", &q).unwrap();
        assert_eq!(b.data["rows"][0]["body"], "Not answered yet.");
        assert_eq!(b.data["rows"][0]["href"], "/p/prd_windbreaker");
    }

    #[test]
    fn an_empty_list_still_answers_and_any_other_read_has_a_general_shape() {
        let b = record_list("list_member_coupons", &json!({"items": [], "total": 0})).unwrap();
        assert_eq!(b.title, "Your coupons · 0");
        assert_eq!(b.data["rows"], json!([]));
        let pay = json!({"id": "pay_1", "payment_status": "payment_complete", "label": "Card ending 4821",
            "amount": {"value": 64.0, "currency": "USD"}, "payment_method": "credit_card"});
        let b = record_list("get_payment", &pay).unwrap();
        let r = &b.data["rows"][0];
        assert_eq!(r["title"], "Card ending 4821");
        assert_eq!(r["status"], "payment_complete");
        assert_eq!(r["amount"]["value"], 64.0);
        assert!(record_list("get_payment", &json!(null)).is_none());
    }

    #[test]
    fn a_tracked_parcel_shows_where_it_is_and_each_scan() {
        let out = json!({"items": [{"at": "2026-11-03T08:00:00-05:00", "delivery_state": "picked_up", "location": null},
            {"at": "2026-11-04T15:05:00-05:00", "delivery_state": "delivered", "location": "Brooklyn, NY"}],
            "total": 2, "delivery_id": "dlv_1", "carrier": "UPS", "tracking_number": "1Z999"});
        let b = record_list("track_shipment", &out).unwrap();
        assert_eq!(b.data["summary"]["label"], "UPS · 1Z999");
        assert_eq!(b.data["summary"]["value"], "Delivered");
        assert_eq!(b.data["rows"][1]["subtitle"], "Brooklyn, NY");
    }

    #[test]
    fn a_collection_shows_its_sections_and_opens_its_page() {
        let c = json!({"id": "col_travel_light", "name": "Travel light", "product_count": 6,
            "valid_through": "2026-11-30T23:59:00-05:00",
            "has_part": [{"name": "Jackets", "item_list_element": ["prd_windbreaker", "prd_field_jacket", "prd_rain_parka"]}]});
        let b = record_list("get_curation", &c).unwrap();
        assert_eq!(b.data["summary"]["label"], "Travel light");
        assert_eq!(b.data["summary"]["href"], "/collections/col_travel_light");
        assert_eq!(b.data["rows"][0]["title"], "Jackets");
        assert_eq!(b.data["rows"][0]["subtitle"], "3 products");
    }

    #[test]
    fn a_policy_says_how_long_and_how() {
        let p = json!({"name": "Standard 30 days", "merchant_return_days": 30,
            "return_fees": "return_fees_customer_responsibility", "return_method": ["pickup", "drop_off"],
            "refund_type": "full_refund"});
        let b = record_list("get_return_policy", &p).unwrap();
        assert_eq!(b.data["summary"]["value"], "30 days to return");
        assert!(
            b.data["rows"][0]["meta"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m == "Send back by pickup or drop off")
        );
    }
}
