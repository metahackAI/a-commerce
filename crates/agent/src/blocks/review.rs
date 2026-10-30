//! The `refund_review` block (docs/api/chat.md §3.3, scene B2): a batch of return requests
//! judged against the store's policy, and the one change set that carries the refunds and the
//! messages. Rows come from `search_return_requests` and `evaluate_return_request` (or from the
//! review shown earlier in the conversation); refund amounts, checks and messages from the
//! previews in the change set. The approve button is the operator's.

use std::collections::BTreeMap;
use std::str::FromStr;

use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::change_set::ChangeSetView;
use super::{Action, Block, SuggestedIntent, words};
use crate::executor::rows;

const REFUND_TOOL: &str = "refund_return";
const MESSAGE_TOOL: &str = "send_customer_message";

fn photos(n: Option<u64>) -> Value {
    match n {
        None => Value::Null,
        Some(0) => json!("no photos"),
        Some(1) => json!("1 photo"),
        Some(n) => json!(format!("{n} photos")),
    }
}

/// One row per request: the queue row joined with its evaluation by `claim_id`. Rows already
/// joined (from a review shown earlier) pass through.
pub fn requests(queue: &[Value], evaluations: &[Value]) -> Vec<Value> {
    let by_claim = |id: &Value| {
        evaluations
            .iter()
            .find(|e| e.get("claim_id") == Some(id) && id.is_string())
    };
    let mut out: Vec<Value> = queue
        .iter()
        .map(|q| {
            if q.get("suggested").is_some() {
                return q.clone();
            }
            let e = q.get("claim_id").and_then(by_claim);
            let pick = |k: &str| e.and_then(|e| e.get(k)).cloned().unwrap_or(Value::Null);
            json!({
                "claim_id": q.get("claim_id"), "order_id": q.get("order_id"),
                "customer_id": q.get("customer_id"), "order_number": q.get("order_number"),
                "item": q.get("item"), "amount": q.get("amount"), "reason": q.get("reason"),
                "evidence": photos(q.get("photos").and_then(Value::as_u64)),
                "day": q.get("days_since_delivery"),
                "suggested": pick("suggested"), "because": pick("rule_text"), "rule": pick("rule"),
                "refund_amount": pick("refund_amount"), "needs_person": pick("needs_person"),
                "flags": e.and_then(|e| e.get("flags")).cloned().unwrap_or(json!([])),
            })
        })
        .collect();
    // Evaluations without a queue row (the operator asked about one request).
    for e in evaluations {
        let id = e.get("claim_id");
        if !out.iter().any(|r| r.get("claim_id") == id) {
            out.push(json!({
                "claim_id": id, "order_number": e.get("order_number"),
                "suggested": e.get("suggested"), "because": e.get("rule_text"),
                "rule": e.get("rule"), "refund_amount": e.get("refund_amount"),
                "needs_person": e.get("needs_person"),
                "flags": e.get("flags").cloned().unwrap_or(json!([])),
            }));
        }
    }
    out
}

fn decimal(v: &Value) -> Option<Decimal> {
    let n = v.get("value")?;
    Decimal::from_str(n.to_string().trim_matches('"')).ok()
}

/// The sum of amounts the tools returned, in their currency (no currency → none). Adding the
/// refunds the previews will pay is the only arithmetic here; every amount comes from a tool.
fn total(amounts: &[&Value]) -> Value {
    let mut sum = Decimal::ZERO;
    let mut currency: Option<&Value> = None;
    for a in amounts {
        let (Some(d), c) = (decimal(a), a.get("currency")) else {
            continue;
        };
        if currency.is_some_and(|x| Some(x) != c) {
            return Value::Null;
        }
        currency = c;
        sum += d;
    }
    let Some(currency) = currency else {
        return Value::Null;
    };
    json!({"value": Value::from_str(&format!("{:.2}", sum.round_dp(2))).unwrap_or(Value::Null),
           "currency": currency})
}

fn count(rows: &[Value], suggested: &str) -> usize {
    rows.iter().filter(|r| r["suggested"] == suggested).count()
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// "Approve 9 refunds and send 3 messages": what the operator's button does, by item kind.
pub fn approve_label(view: &ChangeSetView) -> String {
    let refunds = view.items.iter().filter(|i| i.tool == REFUND_TOOL).count();
    let messages = view.items.iter().filter(|i| i.tool == MESSAGE_TOOL).count();
    let other = view.items.len() - refunds - messages;
    let mut parts = Vec::new();
    if refunds > 0 {
        parts.push(plural(refunds, "refund"));
    }
    if other > 0 {
        parts.push(plural(other, "other change"));
    }
    let mut label = if parts.is_empty() {
        "Approve".to_owned()
    } else {
        format!("Approve {}", parts.join(", "))
    };
    if messages > 0 {
        label.push_str(&format!(" and send {}", plural(messages, "message")));
    }
    label
}

/// Rule checks across the set's items, one line per rule, then what the evaluations add: the
/// return window and the flags that need a person's look.
fn checks(rows: &[Value], view: Option<&ChangeSetView>) -> Vec<Value> {
    let mut out = Vec::new();
    let mut by_rule: BTreeMap<String, Vec<&acom_contract::Check>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for i in view.into_iter().flat_map(|v| &v.items) {
        for c in &i.checks {
            if !by_rule.contains_key(&c.rule) {
                order.push(c.rule.clone());
            }
            by_rule.entry(c.rule.clone()).or_default().push(c);
        }
    }
    for rule in order {
        let cs = &by_rule[&rule];
        let passed = cs.iter().filter(|c| c.passed).count();
        let failed = cs.iter().find(|c| !c.passed);
        let detail = match (failed, cs.as_slice()) {
            (Some(f), _) => f.detail.clone(),
            (None, [one]) => one.detail.clone(),
            (None, _) => Some(format!("{passed} of {} pass", cs.len())),
        };
        out.push(
            json!({"rule": rule, "passed": failed.is_none(), "detail": detail,
                        "fix_tool": failed.and_then(|f| f.fix_tool.clone()),
                        "element": format!("check_row:{rule}")}),
        );
    }
    if !rows.is_empty() && !out.iter().any(|c| c["rule"] == "return_window_days") {
        let late = rows
            .iter()
            .filter(|r| r["rule"] == "return_window_days")
            .count();
        out.push(json!({"rule": "return_window_days", "passed": true,
            "detail": format!("{} of {} are inside the return window", rows.len() - late, rows.len()),
            "fix_tool": null, "element": "check_row:return_window_days"}));
    }
    let mut flagged: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in rows {
        for f in r["flags"].as_array().into_iter().flatten() {
            let rule = f["rule"].as_str().unwrap_or("flag").to_owned();
            let who = r["order_number"]
                .as_str()
                .map_or_else(String::new, |n| format!("#{n}: "));
            flagged.entry(rule).or_default().push(format!(
                "{who}{}",
                f["detail"].as_str().unwrap_or("needs a look")
            ));
        }
    }
    for (rule, details) in flagged {
        out.push(
            json!({"rule": rule, "passed": false, "detail": details.join("; "),
                        "fix_tool": "get_return_history", "element": format!("check_row:{rule}")}),
        );
    }
    out
}

/// The review. `rows` are joined requests ([`requests`]); `view` is the change set holding the
/// refunds and messages, when one was drafted; `check_intents` come from the agent definition.
pub fn refund_review(
    rows_in: &[Value],
    view: Option<&ChangeSetView>,
    check_intents: &BTreeMap<String, String>,
) -> Option<Block> {
    if rows_in.is_empty() && view.is_none() {
        return None;
    }
    let refund_items: Vec<&Value> = view
        .into_iter()
        .flat_map(|v| &v.items)
        .filter(|i| i.tool == REFUND_TOOL)
        .flat_map(|i| &i.changes)
        .filter(|c| c.entity == "refund")
        .filter_map(|c| c.to.get("amount"))
        .collect();
    let refund_amount = if refund_items.is_empty() {
        let suggested: Vec<&Value> = rows_in
            .iter()
            .filter(|r| r["suggested"] == "refund")
            .filter_map(|r| r.get("refund_amount").filter(|a| a.is_object()))
            .collect();
        total(&suggested)
    } else {
        total(&refund_items)
    };
    let refund_count = if refund_items.is_empty() {
        count(rows_in, "refund")
    } else {
        refund_items.len()
    };
    let messages: Vec<Value> = view
        .into_iter()
        .flat_map(|v| &v.items)
        .filter(|i| i.tool == MESSAGE_TOOL)
        .flat_map(|i| &i.changes)
        .map(|c| {
            let about = c.to.get("about");
            let number = rows_in
                .iter()
                .find(|r| about.is_some() && r.get("order_id") == about)
                .and_then(|r| r.get("order_number"))
                .cloned()
                .unwrap_or(Value::Null);
            json!({"to": c.to.get("recipient"), "customer_id": c.to.get("customer_id"),
                   "order_number": number, "kind": c.to.get("kind"),
                   "channel": c.to.get("channel"), "text": c.to.get("text")})
        })
        .collect();
    let data = json!({
        "summary": {"waiting": rows_in.len(),
                    "refund": {"count": refund_count, "amount": refund_amount},
                    "ask_photos": count(rows_in, "ask_for_photos"),
                    "decline": count(rows_in, "decline")},
        "requests": rows_in,
        "checks": checks(rows_in, view),
        "messages": messages,
        "change_set_id": view.map(|v| v.id.clone()),
        "version": view.map(|v| v.version),
        "state": view.map(|v| v.state.clone()),
    });
    let title = match view.and_then(|v| v.number) {
        Some(n) => format!(
            "{} · change set #{n}",
            plural(rows_in.len(), "return request")
        ),
        None => plural(rows_in.len(), "return request"),
    };
    let mut b = Block::new("refund_review", title, data);
    if !messages.is_empty() {
        b.why = Some("Messages can't be taken back once they are sent.".into());
    }
    match view {
        Some(v) => {
            let number = v.number.map_or_else(|| v.id.clone(), |n| format!("#{n}"));
            if v.state == "waiting_for_approval" {
                b.actions.push(Action::person(
                    "approve",
                    approve_label(v),
                    format!("/v1/platform/change-sets/{}/approve", v.id),
                    json!({"version": v.version}),
                    format!(
                        "Approve change set {number}? Refunds go out and messages are sent; neither can be taken back."
                    ),
                ));
                b.actions.push(Action::person(
                    "reject",
                    "Reject",
                    format!("/v1/platform/change-sets/{}/reject", v.id),
                    json!({"reason": ""}),
                    "Reject this change set? Say why so the agent can try again.",
                ));
            }
            if !messages.is_empty() {
                b.actions.push(Action::say(
                    "edit_message",
                    "Change a message",
                    json!({"change_set_id": v.id}),
                ));
            }
            b.actions.push(Action::open(
                "open_change_set",
                format!("Open change set {number}"),
                format!("/bo/change-sets/{}", v.id),
            ));
        }
        None => {
            let to_send = count(rows_in, "ask_for_photos") + count(rows_in, "decline");
            let mut label = format!("Draft {}", plural(count(rows_in, "refund"), "refund"));
            if to_send > 0 {
                label.push_str(&format!(" and {}", plural(to_send, "message")));
            }
            b.actions.push(Action::say(
                "draft",
                label,
                json!({"intent": "draft_refunds"}),
            ));
        }
    }
    for r in rows_in {
        let (Some(id), Some(s)) = (r["claim_id"].as_str(), r["suggested"].as_str()) else {
            continue;
        };
        b.suggested_intents.push(SuggestedIntent::new(
            format!("request_row:{id}"),
            format!("Why {} this one?", words(s)),
        ));
    }
    for c in b.data["checks"].as_array().cloned().unwrap_or_default() {
        if c["passed"] == false
            && let (Some(rule), Some(el)) = (c["rule"].as_str(), c["element"].as_str())
            && let Some(say) = check_intents.get(rule)
        {
            b.suggested_intents
                .push(SuggestedIntent::new(el, say.clone()));
        }
    }
    Some(b)
}

/// Rows of an earlier review (or any block) for the next turn.
pub fn rows_of(block_data: &Value) -> Vec<Value> {
    rows(block_data)
}

#[cfg(test)]
pub(crate) mod fixtures {
    use serde_json::{Value, json};

    /// (claim, customer, item, amount, reason, photos, days, suggested, refund, flag)
    type Row = (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        u64,
        u64,
        &'static str,
        Option<&'static str>,
        bool,
    );

    /// The twelve B2 requests over $200 as `search_return_requests` lists them, and their
    /// evaluations (9 refunds $2,463.00, 2 photo requests, 1 decline).
    pub fn b2() -> (Value, Vec<Value>) {
        let rows: [Row; 12] = [
            (
                "20402",
                "cus_alex",
                "Leather chelsea boots",
                "312.00",
                "defect",
                2,
                6,
                "refund",
                Some("312.00"),
                false,
            ),
            (
                "20431",
                "cus_4431",
                "Wool overcoat",
                "289.00",
                "not_as_described",
                1,
                4,
                "refund",
                Some("289.00"),
                true,
            ),
            (
                "20418",
                "cus_riley",
                "Linen blend field jacket",
                "256.00",
                "too_small",
                0,
                7,
                "refund",
                Some("256.00"),
                false,
            ),
            (
                "20377",
                "cus_sam",
                "Field jacket + windbreaker",
                "217.00",
                "changed_mind",
                0,
                9,
                "refund",
                Some("217.00"),
                false,
            ),
            (
                "20366",
                "cus_dana",
                "Light rain parka",
                "236.00",
                "damaged_in_transit",
                0,
                2,
                "ask_for_photos",
                None,
                false,
            ),
            (
                "20290",
                "cus_priya",
                "Cashmere sweater",
                "248.00",
                "changed_mind",
                0,
                39,
                "decline",
                None,
                false,
            ),
            (
                "20333",
                "cus_marcus",
                "Wool overcoat",
                "289.00",
                "changed_mind",
                0,
                12,
                "refund",
                Some("289.00"),
                false,
            ),
            (
                "20341",
                "cus_elena",
                "Leather chelsea boots",
                "312.00",
                "too_small",
                0,
                15,
                "refund",
                Some("312.00"),
                false,
            ),
            (
                "20352",
                "cus_omar",
                "Linen blend field jacket",
                "256.00",
                "too_small",
                0,
                10,
                "refund",
                Some("256.00"),
                false,
            ),
            (
                "20388",
                "cus_lena",
                "Cashmere sweater",
                "248.00",
                "changed_mind",
                0,
                8,
                "refund",
                Some("248.00"),
                false,
            ),
            (
                "20395",
                "cus_theo",
                "Cotton chore coat",
                "284.00",
                "changed_mind",
                0,
                5,
                "refund",
                Some("284.00"),
                false,
            ),
            (
                "20409",
                "cus_ivy",
                "Wool overcoat",
                "289.00",
                "damaged_in_transit",
                0,
                3,
                "ask_for_photos",
                None,
                false,
            ),
        ];
        let money = |v: &str| json!({"value": v.parse::<f64>().unwrap(), "currency": "USD"});
        let queue = json!({"items": rows.iter().map(|r| json!({
            "claim_id": format!("clm_{}", r.0), "order_id": format!("ord_{}", r.0),
            "order_number": r.0, "customer_id": r.1, "item": r.2, "amount": money(r.3),
            "reason": r.4, "photos": r.5, "days_since_delivery": r.6, "status": "requested"})).collect::<Vec<_>>(),
            "total": 12});
        let evaluations = rows
            .iter()
            .map(|r| {
                let rule = match r.7 {
                    "refund" => "defect_refund",
                    "ask_for_photos" => "damage_claim_needs_photo",
                    _ => "return_window_days",
                };
                json!({"items": [{"claim_id": format!("clm_{}", r.0), "order_number": r.0,
                    "suggested": r.7, "rule": rule, "rule_text": format!("Policy {rule}"),
                    "refund_amount": r.8.map(money),
                    "needs_person": r.8.is_some(),
                    "flags": if r.9 { json!([{"rule": "return_suggestions.flag_return_history",
                                               "detail": "4 returns in the last 90 days"}]) } else { json!([]) }}],
                       "total": 1})
            })
            .collect();
        (queue, evaluations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::change_set::ItemView;
    use acom_contract::{Change, ChangeOp, Check};

    fn evaluations() -> (Vec<Value>, Vec<Value>) {
        let (queue, evals) = fixtures::b2();
        (
            rows(&queue),
            evals.iter().flat_map(rows).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn twelve_requests_sort_into_nine_refunds_two_photo_requests_and_a_decline() {
        let (queue, evals) = evaluations();
        let rows = requests(&queue, &evals);
        assert_eq!(rows.len(), 12);
        assert_eq!(rows[0]["evidence"], "2 photos");
        assert_eq!(rows[0]["day"], 6);
        let b = refund_review(&rows, None, &BTreeMap::new()).unwrap();
        let s = &b.data["summary"];
        assert_eq!(s["waiting"], 12);
        assert_eq!(s["refund"]["count"], 9);
        assert_eq!(s["refund"]["amount"]["value"].to_string(), "2463.00");
        assert_eq!(s["ask_photos"], 2);
        assert_eq!(s["decline"], 1);
        let window = b.data["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["rule"] == "return_window_days")
            .unwrap();
        assert_eq!(window["detail"], "11 of 12 are inside the return window");
        let flag = b.data["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["rule"] == "return_suggestions.flag_return_history")
            .unwrap();
        assert_eq!(flag["passed"], false);
        assert_eq!(flag["detail"], "#20431: 4 returns in the last 90 days");
        assert!(matches!(&b.actions[..], [Action::Say { label, .. }]
            if label == "Draft 9 refunds and 3 messages"));
        assert_eq!(b.person_actions().count(), 0);
        // The next turn reads the same rows back from the block.
        assert_eq!(requests(&rows_of(&b.data), &[]).len(), 12);
    }

    fn item(position: usize, tool: &str, to: Value, check: Check) -> ItemView {
        ItemView {
            position,
            tool: tool.into(),
            label: tool.into(),
            preview_id: format!("pv_{position}"),
            changes: vec![Change {
                op: if tool == REFUND_TOOL {
                    ChangeOp::Create
                } else {
                    ChangeOp::Send
                },
                entity: if tool == REFUND_TOOL {
                    "refund".into()
                } else {
                    "message".into()
                },
                id: String::new(),
                from: Value::Null,
                to,
                note: None,
            }],
            changes_total: 1,
            checks: vec![check],
        }
    }

    #[test]
    fn one_change_set_holds_nine_refunds_and_three_messages() {
        let (queue, evals) = evaluations();
        let rows = requests(&queue, &evals);
        let mut items = Vec::new();
        for r in rows.iter().filter(|r| r["suggested"] == "refund") {
            items.push(item(
                items.len() + 1,
                REFUND_TOOL,
                json!({"claim_id": r["claim_id"], "amount": r["refund_amount"]}),
                Check {
                    rule: "refund.to".into(),
                    passed: true,
                    detail: Some("back to the card".into()),
                    fix_tool: None,
                },
            ));
        }
        for r in rows.iter().filter(|r| r["suggested"] != "refund") {
            items.push(item(
                items.len() + 1,
                MESSAGE_TOOL,
                json!({"recipient": "D*** R***", "customer_id": r["customer_id"], "about": r["order_id"],
                       "kind": r["suggested"], "channel": "email", "text": "Kind words."}),
                Check {
                    rule: "customer_messages".into(),
                    passed: true,
                    detail: Some("Every customer message needs a person's approval".into()),
                    fix_tool: None,
                },
            ));
        }
        let view = ChangeSetView {
            id: "cs_1043".into(),
            number: Some(1043),
            version: 1,
            state: "waiting_for_approval".into(),
            items,
            ..ChangeSetView::default()
        };
        assert_eq!(
            approve_label(&view),
            "Approve 9 refunds and send 3 messages"
        );
        let b = refund_review(&rows, Some(&view), &BTreeMap::new()).unwrap();
        assert_eq!(b.data["change_set_id"], "cs_1043");
        assert_eq!(
            b.data["summary"]["refund"]["amount"]["value"].to_string(),
            "2463.00"
        );
        assert_eq!(b.data["messages"].as_array().unwrap().len(), 3);
        assert_eq!(b.data["messages"][0]["order_number"], "20366");
        let refund_to = b.data["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["rule"] == "refund.to")
            .unwrap();
        assert_eq!(refund_to["detail"], "9 of 9 pass");
        match &b.actions[0] {
            Action::Person {
                label, path, body, ..
            } => {
                assert_eq!(label, "Approve 9 refunds and send 3 messages");
                assert_eq!(path, "/v1/platform/change-sets/cs_1043/approve");
                assert_eq!(body, &json!({"version": 1}));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            b.why.as_deref(),
            Some("Messages can't be taken back once they are sent.")
        );
    }
}
