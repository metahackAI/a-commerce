//! `task_list` (docs/api/chat.md §3.3): the operator's home in the conversation, built from
//! `list_tasks` output as it is. Tasks are grouped as the home shows them (needs you, working,
//! done); every item can be opened and asked about. Nothing here is computed: titles, times and
//! summaries are the tool's.

use serde_json::{Value, json};

use super::shop::items_of;
use super::{Action, Block, SuggestedIntent};

/// The groups, in the order the home shows them.
const GROUPS: &[(&str, &str)] = &[
    ("needs_you", "Needs your decision"),
    ("working", "Agent is working"),
    ("done", "Done"),
    ("dismissed", "Dismissed"),
];

/// Item fields copied from `list_tasks`.
const FIELDS: &[&str] = &[
    "title",
    "origin",
    "at",
    "summary",
    "tag",
    "change_set_id",
    "watcher",
    "updated_at",
];

/// A `task_list` block from a `list_tasks` result; `None` when it holds no task.
pub fn task_list(output: &Value) -> Option<Block> {
    let items = items_of(output);
    let tasks: Vec<&Value> = items
        .iter()
        .filter(|t| t.get("id").and_then(Value::as_str).is_some())
        .collect();
    if tasks.is_empty() {
        return None;
    }
    let mut groups = Vec::new();
    for (state, _) in GROUPS {
        let members: Vec<Value> = tasks
            .iter()
            .filter(|t| t.get("state").and_then(Value::as_str) == Some(state))
            .map(|t| {
                let mut item = json!({"task_id": t["id"]});
                for f in FIELDS {
                    if let Some(v) = t.get(*f) {
                        item[*f] = v.clone();
                    }
                }
                item
            })
            .collect();
        if !members.is_empty() {
            groups.push(json!({"state": state, "items": members}));
        }
    }
    let total = output
        .get("total")
        .and_then(Value::as_u64)
        .unwrap_or(tasks.len() as u64);
    let waiting = tasks.iter().filter(|t| t["state"] == "needs_you").count();
    let title = match waiting {
        0 => "Nothing needs you".to_owned(),
        1 => "1 task needs you".to_owned(),
        n => format!("{n} tasks need you"),
    };
    let mut b = Block::new(
        "task_list",
        title,
        json!({"groups": groups, "total": total}),
    );
    for t in &tasks {
        let Some(id) = t["id"].as_str() else {
            continue;
        };
        b.actions.push(Action::open(
            &format!("open:{id}"),
            t["title"].as_str().unwrap_or(id),
            format!("/bo/tasks/{id}"),
        ));
        let say = match t["state"].as_str() {
            Some("needs_you") => "What does this need from me?",
            Some("working") => "How far along is this?",
            _ => "What happened here?",
        };
        b.suggested_intents
            .push(SuggestedIntent::new(format!("task_item:{id}"), say));
    }
    if total > tasks.len() as u64 {
        b.actions.push(Action::say("more", "Show more", json!({})));
    }
    Some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Value {
        json!({"items": [
            {"id": "task_returns_1106", "title": "12 return requests over $200 are waiting",
             "state": "working", "origin": "agent_noticed", "at": "2026-11-06T10:40:00-05:00",
             "summary": "Operations agent is sorting the requests by policy", "tag": "returns",
             "change_set_id": null, "watcher": "returns_over_refund_threshold",
             "updated_at": "2026-11-06T11:10:00-05:00"},
            {"id": "task_linen_reorder", "title": "Linen shirt sells out in about 4 days",
             "state": "needs_you", "origin": "agent_noticed", "at": "2026-11-06T08:40:00-05:00",
             "summary": "Reorder draft: 3 sizes, 240 units", "tag": "inventory",
             "change_set_id": "cs_1041", "watcher": "stock_out_forecast"},
            {"id": "task_below_cost", "title": "4 products were priced below cost", "state": "done",
             "origin": "automation", "at": "2026-11-06T07:55:00-05:00",
             "summary": "Raised to the price floor at 08:31; undo until Nov 7", "tag": "pricing",
             "change_set_id": "cs_1040", "watcher": "price_below_cost"}],
          "total": 3})
    }

    #[test]
    fn groups_tasks_as_the_home_does() {
        let b = task_list(&list()).unwrap();
        assert_eq!(b.ui, "task_list");
        assert_eq!(b.title, "1 task needs you");
        let states: Vec<&str> = b.data["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["state"].as_str().unwrap())
            .collect();
        assert_eq!(states, ["needs_you", "working", "done"]);
        let working = &b.data["groups"][1]["items"][0];
        assert_eq!(working["task_id"], "task_returns_1106");
        assert_eq!(working["updated_at"], "2026-11-06T11:10:00-05:00");
        assert_eq!(working["watcher"], "returns_over_refund_threshold");
        assert_eq!(b.data["groups"][0]["items"][0]["change_set_id"], "cs_1041");
        assert_eq!(b.actions.len(), 3);
        assert!(
            b.person_actions().next().is_none(),
            "opening is not deciding"
        );
        assert_eq!(
            b.suggested_intents[0].element,
            "task_item:task_returns_1106"
        );
    }

    #[test]
    fn an_empty_list_is_no_block_and_a_long_one_offers_more() {
        assert!(task_list(&json!({"items": [], "total": 0})).is_none());
        let mut l = list();
        l["total"] = json!(9);
        let b = task_list(&l).unwrap();
        assert!(
            b.actions
                .iter()
                .any(|a| matches!(a, Action::Say { action_id, .. } if action_id == "more"))
        );
    }
}
