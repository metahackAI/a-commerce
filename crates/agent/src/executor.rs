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
/// Sources, in order: the steps it `uses`, the ids the person pointed at, then the latest
/// earlier step that produced the slot. A list field takes every value; a single field takes
/// the first (search results come best first). A required field that is still missing takes a
/// same-named field of a used step's result (`client_total` from a cart's `total`).
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
        let short = field.split_once('_').map(|(_, rest)| rest);
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
