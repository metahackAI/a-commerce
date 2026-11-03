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
            } else if let Some(list) = listed(output, &slot.name) {
                // A record that lists its own (`addresses` on a profile for `address_id`).
                for i in list {
                    if let Some(v) = i
                        .get(&slot.name)
                        .or_else(|| i.get("id"))
                        .filter(|v| v.is_string())
                    {
                        push_unique(&mut out, v.clone());
                    }
                }
            }
        }
    }
    out
}

/// The list a record keeps of what an id slot names: `addresses` for `address_id`.
fn listed<'a>(output: &'a Value, slot: &str) -> Option<&'a Vec<Value>> {
    let entity = slot.strip_suffix("_id")?;
    let plurals = [
        format!("{entity}s"),
        format!("{entity}es"),
        entity
            .strip_suffix('y')
            .map(|e| format!("{e}ies"))
            .unwrap_or_default(),
    ];
    plurals
        .iter()
        .filter(|p| !p.is_empty())
        .find_map(|p| output.get(p).and_then(Value::as_array))
}

/// Slots a read produced. A read of one record (`get_promotion`) also gives the id it was read
/// by, when the record carries it: the next step can take the promotion from it as well as from
/// the search before it.
pub fn slots_from_read(tool: &ToolSpec, output: &Value) -> BTreeMap<String, Vec<Value>> {
    let mut out: BTreeMap<String, Vec<Value>> = tool
        .produces
        .iter()
        .enumerate()
        .map(|(n, s)| (s.name.clone(), values_in(output, s, n == 0)))
        .filter(|(_, v)| !v.is_empty())
        .collect();
    if output.get("items").is_none() {
        for slot in tool.consumes.iter().filter(|s| !s.many) {
            let prefix = tool
                .field_for_slot(&slot.name)
                .and_then(|(f, _)| tool.properties().and_then(|p| p.get(&f)))
                .and_then(id_prefix);
            let own = [output.get(&slot.name), output.get("id")]
                .into_iter()
                .flatten()
                .find(|v| {
                    v.as_str()
                        .zip(prefix)
                        .is_some_and(|(s, p)| s.strip_prefix(p).is_some_and(|r| r.starts_with('_')))
                });
            if let Some(v) = own {
                out.entry(slot.name.clone())
                    .or_insert_with(|| vec![v.clone()]);
            }
        }
    }
    out
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

/// The ids of one slot that a write's changes are about (`product_id` in each price change of a
/// sale), in the preview's order.
fn changed_ids(preview: &Preview, slot: &str) -> Vec<Value> {
    let mut ids = Vec::new();
    for c in &preview.changes {
        if let Some(v) = [&c.to, &c.from]
            .into_iter()
            .find_map(|side| side.get(slot).filter(|v| v.is_string()))
        {
            push_unique(&mut ids, v.clone());
        }
    }
    ids
}

/// Slots of a prepared order: its `preview_id`.
pub fn slots_from_prepared(preview: &Preview) -> BTreeMap<String, Vec<Value>> {
    BTreeMap::from([("preview_id".to_owned(), vec![json!(preview.preview_id)])])
}

/// The id prefix of an id field (or of its items): `ord` for `^ord_[a-z0-9_]+$` and for the
/// widened pattern of a field that also takes what people say.
fn id_prefix(schema: &Value) -> Option<&str> {
    schema
        .get("pattern")
        .or_else(|| schema.pointer("/items/pattern"))
        .and_then(Value::as_str)
        .and_then(acom_core::handle::id_prefix)
}

/// Whether the manifest marks the field (or its items) as taking what people say.
fn takes_handle(schema: Option<&Value>) -> bool {
    schema.is_some_and(|p| p.get("x-handle").is_some() || p.pointer("/items/x-handle").is_some())
}

/// Whether the model wrote a stand-in instead of a value for a slot field: the field's or
/// slot's own name (`"promotion_id"`), an id prefix alone (`"promo_"`), a template
/// (`"<promotion id>"`, `"{s1.id}"`), or for an id field, a value without the id's prefix or
/// with more after it than an id is made of (`"promo_???"`).
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
        if let Some(prefix) = schema.and_then(id_prefix) {
            // A value people say where the field takes it (an order number): the server finds
            // the record (`acom_core::handle`).
            if takes_handle(schema) && acom_core::handle::read_value(prefix, t).is_some() {
                return false;
            }
            let prefix = format!("{prefix}_");
            // `$item[n].…` is the change set's own placeholder, filled when it runs. A step's id
            // in place of `n` is not one: the runner would never fill it.
            if let Some(rest) = t.strip_prefix("$item[") {
                let n = rest.split_once("].").map_or("", |(n, _)| n);
                return n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit());
            }
            // After the prefix only what an id is made of: `promo_???` marks where one goes.
            return t.strip_prefix(&prefix).is_none_or(|rest| {
                rest.is_empty()
                    || !rest
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            });
        }
        false
    };
    match v {
        Value::String(s) => one(s),
        Value::Array(a) => !a.is_empty() && a.iter().all(|x| x.as_str().is_some_and(one)),
        _ => false,
    }
}

/// An object that lacks one of its own required parts (`{"value": null, "currency": "USD"}` for a
/// total): written as a shape, not a value, so it is filled like a missing field.
fn is_hollow(v: Option<&Value>, schema: Option<&Value>) -> bool {
    let (Some(Value::Object(o)), Some(required)) = (
        v,
        schema
            .and_then(|s| s.get("required"))
            .and_then(Value::as_array),
    ) else {
        return false;
    };
    required
        .iter()
        .filter_map(Value::as_str)
        .any(|k| is_empty(o.get(k)))
}

/// Money written as zero (`{"value": 0, "currency": "USD"}`): what a model writes for a figure it
/// has not read. Where a step it builds on read that figure, the read wins. Plain numbers (a
/// quantity, a position) are left alone: zero is often meant there.
fn is_zero_amount(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Object(o)) if o.contains_key("currency")
        && o.get("value").and_then(Value::as_f64).is_some_and(|x| x == 0.0))
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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Bound {
    pub input: Value,
    pub notes: Vec<String>,
    /// A required field taken from a used step's result that came back empty (`lines` of a
    /// reorder suggestion with nothing to order): the step has nothing to act on.
    pub empty: Option<String>,
    /// Fields a result could fill with several values and nothing says which: the worker is
    /// asked, as for a missing required field.
    pub undecided: Vec<String>,
}

/// A value the plan wrote as a reference to a step it uses (`uses.suggest.lines`,
/// `suggest.lines`, or just `uses.suggest` for the same-named field): that result's value.
fn referenced(v: &Value, field: &str, used: &[&StepRecord]) -> Option<(String, Value)> {
    let s = v.as_str()?.trim();
    let s = s
        .strip_prefix("uses.")
        .or_else(|| s.strip_prefix("uses:"))
        .unwrap_or(s);
    let mut path = s.split('.');
    let head = path.next()?;
    let r = used.iter().find(|r| r.step == head)?;
    let mut at = r.output.as_ref()?;
    let rest: Vec<&str> = path.collect();
    for key in if rest.is_empty() { vec![field] } else { rest } {
        at = at.get(key)?;
    }
    Some((r.step.clone(), at.clone()))
}

/// Whether an id is opaque: its part after the prefix has a digit (`ord_01m6…`, `ord_12345`).
/// Such an id cannot come from words the way `tier_vip` or `cus_jamie` can.
fn is_opaque(id: &str, prefix: &str) -> bool {
    id.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('_'))
        .is_some_and(|rest| rest.bytes().any(|b| b.is_ascii_digit()))
}

/// Opaque ids a model wrote that nothing it was shown holds (`seen`: what the models read this
/// turn, the results, the request and the screen): made up, so they are taken out (a field, or
/// an item of a list) and each is returned with where it was. Binding then fills the field from
/// the steps it uses, or it is missing and asked for, instead of calling with an id that names
/// nothing.
pub fn unseen_ids(v: &mut Value, schema: &Value, seen: &str, path: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let made_up = |s: &str, schema: &Value| {
        schema
            .get("pattern")
            .and_then(Value::as_str)
            .and_then(acom_core::handle::id_prefix)
            .is_some_and(|p| is_opaque(s, p) && !seen.contains(s))
    };
    match v {
        Value::Object(o) => {
            let Some(props) = schema.get("properties").and_then(Value::as_object) else {
                return out;
            };
            let keys: Vec<String> = o.keys().cloned().collect();
            for k in keys {
                let Some(ps) = props.get(&k) else { continue };
                let at = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                if let Some(s) = o[&k].as_str()
                    && made_up(s, ps)
                {
                    out.push((at, s.to_owned()));
                    o.remove(&k);
                    continue;
                }
                if let Some(inner) = o.get_mut(&k) {
                    out.extend(unseen_ids(inner, ps, seen, &at));
                }
            }
        }
        Value::Array(items) => {
            let Some(is) = schema.get("items") else {
                return out;
            };
            items.retain(|x| match x.as_str() {
                Some(s) if made_up(s, is) => {
                    out.push((format!("{path}[]"), s.to_owned()));
                    false
                }
                _ => true,
            });
            for (n, x) in items.iter_mut().enumerate() {
                out.extend(unseen_ids(x, is, seen, &format!("{path}[{n}]")));
            }
        }
        _ => {}
    }
    out
}

/// The one row of a search by words whose name is those words, whole (`query: "Packable
/// windbreaker"` and the product of that name): its value for the slot.
fn named_by_query(r: &StepRecord, slot: &str, values: &[Value]) -> Option<Value> {
    let words = |t: &str| -> Vec<String> {
        t.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let query = words(r.input.get("query")?.as_str()?);
    if query.is_empty() {
        return None;
    }
    let mut found: Vec<Value> = Vec::new();
    for row in r.output.as_ref().map(rows).unwrap_or_default() {
        let Some(id) = row
            .get(slot)
            .or_else(|| row.get("id"))
            .filter(|v| values.contains(v))
        else {
            continue;
        };
        if row
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|n| words(n) == query)
            && !found.contains(id)
        {
            found.push(id.clone());
        }
    }
    (found.len() == 1).then(|| found.remove(0))
}

/// Whether a read came back with no rows (a search that found no one).
fn found_nothing(r: &StepRecord) -> bool {
    r.kind == ToolKind::Read
        && r.output
            .as_ref()
            .is_some_and(|o| o.get("items").is_some() && rows(o).is_empty())
}

/// Id fields the plan filled with the name of a step it uses (`"customer_id": "find_jamie"`
/// with `uses: ["find_jamie"]`): that step's ids of the field's kind, also where no slot of the
/// tool names the field. Left alone, the name would be read as an id and given the id's prefix
/// (`cus_find_jamie`). A step that found nothing leaves nothing to build on: [`Bound::empty`],
/// so the read is not run unfiltered and no model is asked to make an id up.
fn step_named(
    input: &mut Map<String, Value>,
    tool: &ToolSpec,
    used: &[&StepRecord],
    notes: &mut Vec<String>,
    empty: &mut Option<String>,
    undecided: &mut Vec<String>,
) {
    let fields: Vec<String> = input.keys().cloned().collect();
    for field in fields {
        let Some(name) = input[&field].as_str().map(str::trim) else {
            continue;
        };
        let name = name
            .strip_prefix("uses.")
            .or_else(|| name.strip_prefix("uses:"))
            .unwrap_or(name);
        let Some(r) = used.iter().find(|r| r.step == name) else {
            continue;
        };
        let schema = tool.properties().and_then(|p| p.get(&field));
        let Some(prefix) = schema.and_then(id_prefix) else {
            continue;
        };
        let source = r.step.clone();
        input.remove(&field);
        if found_nothing(r) {
            notes.push(format!("{field}: {source} found nothing"));
            empty.get_or_insert_with(|| format!("{source} found nothing for {field}"));
            continue;
        }
        let kind = format!("{prefix}_");
        let values = r.slots.get(&field).cloned().or_else(|| {
            r.slots
                .values()
                .find(|vs| {
                    !vs.is_empty()
                        && vs
                            .iter()
                            .all(|v| v.as_str().is_some_and(|s| s.starts_with(&kind)))
                })
                .cloned()
        });
        let Some(values) = values.filter(|v| !v.is_empty()) else {
            notes.push(format!("{field}: {source} has no {kind} id"));
            continue;
        };
        let is_list = schema.and_then(|s| s.get("type")).and_then(Value::as_str) == Some("array");
        if is_list {
            notes.push(format!("{field}: {} value(s) from {source}", values.len()));
            input.insert(field, Value::Array(values));
        } else if values.len() == 1 {
            notes.push(format!("{field}: from {source}'s result"));
            input.insert(field, values[0].clone());
        } else {
            notes.push(format!(
                "{field}: {source} has {} candidates; which one is not said",
                values.len()
            ));
            undecided.push(field);
        }
    }
}

/// Fills a step's consumed slots and missing required fields from earlier steps.
///
/// Sources, in order: the steps it `uses`, the ids the person pointed at, then (only for a step
/// that names no `uses`) the latest earlier step that produced the slot. A step that names its
/// sources gets nothing from elsewhere, so a missing id is asked for instead of guessed. A list
/// field takes every value; a single field takes the first only from a search by words (those
/// results come best first), and is otherwise left undecided when there are several. A
/// required field that is still missing takes a same-named field of a used step's result
/// (`client_total` from a cart's `total`). A field the plan wrote as a reference to a used step
/// (`uses.suggest.lines`) takes that value. A required list taken from a result that came back
/// empty is reported in [`Bound::empty`] rather than left for a model to fill.
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
    let mut empty = None;
    let mut undecided_refs = Vec::new();
    step_named(
        &mut input,
        tool,
        &used,
        &mut notes,
        &mut empty,
        &mut undecided_refs,
    );
    let mut undecided = undecided_refs;
    let required = tool.required();
    let mut taken = |field: &str, source: &str, v: &Value, notes: &mut Vec<String>| {
        notes.push(format!("{field}: from {source}'s result"));
        if required.contains(&field) && matches!(v, Value::Array(a) if a.is_empty()) {
            empty.get_or_insert_with(|| format!("{field} from {source} is empty"));
        }
    };

    let fields: Vec<String> = input.keys().cloned().collect();
    for field in fields {
        if let Some((source, v)) = referenced(&input[&field], &field, &used) {
            taken(&field, &source, &v, &mut notes);
            input.insert(field, v);
        }
    }

    for slot in &tool.consumes {
        let Some((field, is_list)) = tool.field_for_slot(&slot.name) else {
            continue;
        };
        let schema = tool.properties().and_then(|p| p.get(&field));
        // A step that builds on a write of this plan takes what that write will produce: a coupon
        // for this sale. An id written next to `uses` was copied from something the model read.
        let own_write = used
            .iter()
            .find_map(|r| r.item.and_then(|_| r.slots.get(&slot.name)))
            .filter(|_| !is_list)
            .and_then(|v| v.first());
        if let Some(v) = own_write
            && input.get(&field).is_some_and(|w| w != v)
        {
            notes.push(format!(
                "{field}: {} replaced by what this plan produces",
                input[&field]
            ));
            input.insert(field, v.clone());
            continue;
        }
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
        // One of several, and nothing says which (a tier among all tiers, a category among all
        // categories). Only a search by words puts what was meant first; a read that lists rows
        // has no such order, so the first would be a wrong target nobody asked for.
        let by_words = done
            .get(&source)
            .is_none_or(|r| !is_empty(r.input.get("query")));
        // The search's words are one row's whole name ("Packable windbreaker"): that row.
        if !is_list
            && values.len() > 1
            && let Some(v) = done
                .get(&source)
                .and_then(|r| named_by_query(r, &slot.name, &values))
        {
            notes.push(format!("{field}: the one {source} found by its name"));
            input.insert(field, v);
            continue;
        }
        if !is_list && values.len() > 1 && (stand_in || !by_words) {
            notes.push(format!(
                "{field}: {source} has {} candidates; which one is not said",
                values.len()
            ));
            undecided.push(field);
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

    // A slot whose field sits inside an object (a banner's picture): the first value of the step
    // it `uses`, else of the latest step that has one. A person sees which in the preview.
    for slot in &tool.consumes {
        if tool.field_for_slot(&slot.name).is_some() {
            continue;
        }
        let Some((object, field)) = tool.nested_field_for_slot(&slot.name) else {
            continue;
        };
        let schema = tool
            .properties()
            .and_then(|p| p.get(&object))
            .and_then(|o| o.get("properties"))
            .and_then(|p| p.get(&field));
        let Some(fields) = input.get_mut(&object).and_then(Value::as_object_mut) else {
            continue;
        };
        // Here a placeholder only counts if a step of this turn really produces it.
        let produced = |v: &Value| {
            done.values()
                .any(|r| r.slots.get(&slot.name).is_some_and(|s| s.contains(v)))
        };
        let stand_in = fields.get(&field).is_some_and(|v| {
            is_stand_in(v, &field, &slot.name, schema)
                || (v.as_str().is_some_and(|s| s.starts_with("$item[")) && !produced(v))
        });
        if stand_in {
            notes.push(format!(
                "{object}.{field}: {} is a stand-in, not a value",
                fields[&field]
            ));
            fields.remove(&field);
        }
        if !is_empty(fields.get(&field)) {
            continue;
        }
        let found = used
            .iter()
            .copied()
            .chain(order.iter().rev().filter_map(|id| done.get(id)))
            .find_map(|r| {
                let first = r.slots.get(&slot.name)?.first()?;
                Some((r.step.clone(), first.clone(), r.slots[&slot.name].len()))
            });
        if let Some((source, value, count)) = found {
            notes.push(format!(
                "{object}.{field}: the first of {count} from {source}"
            ));
            fields.insert(field, value);
            continue;
        }
        // Else a record a write it uses changes: a banner for a sale shows one of the sale's
        // products.
        let changed = used.iter().find_map(|r| {
            let ids = changed_ids(r.preview.as_ref()?, &slot.name);
            Some((r.step.clone(), ids.first()?.clone(), ids.len()))
        });
        if let Some((source, value, count)) = changed {
            notes.push(format!(
                "{object}.{field}: the first of the {count} {source} changes"
            ));
            fields.insert(field, value);
        }
    }

    for field in tool.required() {
        let schema = tool.properties().and_then(|p| p.get(field));
        let zero = is_zero_amount(input.get(field));
        if !is_empty(input.get(field)) && !is_hollow(input.get(field), schema) && !zero {
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
                .filter(|v| !v.is_null() && !(zero && is_zero_amount(Some(v))))
                .map(|v| (r.step.clone(), v.clone()))
        });
        if let Some((source, v)) = found {
            taken(field, &source, &v, &mut notes);
            input.insert(field.to_owned(), v);
        }
    }
    // A required id still missing where every step it builds on found nothing: there is nothing
    // to act on, and a model asked for the id could only make one up (`ord_12345`).
    if empty.is_none()
        && !used.is_empty()
        && used.iter().all(|r| found_nothing(r))
        && let Some(field) = required.iter().find(|f| {
            is_empty(input.get(**f))
                && tool
                    .properties()
                    .and_then(|p| p.get(**f))
                    .and_then(id_prefix)
                    .is_some()
        })
    {
        let sources: Vec<&str> = used.iter().map(|r| r.step.as_str()).collect();
        empty = Some(format!("{} found nothing for {field}", sources.join(", ")));
    }
    // An empty text for a field the tool can do without filters by nothing: every member
    // with the tier "" is no member. Left out, the tool does what it does without it.
    let blank: Vec<String> = input
        .iter()
        .filter(|(k, v)| {
            v.as_str().is_some_and(|t| t.trim().is_empty()) && !required.contains(&k.as_str())
        })
        .map(|(k, _)| k.clone())
        .collect();
    for field in blank {
        notes.push(format!("{field}: empty, left out"));
        input.remove(&field);
    }
    Bound {
        input: Value::Object(input),
        notes,
        empty,
        undecided,
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
        let prefix = schema.and_then(id_prefix).map(|p| format!("{p}_"));
        let prefix = prefix.as_deref();
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
        ..Bound::default()
    }
}

/// A list slot whose ids sit in lists inside a list of objects (a curated page's sections, each
/// with its products): which ids go in which object is a choice, not a copy. A write whose plan
/// left those inner lists empty while a step of this turn produced that slot left the choice for
/// later, so the field is taken out and the worker fills it from the results, as for a missing
/// field. `sources` are the steps it `uses` first, then this turn's earlier steps, latest first
/// (a plan revised after an error names only the step it added).
pub fn open_inner_lists(tool: &ToolSpec, bound: &mut Bound, sources: &[&StepRecord]) {
    if !tool.is_write() {
        return;
    }
    let required = tool.required();
    for slot in &tool.consumes {
        if !slot.many
            || tool.field_for_slot(&slot.name).is_some()
            || tool.nested_field_for_slot(&slot.name).is_some()
        {
            continue;
        }
        let Some(source) = sources
            .iter()
            .find(|r| r.slots.get(&slot.name).is_some_and(|v| !v.is_empty()))
            .map(|r| r.step.clone())
        else {
            continue;
        };
        let Some(input) = bound.input.as_object_mut() else {
            return;
        };
        let open: Vec<String> = input
            .iter()
            .filter(|(_, v)| has_empty_inner_list(v))
            .map(|(k, _)| k.clone())
            .collect();
        for field in open {
            bound.notes.push(format!(
                "{field}: its {} lists were left empty; to fill from {source}'s result",
                slot.name
            ));
            input.remove(&field);
            if !required.contains(&field.as_str()) {
                bound.undecided.push(field);
            }
        }
    }
}

/// A list of objects where some object holds an empty list (`[{name, item_list_element: []}]`).
fn has_empty_inner_list(v: &Value) -> bool {
    v.as_array().is_some_and(|items| {
        items.iter().any(|i| {
            i.as_object()
                .is_some_and(|o| o.values().any(|x| x.as_array().is_some_and(Vec::is_empty)))
        })
    })
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

    fn member_order_tools() -> (ToolSpec, ToolSpec, ToolSpec) {
        let tool = |name, kind, consumes: Value, produces: Value, props: Value, required: Value| {
            serde_json::from_value::<ToolSpec>(crate::tools::fixtures::tool(
                name,
                "sales.order.x",
                kind,
                "A tool.",
                consumes,
                produces,
                props,
                required,
            ))
            .unwrap_or_else(|e| panic!("{e}"))
        };
        let members = tool(
            "search_members",
            "read",
            json!([]),
            json!([{"name": "customer_id", "many": true}]),
            json!({"query": {"type": "string", "description": "Words."}}),
            json!([]),
        );
        // A filter no slot names: the plan says where its id comes from by the step's name.
        let orders = tool(
            "search_orders",
            "read",
            json!([]),
            json!([{"name": "order_id", "many": true}]),
            json!({"customer_id": {"type": "string", "pattern": "^cus_[a-z0-9_]+$", "description": "Whose."},
                   "limit": {"type": "integer", "description": "Max."}}),
            json!([]),
        );
        let order = tool(
            "get_order",
            "read",
            json!([{"name": "order_id", "many": false}]),
            json!([]),
            json!({"order_id": {"type": "string", "pattern": "^ord_[a-z0-9_]+$", "description": "Which."}}),
            json!(["order_id"]),
        );
        (members, orders, order)
    }

    #[test]
    fn a_step_named_where_an_id_goes_gives_its_id_and_one_that_found_nothing_stops_the_chain() {
        let (members, orders, order) = member_order_tools();
        let mut found = record(
            "find_jamie",
            &members,
            json!({"items": [{"id": "cus_jamie", "name": "Jamie"}], "total": 1}),
        );
        found.input = json!({"query": "Jamie"});
        let done = BTreeMap::from([("find_jamie".to_owned(), found)]);
        let latest = step(
            "latest_order",
            "search_orders",
            json!({"customer_id": "find_jamie", "limit": 1}),
            &["find_jamie"],
        );
        let b = bind(
            &latest,
            &orders,
            &done,
            &["find_jamie".into()],
            &BTreeMap::new(),
        );
        assert_eq!(b.input["customer_id"], "cus_jamie", "{:?}", b.notes);
        assert!(b.empty.is_none());

        // Nobody found: the orders are not read unfiltered, and the order after them is not
        // left for a model to make up.
        let none = record("find_jamie", &members, json!({"items": [], "total": 0}));
        let done = BTreeMap::from([("find_jamie".to_owned(), none)]);
        let b = bind(
            &latest,
            &orders,
            &done,
            &["find_jamie".into()],
            &BTreeMap::new(),
        );
        assert!(b.input.get("customer_id").is_none(), "{}", b.input);
        assert!(
            b.empty.as_deref().is_some_and(|e| e.contains("find_jamie")),
            "{:?}",
            b.empty
        );

        let no_orders = record("latest_order", &orders, json!({"items": [], "total": 0}));
        let done = BTreeMap::from([("latest_order".to_owned(), no_orders)]);
        for written in ["latest_order", "order_id"] {
            let detail = step(
                "detail",
                "get_order",
                json!({"order_id": written}),
                &["latest_order"],
            );
            let b = bind(
                &detail,
                &order,
                &done,
                &["latest_order".into()],
                &BTreeMap::new(),
            );
            assert!(b.input.get("order_id").is_none(), "{written}: {}", b.input);
            assert!(b.empty.is_some(), "{written}: {:?}", b.notes);
        }
    }

    #[test]
    fn a_search_whose_words_are_one_products_name_gives_that_product() {
        let m = manifest();
        let search_tool = get(&m, "search_products");
        let mut search = record(
            "s2",
            search_tool,
            json!({"items": [{"id": "prd_windbreaker", "name": "Packable windbreaker"},
                             {"id": "prd_rain_shell", "name": "Packable rain shell"},
                             {"id": "prd_windbreaker_kids", "name": "Packable windbreaker for kids"}], "total": 3}),
        );
        search.input = json!({"query": "packable  Windbreaker"});
        let done = BTreeMap::from([("s2".to_owned(), search)]);
        let cart: ToolSpec = serde_json::from_value(crate::tools::fixtures::tool(
            "add_to_cart", "sales.cart.add", "write", "Add to the cart.",
            json!([{"name": "product_id", "many": false}]), json!([]),
            json!({"product_id": {"type": "string", "pattern": "^prd_[a-z0-9_]+$", "description": "Which."}}),
            json!([]),
        ))
        .unwrap_or_else(|e| panic!("{e}"));
        let b = bind(
            &step(
                "s3",
                "add_to_cart",
                json!({"product_id": "prd_..."}),
                &["s2"],
            ),
            &cart,
            &done,
            &["s2".into()],
            &BTreeMap::new(),
        );
        assert_eq!(b.input["product_id"], "prd_windbreaker", "{:?}", b.notes);
        assert!(b.undecided.is_empty());
    }

    #[test]
    fn a_record_that_lists_its_own_gives_their_ids() {
        let slot: SlotSpec = serde_json::from_value(json!({"name": "address_id", "many": true}))
            .unwrap_or_else(|e| panic!("{e}"));
        let profile =
            json!({"id": "cus_jamie", "addresses": [{"id": "adr_home"}, {"id": "adr_work"}]});
        assert_eq!(
            values_in(&profile, &slot, false),
            vec![json!("adr_home"), json!("adr_work")]
        );
        assert!(values_in(&json!({"id": "cus_jamie"}), &slot, false).is_empty());
    }

    #[test]
    fn a_record_read_by_its_id_hands_that_id_on() {
        let (_, orders, order) = member_order_tools();
        let read = record(
            "o1",
            &order,
            json!({"id": "ord_01m6gca7b028fw0nq7gdfg2kxq", "order_status": "paid"}),
        );
        assert_eq!(
            read.slots.get("order_id"),
            Some(&vec![json!("ord_01m6gca7b028fw0nq7gdfg2kxq")])
        );
        // An `id` of another kind is not the one it was read by; a list is not one record.
        assert!(
            record("o2", &order, json!({"id": "cus_jamie"}))
                .slots
                .is_empty()
        );
        assert!(
            !record("o3", &orders, json!({"items": [], "total": 0}))
                .slots
                .contains_key("order_id")
        );
        let done = BTreeMap::from([("o1".to_owned(), read)]);
        let b = bind(
            &step(
                "again",
                "get_order",
                json!({"order_id": "ord_???"}),
                &["o1"],
            ),
            &order,
            &done,
            &["o1".to_owned()],
            &BTreeMap::new(),
        );
        assert_eq!(
            b.input["order_id"], "ord_01m6gca7b028fw0nq7gdfg2kxq",
            "{:?}",
            b.notes
        );
    }

    #[test]
    fn opaque_ids_nothing_showed_are_taken_out_and_word_ids_are_kept() {
        let (_, orders, order) = member_order_tools();
        let seen = r#"{"items":[{"id":"ord_01m6gca7b028fw0nq7gdfg2kxq"}]}"#;
        let mut made_up = json!({"order_id": "ord_12345"});
        let out = unseen_ids(&mut made_up, &order.input_schema, seen, "");
        assert_eq!(out, vec![("order_id".to_owned(), "ord_12345".to_owned())]);
        assert_eq!(made_up, json!({}));

        let mut real = json!({"order_id": "ord_01m6gca7b028fw0nq7gdfg2kxq"});
        assert!(unseen_ids(&mut real, &order.input_schema, seen, "").is_empty());
        // An id made of words may come from the words of the request.
        let mut words = json!({"customer_id": "cus_jamie", "limit": 1});
        assert!(unseen_ids(&mut words, &orders.input_schema, seen, "").is_empty());
        assert_eq!(words["customer_id"], "cus_jamie");
    }

    #[test]
    fn sections_left_empty_next_to_a_search_are_left_to_the_worker() {
        let m = manifest();
        let search = record(
            "s1",
            get(&m, "search_products"),
            json!({"items": [{"id": "prd_wool_overcoat"}, {"id": "prd_chore_coat"}], "total": 2}),
        );
        let page: ToolSpec = serde_json::from_value(crate::tools::fixtures::tool(
            "create_curation",
            "merchandising.collection_page.create",
            "write",
            "Make a curated page.",
            json!([{"name": "product_id", "many": true}]),
            json!([{"name": "collection_page_id", "many": false}]),
            json!({"name": {"type": "string", "description": "Title."},
                   "has_part": {"type": "array", "description": "Sections.", "items": {
                       "type": "object", "additionalProperties": false,
                       "properties": {"name": {"type": "string", "description": "Heading."},
                                      "item_list_element": {"type": "array", "description": "Products.",
                                          "items": {"type": "string", "pattern": "^prd_[a-z0-9_]+$"}}}}}}),
            json!(["name", "has_part"]),
        ))
        .unwrap_or_else(|e| panic!("{e}"));
        let args = |coats: Value| {
            json!({"name": "Winter layers",
                   "has_part": [{"name": "Coats", "item_list_element": coats}]})
        };
        let open = |input: Value, sources: &[&StepRecord]| {
            let mut b = Bound {
                input,
                ..Bound::default()
            };
            open_inner_lists(&page, &mut b, sources);
            b
        };
        let b = open(args(json!([])), &[&search]);
        assert!(b.input.get("has_part").is_none(), "{}", b.input);
        assert!(b.notes[0].starts_with("has_part:"), "{:?}", b.notes);
        // Filled by the plan, or with nothing this turn to fill it from: kept as written.
        let kept = args(json!(["prd_chore_coat"]));
        assert_eq!(open(kept.clone(), &[&search]).input, kept);
        assert_eq!(open(args(json!([])), &[]).input, args(json!([])));
    }

    #[test]
    fn an_empty_text_for_a_field_the_tool_can_do_without_is_left_out() {
        let m = manifest();
        let b = bind(
            &step(
                "s1",
                "search_products",
                json!({"query": "", "category": " ", "limit": 20}),
                &[],
            ),
            get(&m, "search_products"),
            &BTreeMap::new(),
            &[],
            &BTreeMap::new(),
        );
        assert_eq!(b.input, json!({"limit": 20}));
        assert!(
            b.notes.iter().any(|n| n == "query: empty, left out"),
            "{:?}",
            b.notes
        );
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
        // An order number where the field takes it is a value; without the handle it is not.
        let widened = json!({"type": "string", "pattern": "^(?:ord_[a-z0-9_]+|#?[0-9]+)$",
                             "x-handle": {"field": "order_number", "shape": "number"}});
        assert!(!is_stand_in(
            &json!("27184"),
            "order_id",
            "order_id",
            Some(&widened)
        ));
        assert!(is_stand_in(
            &json!("order_id"),
            "order_id",
            "order_id",
            Some(&widened)
        ));
        assert!(is_stand_in(
            &json!("ord_"),
            "order_id",
            "order_id",
            Some(&widened)
        ));
        let plain = json!({"type": "string", "pattern": "^ord_[a-z0-9_]+$"});
        assert!(is_stand_in(
            &json!("27184"),
            "order_id",
            "order_id",
            Some(&plain)
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
        // Written as a shape without its amount: the cart's total, the same as left out.
        let b = bind(
            &step(
                "s2",
                "prepare_order",
                json!({"delivery_mode": "saved", "client_total": {"value": null, "currency": "USD"}}),
                &["s1"],
            ),
            get(&m, "prepare_order"),
            &done,
            &["s1".into()],
            &BTreeMap::new(),
        );
        assert_eq!(
            b.input["client_total"]["value"],
            json!(71.20),
            "{:?}",
            b.notes
        );
        // Written as zero: the cart read the total, so the cart's total.
        let b = bind(
            &step(
                "s2",
                "prepare_order",
                json!({"delivery_mode": "saved", "client_total": {"value": 0, "currency": "USD"}}),
                &["s1"],
            ),
            get(&m, "prepare_order"),
            &done,
            &["s1".into()],
            &BTreeMap::new(),
        );
        assert_eq!(
            b.input["client_total"]["value"],
            json!(71.20),
            "{:?}",
            b.notes
        );
        // A zero nothing read stays: the server's check says what the total is.
        let b = bind(
            &step(
                "s2",
                "prepare_order",
                json!({"delivery_mode": "saved", "client_total": {"value": 0, "currency": "USD"}}),
                &[],
            ),
            get(&m, "prepare_order"),
            &done,
            &["s1".into()],
            &BTreeMap::new(),
        );
        assert_eq!(b.input["client_total"]["value"], json!(0));
    }

    fn with_nested_slot() -> ToolSpec {
        let m = manifest();
        let mut banner = get(&m, "schedule_banner").clone();
        banner.input_schema["properties"]["content"] = json!({"type": "object", "properties": {
            "headline": {"type": "string"},
            "image_product_id": {"type": "string", "pattern": "^prd_[a-z0-9_]+$", "x-slot": "product_id"}}});
        banner.consumes =
            vec![serde_json::from_value(json!({"name": "product_id", "many": false})).unwrap()];
        banner
    }

    #[test]
    fn a_slot_inside_an_object_is_filled_from_the_steps_results() {
        let m = manifest();
        let banner = with_nested_slot();
        let search = record(
            "s1",
            get(&m, "search_products"),
            json!({"items": [{"product_id": "prd_a"}, {"product_id": "prd_b"}]}),
        );
        let done = BTreeMap::from([("s1".to_owned(), search)]);
        for written in [
            json!({"headline": "Sale"}),
            json!({"headline": "Sale", "image_product_id": "$item[1].product_id"}),
        ] {
            let b = bind(
                &step(
                    "s2",
                    "schedule_banner",
                    json!({"content": written}),
                    &["s1"],
                ),
                &banner,
                &done,
                &["s1".into()],
                &BTreeMap::new(),
            );
            assert_eq!(
                b.input["content"]["image_product_id"], "prd_a",
                "{:?}",
                b.notes
            );
            assert_eq!(b.input["content"]["headline"], "Sale");
        }
        // What the model chose stays.
        let b = bind(
            &step(
                "s2",
                "schedule_banner",
                json!({"content": {"image_product_id": "prd_b"}}),
                &["s1"],
            ),
            &banner,
            &done,
            &["s1".into()],
            &BTreeMap::new(),
        );
        assert_eq!(b.input["content"]["image_product_id"], "prd_b");
    }

    #[test]
    fn a_slot_inside_an_object_takes_a_record_a_used_write_changes() {
        let m = manifest();
        let banner = with_nested_slot();
        let sale = get(&m, "set_sale_prices");
        let price = |p: &str| {
            json!({"op": "update", "entity": "unit_price_specification", "id": null,
                   "from": {"product_id": p, "price": 88.0}, "to": {"product_id": p, "price": 70.4}})
        };
        let preview: Preview = serde_json::from_value(json!({
            "preview_id": "pv_1", "expires_at": null, "needs_approval": true, "checks": [],
            "changes": [{"op": "create", "entity": "promotion", "id": null, "to": {"products": 2}},
                        price("prd_overshirt"), price("prd_field_jacket"), price("prd_overshirt")]}))
        .unwrap();
        let written = StepRecord {
            step: "s1".into(),
            tool: sale.tool.clone(),
            kind: sale.kind,
            input: json!({}),
            output: None,
            preview: Some(preview),
            item: Some(1),
            slots: slots_from_item(sale, 1),
        };
        let done = BTreeMap::from([("s1".to_owned(), written)]);
        let b = bind(
            &step(
                "s2",
                "schedule_banner",
                json!({"content": {"headline": "Sale", "image_product_id": "prd_..."}}),
                &["s1"],
            ),
            &banner,
            &done,
            &["s1".into()],
            &BTreeMap::new(),
        );
        assert_eq!(
            b.input["content"]["image_product_id"], "prd_overshirt",
            "{:?}",
            b.notes
        );
        assert!(
            b.notes
                .iter()
                .any(|n| n.contains("the first of the 2 s1 changes")),
            "{:?}",
            b.notes
        );
        // A step that does not use the write gets nothing from it.
        let b = bind(
            &step(
                "s2",
                "schedule_banner",
                json!({"content": {"headline": "Sale"}}),
                &[],
            ),
            &banner,
            &done,
            &["s1".into()],
            &BTreeMap::new(),
        );
        assert!(
            b.input["content"].get("image_product_id").is_none(),
            "{:?}",
            b.notes
        );
    }

    #[test]
    fn one_of_several_is_only_taken_from_a_search_by_words() {
        let m = manifest();
        let rows = json!({"items": [{"product_id": "prd_a"}, {"product_id": "prd_b"}]});
        let mut listed = record("s1", get(&m, "search_products"), rows);
        let bound = |r: &StepRecord| {
            bind(
                &step("s2", "add_to_cart", json!({}), &["s1"]),
                get(&m, "add_to_cart"),
                &BTreeMap::from([("s1".to_owned(), r.clone())]),
                &["s1".into()],
                &BTreeMap::new(),
            )
        };
        listed.input = json!({"category_id": "cat_outerwear"});
        let b = bound(&listed);
        assert!(b.input.get("product_id").is_none(), "{:?}", b.notes);
        assert_eq!(b.undecided, ["product_id"]);
        listed.input = json!({"query": "windbreaker"});
        let b = bound(&listed);
        assert_eq!(b.input["product_id"], "prd_a");
        assert!(b.undecided.is_empty());
    }

    #[test]
    fn a_step_on_a_write_of_this_plan_takes_what_it_produces() {
        let m = manifest();
        let sale = StepRecord {
            step: "s1".into(),
            tool: "set_sale_prices".into(),
            kind: ToolKind::WriteApproval,
            input: json!({}),
            slots: slots_from_item(get(&m, "set_sale_prices"), 1),
            output: None,
            preview: None,
            item: Some(1),
        };
        let b = bind(
            &step(
                "s2",
                "create_coupon",
                json!({"promotion_id": "promo_autumn_knit"}),
                &["s1"],
            ),
            get(&m, "create_coupon"),
            &BTreeMap::from([("s1".to_owned(), sale)]),
            &["s1".into()],
            &BTreeMap::new(),
        );
        assert_eq!(
            b.input["promotion_id"], "$item[1].promotion_id",
            "{:?}",
            b.notes
        );
    }

    #[test]
    fn a_placeholder_needs_an_item_number() {
        let schema = json!({"type": "string", "pattern": "^promo_[a-z0-9_]+$"});
        let stand_in =
            |v: &str| is_stand_in(&json!(v), "promotion_id", "promotion_id", Some(&schema));
        assert!(!stand_in("$item[1].promotion_id"));
        assert!(stand_in("$item[set_sale_prices_1].promotion_id"));
        assert!(stand_in("promo_"));
        assert!(!stand_in("promo_autumn_knit"));
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

    fn reorder_tools() -> Vec<ToolSpec> {
        let spec = |tool: &str, kind: &str, properties: Value, required: Value| {
            json!({"tool": tool, "address": format!("catalog.reorder.{tool}"), "kind": kind,
                "summary": "", "consumes": [{"name": "product_id", "many": false}], "produces": [],
                "checks": [], "group": "offer", "method": "GET", "path": "/v1/x", "errors": [],
                "audience": "operator", "action_type": "SearchAction", "person_scope": "",
                "input_schema": {"type": "object", "additionalProperties": false,
                                 "properties": properties, "required": required}})
        };
        let id = json!({"type": "string", "pattern": "^prd_[a-z0-9_]+$"});
        let lines = json!({"type": "array", "minItems": 1, "items": {"type": "object"}});
        serde_json::from_value::<crate::tools::Manifest>(json!({"items": [
            spec("suggest_reorder", "read", json!({"product_id": id}), json!(["product_id"])),
            spec("create_reorder", "write",
                 json!({"product_id": id, "lines": lines, "description": {"type": "string"}}),
                 json!(["product_id", "lines"])),
        ]}))
        .unwrap()
        .items
    }

    fn suggestion(lines: Value) -> StepRecord {
        let m = reorder_tools();
        record(
            "suggest",
            get(&m, "suggest_reorder"),
            json!({"product_id": "prd_rash_guard", "lines": lines, "total_units": 0}),
        )
    }

    #[test]
    fn a_reference_to_a_used_steps_result_takes_its_value() {
        let m = reorder_tools();
        let lines = json!([{"variant_id": "var_rash_guard_s_black", "order_quantity": 4}]);
        let done = BTreeMap::from([("suggest".to_owned(), suggestion(lines.clone()))]);
        let order = vec!["suggest".to_owned()];
        for written in ["uses.suggest.lines", "suggest.lines", "uses:suggest"] {
            let b = bind(
                &step(
                    "draft",
                    "create_reorder",
                    json!({"product_id": "uses.suggest.product_id", "lines": written}),
                    &["suggest"],
                ),
                get(&m, "create_reorder"),
                &done,
                &order,
                &BTreeMap::new(),
            );
            assert_eq!(b.input["lines"], lines, "{written}");
            assert_eq!(b.input["product_id"], "prd_rash_guard");
            assert_eq!(b.empty, None);
        }
        // A step it does not use is not a source.
        let b = bind(
            &step(
                "draft",
                "create_reorder",
                json!({"lines": "suggest.lines"}),
                &[],
            ),
            get(&m, "create_reorder"),
            &done,
            &order,
            &BTreeMap::new(),
        );
        assert_eq!(b.input["lines"], "suggest.lines");
    }

    #[test]
    fn a_required_list_that_came_back_empty_leaves_nothing_to_do() {
        let m = reorder_tools();
        let done = BTreeMap::from([("suggest".to_owned(), suggestion(json!([])))]);
        let order = vec!["suggest".to_owned()];
        // Written as a reference, or left out for the runtime to fill: either way the
        // suggestion's empty `lines` is not handed to a model to make up.
        for args in [
            json!({"lines": "uses.suggest.lines"}),
            json!({"description": "Selling fast"}),
        ] {
            let b = bind(
                &step("draft", "create_reorder", args.clone(), &["suggest"]),
                get(&m, "create_reorder"),
                &done,
                &order,
                &BTreeMap::new(),
            );
            assert_eq!(
                b.empty.as_deref(),
                Some("lines from suggest is empty"),
                "{args}"
            );
        }
    }
}
