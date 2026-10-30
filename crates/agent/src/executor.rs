//! Running plan steps (docs/backend/agent.md §5): what a step produced, and how the next step's
//! inputs are filled from it.
//!
//! The model never carries ids between steps. A step names earlier steps in `uses`; for every
//! slot its tool consumes (`product_id[]`, `promotion_id`), the executor takes the values those
//! steps produced and puts them in the matching input field. Writes waiting in a change set
//! have not produced anything yet, so they pass `$item[n].slot` placeholders that commerce's
//! change-set runner binds when the set runs.

use std::collections::BTreeMap;

use acom_contract::Preview;
use serde_json::{Map, Value, json};

use crate::planner::PlanStep;
use crate::tools::{SlotSpec, ToolKind, ToolSpec};

/// What one executed step left behind.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRecord {
    /// The plan step id it ran for (`s3`).
    pub step: String,
    pub tool: String,
    pub kind: ToolKind,
    pub input: Value,
    /// The read's result, or the commit's result.
    pub output: Option<Value>,
    pub preview: Option<Preview>,
    /// Position in the change set, for writes waiting for approval.
    pub item: Option<usize>,
    /// Slot values it produced, by slot name.
    pub slots: BTreeMap<String, Vec<Value>>,
}

fn push_unique(list: &mut Vec<Value>, v: Value) {
    if !list.contains(&v) {
        list.push(v);
    }
}

/// The ids one slot takes from a read: `items[].<slot>`, else the top-level field. A tool's
/// first produced slot is its own entity, so for that one an item's `id` counts too
/// (`search_products` items carry `id: prd_…`, which is a `product_id`, not a `category_id`).
fn values_in(output: &Value, slot: &SlotSpec, primary: bool) -> Vec<Value> {
    let pick = |o: &Value| -> Option<Value> {
        o.get(&slot.name)
            .filter(|v| v.is_string() || v.is_number())
            .or_else(|| {
                (primary && slot.name.ends_with("_id"))
                    .then(|| o.get("id"))
                    .flatten()
                    .filter(|v| v.is_string())
            })
            .cloned()
    };
    let mut out = Vec::new();
    match output.get("items").and_then(Value::as_array) {
        Some(items) => {
            for i in items {
                if let Some(v) = pick(i) {
                    push_unique(&mut out, v);
                }
            }
        }
        None => {
            if let Some(Value::Array(a)) = output.get(&slot.name) {
                for v in a {
                    push_unique(&mut out, v.clone());
                }
            } else if let Some(v) = pick(output) {
                out.push(v);
            }
        }
    }
    out
}

/// Slots a read produced.
pub fn slots_from_read(tool: &ToolSpec, output: &Value) -> BTreeMap<String, Vec<Value>> {
    tool.produces
        .iter()
        .enumerate()
        .map(|(n, s)| (s.name.clone(), values_in(output, s, n == 0)))
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

/// Slots a write waiting in a change set will produce: placeholders for the runner.
pub fn slots_from_item(tool: &ToolSpec, position: usize) -> BTreeMap<String, Vec<Value>> {
    tool.produces
        .iter()
        .map(|s| {
            (
                s.name.clone(),
                vec![json!(format!("$item[{position}].{}", s.name))],
            )
        })
        .collect()
}

/// Slots a committed write produced: its result's `id` for the first slot, and any slot the
/// result names.
pub fn slots_from_commit(tool: &ToolSpec, result: &Value) -> BTreeMap<String, Vec<Value>> {
    let mut out = BTreeMap::new();
    for (n, s) in tool.produces.iter().enumerate() {
        let v = result
            .get(&s.name)
            .cloned()
            .or_else(|| (n == 0).then(|| result.get("id").cloned()).flatten());
        if let Some(v) = v.filter(|v| !v.is_null()) {
            out.insert(s.name.clone(), vec![v]);
        }
    }
    out
}

/// Slots of a prepared order: its `preview_id`.
pub fn slots_from_prepared(preview: &Preview) -> BTreeMap<String, Vec<Value>> {
    BTreeMap::from([("preview_id".to_owned(), vec![json!(preview.preview_id)])])
}

/// Whether the model wrote a stand-in instead of a value for a slot field: the field's or
/// slot's own name (`"promotion_id"`), an id prefix alone (`"promo_"`), a template
/// (`"<promotion id>"`, `"{s1.id}"`), or for an id field, a value without the id's prefix.
/// Such a value is treated as missing, so the step's `uses` fill it.
fn is_stand_in(v: &Value, field: &str, slot: &str, schema: Option<&Value>) -> bool {
    let one = |s: &str| {
        let t = s.trim();
        let lower = t.to_lowercase();
        if lower == field || lower == slot || lower.trim_end_matches('s') == slot {
            return true;
        }
        if t.contains(['<', '>', '{', '}']) || t.contains("...") {
            return true;
        }
        let pattern = schema
            .and_then(|p| p.get("pattern").or_else(|| p.pointer("/items/pattern")))
            .and_then(Value::as_str);
        if let Some(prefix) = pattern
            .and_then(|p| p.strip_prefix('^'))
            .and_then(|p| p.strip_suffix("[a-z0-9_]+$"))
            .filter(|p| {
                p.ends_with('_') && p[..p.len() - 1].bytes().all(|b| b.is_ascii_lowercase())
            })
        {
            // `$item[n].…` is the change set's own placeholder, filled when it runs.
            return !t.starts_with("$item[") && (!t.starts_with(prefix) || t.len() == prefix.len());
        }
        false
    };
    match v {
        Value::String(s) => one(s),
        Value::Array(a) => !a.is_empty() && a.iter().all(|x| x.as_str().is_some_and(one)),
        _ => false,
    }
}

fn is_empty(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.trim().is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        _ => false,
    }
}

/// A step's input after binding, and what binding did.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound {
    pub input: Value,
    pub notes: Vec<String>,
}

/// Fills a step's consumed slots and missing required fields from earlier steps.
///
/// Sources, in order: the steps it `uses`, the ids the person pointed at, then (only for a step
/// that names no `uses`) the latest earlier step that produced the slot. A step that names its
/// sources gets nothing from elsewhere, so a missing id is asked for instead of guessed. A list
/// field takes every value; a single field takes the first (search results come best first). A
/// required field that is still missing takes a same-named field of a used step's result
/// (`client_total` from a cart's `total`).
pub fn bind(
    step: &PlanStep,
    tool: &ToolSpec,
    done: &BTreeMap<String, StepRecord>,
    order: &[String],
    pointed: &BTreeMap<String, Vec<String>>,
) -> Bound {
    let mut input = match &step.args {
        Value::Object(o) => o.clone(),
        _ => Map::new(),
    };
    let mut notes = Vec::new();
    let used: Vec<&StepRecord> = step.uses.iter().filter_map(|u| done.get(u)).collect();

    for slot in &tool.consumes {
        let Some((field, is_list)) = tool.field_for_slot(&slot.name) else {
            continue;
        };
        let schema = tool.properties().and_then(|p| p.get(&field));
        let stand_in = input
            .get(&field)
            .is_some_and(|v| is_stand_in(v, &field, &slot.name, schema));
        if stand_in {
            notes.push(format!(
                "{field}: {} is a stand-in, not a value",
                input[&field]
            ));
            input.remove(&field);
        }
        if !is_empty(input.get(&field)) {
            continue;
        }
        let from_uses = used
            .iter()
            .find_map(|r| r.slots.get(&slot.name).map(|v| (r.step.clone(), v.clone())));
        let from_pointed = || {
            pointed.get(&slot.name).filter(|v| !v.is_empty()).map(|v| {
                (
                    "what you pointed at".to_owned(),
                    v.iter().map(|s| json!(s)).collect::<Vec<_>>(),
                )
            })
        };
        let from_latest = || {
            if !step.uses.is_empty() {
                return None;
            }
            order
                .iter()
                .rev()
                .filter_map(|id| done.get(id))
                .find_map(|r| r.slots.get(&slot.name).map(|v| (r.step.clone(), v.clone())))
        };
        let Some((source, values)) = from_uses.or_else(from_pointed).or_else(from_latest) else {
            continue;
        };
        if values.is_empty() {
            continue;
        }
        // The model meant one of several (a tier among all tiers) but did not say which: leave it
        // missing, so the plan is corrected rather than the first one guessed.
        if stand_in && !is_list && values.len() > 1 {
            notes.push(format!(
                "{field}: {source} has {} candidates; which one is not said",
                values.len()
            ));
            continue;
        }
        let value = if is_list {
            Value::Array(values.clone())
        } else {
            values[0].clone()
        };
        notes.push(if is_list || values.len() == 1 {
            format!("{field}: {} value(s) from {source}", values.len())
        } else {
            format!("{field}: the first of {} from {source}", values.len())
        });
        input.insert(field, value);
    }

    for field in tool.required() {
        if !is_empty(input.get(field)) {
            continue;
        }
        // `client_total` may take a result's `total`; an id never takes another entity's `id`.
        let short = field
            .split_once('_')
            .map(|(_, rest)| rest)
            .filter(|_| !field.ends_with("_id"));
        let found = used.iter().find_map(|r| {
            let out = r.output.as_ref()?;
            out.get(field)
                .or_else(|| short.and_then(|s| out.get(s)))
                .filter(|v| !v.is_null())
                .map(|v| (r.step.clone(), v.clone()))
        });
        if let Some((source, v)) = found {
            notes.push(format!("{field}: from {source}'s result"));
            input.insert(field.to_owned(), v);
        }
    }
    Bound {
        input: Value::Object(input),
        notes,
    }
}

/// The step a read should run once per row of, when the plan meant it but did not say `each`: the
/// read needs one id it was not given (or got a stand-in for), and the one step it `uses`
/// returned several ("evaluate the requests" after a search). Writes are never fanned out on a
/// guess.
pub fn implied_each(
    step: &PlanStep,
    tool: &ToolSpec,
    done: &BTreeMap<String, StepRecord>,
) -> Option<String> {
    if step.each.is_some() || tool.is_write() || step.uses.len() != 1 {
        return None;
    }
    let source = done.get(&step.uses[0])?;
    // A ranked text search means its best hit ("the wool overcoat"), not every hit.
    if source
        .input
        .get("query")
        .is_some_and(|q| !is_empty(Some(q)))
    {
        return None;
    }
    tool.consumes.iter().find_map(|slot| {
        let (field, is_list) = tool.field_for_slot(&slot.name)?;
        let given = step.args.get(&field);
        let schema = tool.properties().and_then(|p| p.get(&field));
        let missing =
            is_empty(given) || given.is_some_and(|v| is_stand_in(v, &field, &slot.name, schema));
        let several = source.slots.get(&slot.name).is_some_and(|v| v.len() > 1);
        (!is_list && missing && several).then(|| step.uses[0].clone())
    })
}

/// The rows of a result a step can run once per row for (`each`): its `items` (or a block's
/// `requests`, `picks`, `lines`, a screen grid's `list`), or the result itself when it is one
/// object.
pub fn rows(output: &Value) -> Vec<Value> {
    for key in ["items", "requests", "picks", "lines", "products", "list"] {
        if let Some(Value::Array(a)) = output.get(key) {
            return a.iter().filter(|r| r.is_object()).cloned().collect();
        }
    }
    match output {
        Value::Array(a) => a.iter().filter(|r| r.is_object()).cloned().collect(),
        Value::Object(_) => vec![output.clone()],
        _ => Vec::new(),
    }
}

/// Whether a row has every field `when` names, with the same value.
pub fn row_matches(row: &Value, when: Option<&Value>) -> bool {
    let Some(Value::Object(w)) = when else {
        return true;
    };
    w.iter().all(|(k, v)| row.get(k) == Some(v))
}

/// The ids a list of rows carries, by field name (`claim_id` → […]): what rows from a block
/// shown earlier produce when a step uses them.
pub fn slots_from_rows(rows: &[Value]) -> BTreeMap<String, Vec<Value>> {
    let mut out: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for r in rows {
        for (k, v) in r.as_object().into_iter().flatten() {
            if k.ends_with("_id") && v.is_string() {
                push_unique(out.entry(k.clone()).or_default(), v.clone());
            }
        }
    }
    out
}

/// One run of an `each` step: the step's own arguments, then its consumed slots and missing
/// required fields from the row's same-named fields (`claim_id`, `customer_id`).
pub fn bind_row(step: &PlanStep, tool: &ToolSpec, row: &Value) -> Bound {
    let mut input = match &step.args {
        Value::Object(o) => o.clone(),
        _ => Map::new(),
    };
    let mut notes = Vec::new();
    for slot in &tool.consumes {
        let Some((field, is_list)) = tool.field_for_slot(&slot.name) else {
            continue;
        };
        let schema = tool.properties().and_then(|p| p.get(&field));
        if input
            .get(&field)
            .is_some_and(|v| is_stand_in(v, &field, &slot.name, schema))
        {
            input.remove(&field);
        }
        if !is_empty(input.get(&field)) {
            continue;
        }
        // The row's field named like the slot, else its own `id` when it has the field's prefix
        // (search rows name a product by `id`).
        let prefix = schema
            .and_then(|p| p.get("pattern").or_else(|| p.pointer("/items/pattern")))
            .and_then(Value::as_str)
            .and_then(|p| p.strip_prefix('^'))
            .and_then(|p| p.strip_suffix("[a-z0-9_]+$"));
        let own_id = || {
            row.get("id").filter(|v| {
                v.as_str()
                    .is_some_and(|s| prefix.is_some_and(|p| s.starts_with(p)))
            })
        };
        if let Some(v) = row
            .get(&slot.name)
            .filter(|v| v.is_string())
            .or_else(own_id)
        {
            input.insert(field.clone(), if is_list { json!([v]) } else { v.clone() });
            notes.push(format!("{field}: from the row"));
        }
    }
    for field in tool.required() {
        if is_empty(input.get(field))
            && let Some(v) = row.get(field).filter(|v| !v.is_null())
        {
            input.insert(field.to_owned(), v.clone());
            notes.push(format!("{field}: from the row"));
        }
    }
    Bound {
        input: Value::Object(input),
        notes,
    }
}

/// A correction keeps what the item had and changes what the model said.
pub fn merge(base: &Value, changes: &Value) -> Value {
    match (base, changes) {
        (Value::Object(b), Value::Object(c)) => {
            let mut out = b.clone();
            for (k, v) in c {
                out.insert(k.clone(), v.clone());
            }
            Value::Object(out)
        }
        (_, c) if c.is_object() => c.clone(),
        (b, _) => b.clone(),
    }
}

/// The remembered line a preference write returned, or one built from what it stored.
pub fn memory_of(result: &Value, preview: Option<&Preview>) -> Option<(String, String)> {
    let id = result
        .pointer("/memory/memory_id")
        .or_else(|| result.pointer("/undo/target"))
        .or_else(|| result.get("id"))
        .and_then(Value::as_str)?
        .to_owned();
    let text = result
        .pointer("/memory/text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            let to = &preview?.changes.first()?.to;
            to.get("value")
                .and_then(Value::as_str)
                .map(|v| format!("Remembered: {v}"))
        })?;
    Some((id, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::fixtures::{get, manifest};

    fn record(step: &str, tool: &ToolSpec, output: Value) -> StepRecord {
        StepRecord {
            step: step.into(),
            tool: tool.tool.clone(),
            kind: tool.kind,
            input: json!({}),
            slots: slots_from_read(tool, &output),
            output: Some(output),
            preview: None,
            item: None,
        }
    }

    fn step(id: &str, tool: &str, args: Value, uses: &[&str]) -> PlanStep {
        PlanStep {
            id: id.into(),
            tool: tool.into(),
            args,
            uses: uses.iter().map(|u| (*u).to_owned()).collect(),
            each: None,
            when: None,
        }
    }

    #[test]
    fn ids_flow_through_slots_never_through_the_model() {
        let m = manifest();
        let search = record(
            "s1",
            get(&m, "search_products"),
            json!({"items": [{"product_id": "prd_field_jacket"}, {"product_id": "prd_windbreaker"},
                             {"id": "prd_seersucker_overshirt"}], "total": 42}),
        );
        assert_eq!(search.slots["product_id"].len(), 3);
        assert!(
            !search.slots.contains_key("category_id"),
            "an item's id is not its category"
        );
        let mut done = BTreeMap::from([("s1".to_owned(), search)]);
        let order = vec!["s1".to_owned()];

        let sale = get(&m, "set_sale_prices");
        let b = bind(
            &step(
                "s3",
                "set_sale_prices",
                json!({"scope_mode": "products", "percent_off": 20}),
                &["s1"],
            ),
            sale,
            &done,
            &order,
            &BTreeMap::new(),
        );
        assert_eq!(
            b.input["product_ids"],
            json!([
                "prd_field_jacket",
                "prd_windbreaker",
                "prd_seersucker_overshirt"
            ])
        );
        assert!(b.notes[0].contains("3 value(s) from s1"));

        // The sale waits in the change set as item 1; the coupon gets a placeholder.
        let mut s3 = record("s3", sale, json!({}));
        s3.slots = slots_from_item(sale, 1);
        done.insert("s3".into(), s3);
        let coupon = bind(
            &step(
                "s4",
                "create_coupon",
                json!({"name": "VIP weekend"}),
                &["s3"],
            ),
            get(&m, "create_coupon"),
            &done,
            &["s1".into(), "s3".into()],
            &BTreeMap::new(),
        );
        assert_eq!(coupon.input["promotion_id"], "$item[1].promotion_id");
    }

    #[test]
    fn stand_ins_for_ids_are_filled_from_uses_and_real_values_stay() {
        let m = manifest();
        let sale = get(&m, "set_sale_prices");
        let mut s3 = record("s3", sale, json!({}));
        s3.slots = slots_from_item(sale, 1);
        let done = BTreeMap::from([("s3".to_owned(), s3)]);
        let order = vec!["s3".to_owned()];
        for stand_in in [
            "promotion_id",
            "promo_",
            "<promotion id>",
            "{s3.promotion_id}",
        ] {
            let b = bind(
                &step(
                    "s4",
                    "create_coupon",
                    json!({"code": "VIPWEEKEND20", "promotion_id": stand_in}),
                    &["s3"],
                ),
                get(&m, "create_coupon"),
                &done,
                &order,
                &BTreeMap::new(),
            );
            assert_eq!(
                b.input["promotion_id"], "$item[1].promotion_id",
                "{stand_in}"
            );
            assert!(b.notes.iter().any(|n| n.contains("stand-in")), "{stand_in}");
        }
        // A real id the model was given stays as written.
        let b = bind(
            &step(
                "s4",
                "create_coupon",
                json!({"promotion_id": "promo_autumn_knit"}),
                &["s3"],
            ),
            get(&m, "create_coupon"),
            &done,
            &order,
            &BTreeMap::new(),
        );
        assert_eq!(b.input["promotion_id"], "promo_autumn_knit");
        assert!(is_stand_in(
            &json!("tier_id"),
            "valid_for_member_tier",
            "tier_id",
            None
        ));
        assert!(!is_stand_in(
            &json!("vip"),
            "valid_for_member_tier",
            "tier_id",
            None
        ));

        // One product among several: the stand-in is dropped and nothing is guessed.
        let search = record(
            "s1",
            get(&m, "search_products"),
            json!({"items": [{"product_id": "prd_a"}, {"product_id": "prd_b"}, {"product_id": "prd_c"}]}),
        );
        let done = BTreeMap::from([("s1".to_owned(), search)]);
        let b = bind(
            &step(
                "s2",
                "add_to_cart",
                json!({"product_id": "product_id", "size": "M"}),
                &["s1"],
            ),
            get(&m, "add_to_cart"),
            &done,
            &["s1".into()],
            &BTreeMap::new(),
        );
        assert!(b.input.get("product_id").is_none(), "{:?}", b.input);
        assert!(
            b.notes.iter().any(|n| n.contains("3 candidates")),
            "{:?}",
            b.notes
        );
    }

    #[test]
    fn a_read_meant_for_every_row_runs_per_row_and_rows_replace_stand_ins() {
        let m = manifest();
        let search = record(
            "s1",
            get(&m, "search_products"),
            json!({"items": [{"product_id": "prd_a"}, {"product_id": "prd_b"}]}),
        );
        let done = BTreeMap::from([("s1".to_owned(), search)]);
        let get_product: ToolSpec = serde_json::from_value(json!({
            "tool": "get_product", "address": "catalog.product.get_product", "kind": "read",
            "summary": "One product.", "consumes": [{"name": "product_id", "many": false}],
            "produces": [], "checks": [], "group": "offer", "method": "GET", "path": "/v1/x",
            "errors": [], "audience": "both", "action_type": "SearchAction", "person_scope": "",
            "input_schema": {"type": "object", "additionalProperties": false, "required": ["product_id"],
                "properties": {"product_id": {"type": "string", "pattern": "^prd_[a-z0-9_]+$", "description": "Product."}}}
        }))
        .unwrap();
        let read = &get_product;
        let meant = step(
            "s2",
            "get_product",
            json!({"product_id": "product_id"}),
            &["s1"],
        );
        assert_eq!(implied_each(&meant, read, &done).as_deref(), Some("s1"));
        let chosen = step("s2", "get_product", json!({"product_id": "prd_b"}), &["s1"]);
        assert_eq!(
            implied_each(&chosen, read, &done),
            None,
            "the plan chose one"
        );
        let write = step("s2", "add_to_cart", json!({"size": "M"}), &["s1"]);
        assert_eq!(
            implied_each(&write, get(&m, "add_to_cart"), &done),
            None,
            "never a write"
        );
        let b = bind_row(&meant, read, &json!({"product_id": "prd_b"}));
        assert_eq!(b.input["product_id"], "prd_b");
        let b = bind_row(&meant, read, &json!({"id": "prd_c", "name": "C"}));
        assert_eq!(
            b.input["product_id"], "prd_c",
            "a search row names its product by id"
        );

        // A ranked text search means its best hit: no fan-out.
        let mut searched = record(
            "s1",
            get(&m, "search_products"),
            json!({"items": [{"product_id": "prd_a"}, {"product_id": "prd_b"}]}),
        );
        searched.input = json!({"query": "wool overcoat"});
        let done = BTreeMap::from([("s1".to_owned(), searched)]);
        assert_eq!(implied_each(&meant, read, &done), None);
    }

    #[test]
    fn pointed_ids_and_result_fields_fill_what_is_missing() {
        let m = manifest();
        let pointed =
            BTreeMap::from([("product_id".to_owned(), vec!["prd_windbreaker".to_owned()])]);
        let b = bind(
            &step("s1", "add_to_cart", json!({"size": "M"}), &[]),
            get(&m, "add_to_cart"),
            &BTreeMap::new(),
            &[],
            &pointed,
        );
        assert_eq!(
            b.input,
            json!({"size": "M", "product_id": "prd_windbreaker"})
        );

        let cart = record(
            "s1",
            get(&m, "get_cart"),
            json!({"id": "cart_1", "lines": [], "total": {"value": 71.20, "currency": "USD"}}),
        );
        assert_eq!(cart.slots["cart_id"], [json!("cart_1")]);
        let done = BTreeMap::from([("s1".to_owned(), cart)]);
        let b = bind(
            &step(
                "s2",
                "prepare_order",
                json!({"delivery_mode": "saved"}),
                &["s1"],
            ),
            get(&m, "prepare_order"),
            &done,
            &["s1".into()],
            &BTreeMap::new(),
        );
        assert_eq!(b.input["client_total"]["currency"], "USD");
        assert!(b.notes.iter().any(|n| n.contains("client_total")));
    }

    #[test]
    fn the_models_values_win_and_corrections_merge() {
        let m = manifest();
        let search = record(
            "s1",
            get(&m, "search_products"),
            json!({"items": [{"product_id": "prd_a"}]}),
        );
        let done = BTreeMap::from([("s1".to_owned(), search)]);
        let b = bind(
            &step("s2", "add_to_cart", json!({"product_id": "prd_b"}), &["s1"]),
            get(&m, "add_to_cart"),
            &done,
            &["s1".into()],
            &BTreeMap::new(),
        );
        assert_eq!(b.input["product_id"], "prd_b");
        assert_eq!(
            merge(
                &json!({"percent_off": 20, "name": "VIP weekend"}),
                &json!({"exclude_low_stock": true})
            ),
            json!({"percent_off": 20, "name": "VIP weekend", "exclude_low_stock": true})
        );
    }

    #[test]
    fn a_step_that_names_its_sources_takes_ids_only_from_them() {
        let m = manifest();
        let search = record(
            "s1",
            get(&m, "search_products"),
            json!({"items": [{"product_id": "prd_wool_overcoat"}]}),
        );
        let cart = record(
            "s2",
            get(&m, "get_cart"),
            json!({"id": "cart_1", "lines": []}),
        );
        let done = BTreeMap::from([("s1".to_owned(), search), ("s2".to_owned(), cart)]);
        let order = vec!["s1".to_owned(), "s2".to_owned()];
        let named = bind(
            &step("s3", "add_to_cart", json!({"size": "M"}), &["s2"]),
            get(&m, "add_to_cart"),
            &done,
            &order,
            &BTreeMap::new(),
        );
        assert!(
            named.input.get("product_id").is_none(),
            "s2 has no product; s1's is not taken"
        );
        let unnamed = bind(
            &step("s3", "add_to_cart", json!({"size": "M"}), &[]),
            get(&m, "add_to_cart"),
            &done,
            &order,
            &BTreeMap::new(),
        );
        assert_eq!(unnamed.input["product_id"], "prd_wool_overcoat");
    }

    #[test]
    fn rows_fan_out_with_their_own_ids() {
        let m = manifest();
        let queue = json!({"items": [
            {"claim_id": "clm_20402", "suggested": "refund", "customer_id": "cus_alex"},
            {"claim_id": "clm_20366", "suggested": "ask_for_photos"},
            "not a row"], "total": 2});
        let rows = rows(&queue);
        assert_eq!(rows.len(), 2);
        let refunds: Vec<&Value> = rows
            .iter()
            .filter(|r| row_matches(r, Some(&json!({"suggested": "refund"}))))
            .collect();
        assert_eq!(refunds.len(), 1);
        assert!(row_matches(&rows[1], None));
        let slots = slots_from_rows(&rows);
        assert_eq!(slots["claim_id"].len(), 2);
        assert_eq!(slots["customer_id"], [json!("cus_alex")]);
        let b = bind_row(
            &step("s2", "add_to_cart", json!({"size": "L"}), &[]),
            get(&m, "add_to_cart"),
            &json!({"product_id": "prd_windbreaker"}),
        );
        assert_eq!(
            b.input,
            json!({"size": "L", "product_id": "prd_windbreaker"})
        );
        assert_eq!(rows_of_one(), 1);
    }

    fn rows_of_one() -> usize {
        rows(&json!({"product_id": "prd_a"})).len()
    }

    #[test]
    fn commits_and_memories() {
        let m = manifest();
        let r = json!({"ok": true, "id": "mem_12",
            "memory": {"memory_id": "mem_12", "text": "Remembered: weight and packed size matter to you when traveling"},
            "undo": {"by": "person", "target": "mem_12", "until": null}});
        assert_eq!(
            slots_from_commit(get(&m, "update_my_preferences"), &r)["memory_id"],
            [json!("mem_12")]
        );
        let (id, text) = memory_of(&r, None).unwrap();
        assert_eq!(id, "mem_12");
        assert!(text.starts_with("Remembered: weight"));
        assert!(memory_of(&json!({"ok": true}), None).is_none());
    }
}
