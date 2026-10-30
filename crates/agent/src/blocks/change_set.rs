//! The `change_set` block: the result of every operator write request (docs/api/chat.md §3.3).
//!
//! Built from `get_change_set` when the caller can read it (the authoritative view, with the
//! number, the undo deadline, and what was left out), or else from the previews this turn made.

use std::collections::BTreeMap;

use acom_contract::{Change, ChangeOp, Check};
use serde_json::{Map, Value, json};

use super::{Action, Block, SuggestedIntent, words};
use crate::planner::Understood;

/// How many change rows a group shows before `more` counts the rest.
const ROWS_SHOWN: usize = 20;

/// One item of the set.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemView {
    pub position: usize,
    pub tool: String,
    /// The tool's one-line summary, the group's label.
    pub label: String,
    pub preview_id: String,
    pub changes: Vec<Change>,
    /// All changes, when the view holds only the first ones.
    pub changes_total: usize,
    pub checks: Vec<Check>,
}

/// The set as the block shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChangeSetView {
    pub id: String,
    pub number: Option<i64>,
    pub version: i32,
    pub state: String,
    pub request: Vec<String>,
    pub interpretation: Vec<Understood>,
    pub items: Vec<ItemView>,
    /// `{entity, id, reason}` for what the previews left out.
    pub left_out: Vec<Value>,
    pub undo_until: Option<String>,
}

impl ChangeSetView {
    /// From `get_change_set`'s output. `label` gives a tool's summary.
    pub fn from_read(v: &Value, label: &dyn Fn(&str) -> String) -> Option<Self> {
        let items = v
            .get("items")?
            .as_array()?
            .iter()
            .map(|i| {
                let tool = i.get("tool").and_then(Value::as_str).unwrap_or_default();
                let changes: Vec<Change> = i
                    .get("changes")
                    .cloned()
                    .and_then(|c| serde_json::from_value(c).ok())
                    .unwrap_or_default();
                ItemView {
                    position: i.get("position").and_then(Value::as_u64).unwrap_or(0) as usize,
                    tool: tool.to_owned(),
                    label: label(tool),
                    preview_id: i
                        .get("preview_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    changes_total: i
                        .get("changes_total")
                        .and_then(Value::as_u64)
                        .map_or(changes.len(), |n| n as usize),
                    changes,
                    checks: i
                        .get("checks")
                        .cloned()
                        .and_then(|c| serde_json::from_value(c).ok())
                        .unwrap_or_default(),
                }
            })
            .collect();
        let interpretation = v
            .get("interpretation")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(understood).collect())
            .unwrap_or_default();
        Some(Self {
            id: v.get("id")?.as_str()?.to_owned(),
            number: v.get("number").and_then(Value::as_i64),
            version: v
                .get("version")
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
                .unwrap_or(1),
            state: v
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("drafting")
                .to_owned(),
            request: v
                .pointer("/request/text")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            interpretation,
            items,
            left_out: v
                .get("left_out")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            undo_until: v
                .get("undo_until")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }

    /// The `exclude` changes of the items, as `left_out` rows (when building from previews).
    pub fn left_out_from_items(&mut self) {
        for i in &self.items {
            for c in i.changes.iter().filter(|c| c.op == ChangeOp::Exclude) {
                self.left_out.push(json!({"entity": c.entity, "id": c.id,
                    "reason": c.note.clone().map_or_else(|| c.to.get("why").cloned().unwrap_or(Value::Null), Value::String),
                    "preview_id": i.preview_id}));
            }
        }
    }

    pub fn failed_checks(&self) -> Vec<&Check> {
        self.items
            .iter()
            .flat_map(|i| &i.checks)
            .filter(|c| !c.passed)
            .collect()
    }

    pub fn check_count(&self) -> usize {
        self.items.iter().map(|i| i.checks.len()).sum()
    }
}

/// An interpretation line stored as `{aspect, text}`, or a plain string.
fn understood(v: &Value) -> Option<Understood> {
    match v {
        Value::String(s) => Some(Understood {
            aspect: "other".into(),
            text: s.clone(),
            source: None,
        }),
        Value::Object(o) => Some(Understood {
            aspect: o
                .get("aspect")
                .and_then(Value::as_str)
                .unwrap_or("other")
                .to_owned(),
            text: o.get("text")?.as_str()?.to_owned(),
            source: None,
        }),
        _ => None,
    }
}

/// The element id of a change row: the entity id, else the product it prices.
fn row_id(c: &Change, n: usize) -> String {
    if !c.id.is_empty() {
        return c.id.clone();
    }
    c.to.get("product_id")
        .or_else(|| c.from.get("product_id"))
        .and_then(Value::as_str)
        .map_or_else(|| format!("{}_{}", c.entity, n + 1), str::to_owned)
}

/// The block. `check_intents` (from the agent definition) gives what a person can say to fix a
/// failed check; `title` is the plan's short title when there is one.
pub fn block(
    view: &ChangeSetView,
    check_intents: &BTreeMap<String, String>,
    title: Option<&str>,
) -> Block {
    let mut understood = Map::new();
    for u in &view.interpretation {
        let entry = understood
            .entry(u.aspect.clone())
            .or_insert_with(|| Value::String(String::new()));
        if let Value::String(s) = entry {
            if !s.is_empty() {
                s.push_str("; ");
            }
            s.push_str(&u.text);
        }
    }

    let mut checks = Vec::new();
    let mut intents = Vec::new();
    let mut seen_rules: BTreeMap<String, usize> = BTreeMap::new();
    for i in &view.items {
        for c in &i.checks {
            let n = seen_rules.entry(c.rule.clone()).or_default();
            *n += 1;
            let element = if *n == 1 {
                format!("check_row:{}", c.rule)
            } else {
                format!("check_row:{}@{}", c.rule, i.position)
            };
            if !c.passed {
                let say = check_intents
                    .get(&c.rule)
                    .cloned()
                    .unwrap_or_else(|| format!("Fix the {} check", words(&c.rule)));
                intents.push(SuggestedIntent::new(element.clone(), say));
            }
            checks.push(
                json!({"rule": c.rule, "passed": c.passed, "detail": c.detail,
                               "fix_tool": c.fix_tool, "tool": i.tool, "element": element}),
            );
        }
    }

    let mut groups = Vec::new();
    for i in &view.items {
        let shown: Vec<&Change> = i
            .changes
            .iter()
            .filter(|c| c.op != ChangeOp::Exclude)
            .collect();
        let rows: Vec<Value> = shown
            .iter()
            .take(ROWS_SHOWN)
            .enumerate()
            .map(|(n, c)| {
                let element = format!("change_row:{}", row_id(c, n));
                if intents.len() < 40 {
                    intents.push(SuggestedIntent::new(element.clone(), "Leave this one out"));
                }
                json!({"element": element, "op": c.op, "entity": c.entity, "id": c.id,
                       "from": c.from, "to": c.to, "note": c.note})
            })
            .collect();
        let excluded = i
            .changes
            .iter()
            .filter(|c| c.op == ChangeOp::Exclude)
            .count();
        let total = i
            .changes_total
            .max(i.changes.len())
            .saturating_sub(excluded);
        groups.push(json!({
            "position": i.position, "op": i.tool, "label": i.label, "preview_id": i.preview_id,
            "summary": format!("{total} change{}", if total == 1 { "" } else { "s" }),
            "rows": rows, "more": total.saturating_sub(rows.len()),
        }));
    }

    let after_approval: Vec<Value> = view
        .items
        .iter()
        .map(|i| json!({"position": i.position, "tool": i.tool, "label": i.label}))
        .collect();

    let number = view
        .number
        .map_or_else(|| view.id.clone(), |n| format!("#{n}"));
    let failed = view.failed_checks().len();
    let data = json!({
        "change_set_id": view.id, "number": view.number, "version": view.version,
        "state": view.state, "request": view.request, "understood": understood,
        "checks": checks, "groups": groups, "left_out": view.left_out,
        "after_approval": after_approval, "undo": {"until": view.undo_until},
    });
    let heading = title.map_or_else(
        || format!("Change set {number}"),
        |t| format!("{t} · change set {number}"),
    );
    let mut b = Block::new("change_set", heading, data);
    b.why = (failed > 0).then(|| {
        format!(
            "{failed} of {} checks need your decision before this can go for approval.",
            view.check_count()
        )
    });
    if view.state == "waiting_for_approval" {
        b.actions.push(Action::person(
            "approve",
            format!("Approve version {}", view.version),
            format!("/v1/platform/change-sets/{}/approve", view.id),
            json!({"version": view.version}),
            format!(
                "Approve change set {number}, version {}? Its changes run right away.",
                view.version
            ),
        ));
        b.actions.push(Action::person(
            "reject",
            "Reject",
            format!("/v1/platform/change-sets/{}/reject", view.id),
            json!({"reason": ""}),
            "Reject this change set? Say why so the agent can try again.",
        ));
    }
    b.actions.push(Action::say(
        "edit",
        "Change something",
        json!({"change_set_id": view.id}),
    ));
    if view.version > 1 {
        b.actions.push(Action::open(
            "compare",
            format!("Compare with version {}", view.version - 1),
            format!("/bo/change-sets/{}?compare={}", view.id, view.version - 1),
        ));
    }
    b.suggested_intents = intents;
    b
}

#[cfg(test)]
pub(crate) mod fixtures {
    use serde_json::{Value, json};

    /// `get_change_set` for #1042 v1 (scene B1): two failed checks, two products left out.
    pub fn b1_v1() -> Value {
        json!({
            "id": "cs_1042", "number": 1042, "version": 1, "state": "drafting",
            "title": "VIP weekend",
            "request": {"text": ["Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."]},
            "interpretation": [{"aspect": "who", "text": "VIP members"}, {"aspect": "when", "text": "Sat Nov 7 – Sun Nov 8"}],
            "items": [
                {"position": 1, "preview_id": "pv_1", "tool": "set_sale_prices", "changes_total": 43,
                 "changes": [
                    {"op": "create", "entity": "promotion", "id": "", "to": {"name": "VIP weekend", "products": 42}},
                    {"op": "update", "entity": "unit_price_specification", "id": "",
                     "from": {"product_id": "prd_seersucker_overshirt", "price": 79.00, "price_currency": "USD"},
                     "to": {"product_id": "prd_seersucker_overshirt", "price": 63.20, "price_currency": "USD", "percent_off": 20}}],
                 "checks": [
                    {"rule": "max_discount", "passed": true, "detail": "Highest is 20%; policy allows 30% for vip", "fix_tool": null},
                    {"rule": "price_floor", "passed": false, "detail": "Seersucker overshirt at 20% would be $63.20, under the floor $66.70", "fix_tool": "set_sale_prices"},
                    {"rule": "promo_min_stock", "passed": false, "detail": "Cotton chore coat (7) and Light rain parka (4) are under 10 units", "fix_tool": "set_sale_prices"}]},
                {"position": 2, "preview_id": "pv_2", "tool": "create_coupon", "changes_total": 1,
                 "changes": [{"op": "create", "entity": "coupon", "id": "", "to": {"code": "VIPWEEKEND20"}}],
                 "checks": [{"rule": "member_benefits_per_order", "passed": true, "detail": "One member benefit per order", "fix_tool": null}]}
            ],
            "left_out": [],
            "undo_until": "2026-11-09T23:59:00-05:00"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intents() -> BTreeMap<String, String> {
        BTreeMap::from([(
            "promo_min_stock".to_owned(),
            "Drop the low-stock ones".to_owned(),
        )])
    }

    #[test]
    fn a_draft_with_failed_checks_has_no_approve_button_and_says_why() {
        let view =
            ChangeSetView::from_read(&fixtures::b1_v1(), &|t| format!("{t} summary")).unwrap();
        assert_eq!(view.failed_checks().len(), 2);
        let b = block(&view, &intents(), Some("VIP weekend"));
        assert_eq!(b.ui, "change_set");
        assert_eq!(b.title, "VIP weekend · change set #1042");
        assert_eq!(
            b.why.as_deref(),
            Some("2 of 4 checks need your decision before this can go for approval.")
        );
        assert_eq!(b.person_actions().count(), 0);
        assert_eq!(b.data["understood"]["who"], "VIP members");
        assert_eq!(b.data["groups"][0]["summary"], "43 changes");
        assert_eq!(b.data["groups"][0]["more"], 41);
        assert_eq!(
            b.data["groups"][0]["rows"][1]["element"],
            "change_row:prd_seersucker_overshirt"
        );
        assert!(b.suggested_intents.contains(&SuggestedIntent::new(
            "check_row:promo_min_stock",
            "Drop the low-stock ones"
        )));
        assert!(b.suggested_intents.contains(&SuggestedIntent::new(
            "check_row:price_floor",
            "Fix the price floor check"
        )));
        assert_eq!(b.data["undo"]["until"], "2026-11-09T23:59:00-05:00");
    }

    #[test]
    fn a_submitted_set_offers_approval_of_its_exact_version() {
        let mut v = fixtures::b1_v1();
        v["state"] = json!("waiting_for_approval");
        v["version"] = json!(2);
        let view = ChangeSetView::from_read(&v, &|t| t.to_owned()).unwrap();
        let b = block(&view, &BTreeMap::new(), None);
        let approve = b
            .actions
            .iter()
            .find_map(|a| match a {
                Action::Person {
                    action_id,
                    path,
                    body,
                    ..
                } if action_id == "approve" => Some((path.clone(), body.clone())),
                _ => None,
            })
            .unwrap();
        assert_eq!(approve.0, "/v1/platform/change-sets/cs_1042/approve");
        assert_eq!(approve.1, json!({"version": 2}));
        assert!(
            b.actions
                .iter()
                .any(|a| matches!(a, Action::Open { target, .. } if target.ends_with("compare=1")))
        );
    }

    #[test]
    fn previews_left_out_become_left_out_rows() {
        let mut view = ChangeSetView {
            id: "cs_1".into(),
            items: vec![ItemView {
                position: 1,
                tool: "set_sale_prices".into(),
                label: "x".into(),
                preview_id: "pv_1".into(),
                changes: vec![Change {
                    op: ChangeOp::Exclude,
                    entity: "product".into(),
                    id: "prd_chore_coat".into(),
                    from: Value::Null,
                    to: json!({"why": "promo_min_stock", "inventory_level": 7}),
                    note: None,
                }],
                changes_total: 1,
                checks: vec![],
            }],
            ..Default::default()
        };
        view.left_out_from_items();
        assert_eq!(view.left_out[0]["id"], "prd_chore_coat");
        assert_eq!(view.left_out[0]["reason"], "promo_min_stock");
        let b = block(&view, &BTreeMap::new(), None);
        assert_eq!(b.data["groups"][0]["summary"], "0 changes");
    }
}
