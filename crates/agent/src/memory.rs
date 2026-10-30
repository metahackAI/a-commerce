//! Conversation context (docs/backend/agent.md §1, §9): recent turns, the ids that came up, and
//! the open change set. The agent keeps no hidden memory about shoppers: what it remembers is
//! the customer domain's preferences, shown as a `memory_note` the shopper can undo.

use std::collections::BTreeMap;

use acom_auth::Principal;
use serde_json::{Value, json};

/// One earlier turn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PastTurn {
    pub text: String,
    pub reply: Option<String>,
    pub agent: String,
    pub blocks: Vec<Value>,
}

/// One item of a change set the conversation drafted.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenItem {
    pub position: usize,
    pub tool: String,
    pub preview_id: String,
    /// The arguments it was previewed with (from the step log).
    pub input: Value,
}

/// The change set the conversation is working on, as last seen.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OpenChangeSet {
    pub id: String,
    pub number: Option<i64>,
    pub version: i32,
    pub state: String,
    pub items: Vec<OpenItem>,
    /// The last `change_set` block's data: failed checks and rows for the planner.
    pub last_view: Value,
}

impl OpenChangeSet {
    /// Whether items can still change (drafting or waiting for approval).
    pub fn is_open(&self) -> bool {
        matches!(self.state.as_str(), "drafting" | "waiting_for_approval")
    }

    /// What the planner is told: items with their arguments, failed checks, and some rows.
    pub fn summary(&self) -> Value {
        let failed: Vec<Value> = self.last_view["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["passed"] == false)
            .map(|c| json!({"rule": c["rule"], "detail": c["detail"], "tool": c["tool"]}))
            .collect();
        let rows: Vec<Value> = self.last_view["groups"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|g| g["rows"].as_array().cloned().unwrap_or_default())
            .take(12)
            .map(|r| json!({"element": r["element"], "to": r["to"]}))
            .collect();
        json!({
            "change_set_id": self.id, "version": self.version, "state": self.state,
            "items": self.items.iter().map(|i| json!({"position": i.position, "tool": i.tool, "args": i.input})).collect::<Vec<_>>(),
            "failed_checks": failed, "rows": rows,
            "left_out": self.last_view.get("left_out").cloned().unwrap_or(Value::Null),
        })
    }
}

/// Who a conversation belongs to. A conversation is only continued by the same person (or,
/// for a guest, the same browser session).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Owner {
    pub customer_id: Option<String>,
    pub admin_id: Option<String>,
    pub session_id: Option<String>,
}

impl Owner {
    pub fn of(p: &Principal) -> Self {
        let id = |prefix: &str| p.subject.starts_with(prefix).then(|| p.subject.clone());
        Self {
            customer_id: id("cus_"),
            admin_id: id("adm_"),
            session_id: p.session_id.clone(),
        }
    }

    pub fn same_person(&self, other: &Owner) -> bool {
        match (&self.customer_id, &self.admin_id) {
            (Some(c), _) => other.customer_id.as_ref() == Some(c),
            (None, Some(a)) => other.admin_id.as_ref() == Some(a),
            (None, None) => {
                other.customer_id.is_none()
                    && other.admin_id.is_none()
                    && self.session_id.is_some()
                    && self.session_id == other.session_id
            }
        }
    }
}

/// What a turn starts from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConversationContext {
    pub conversation_id: String,
    pub owner: Owner,
    /// The agent of the last turn.
    pub agent: Option<String>,
    /// Earlier turns, oldest first.
    pub turns: Vec<PastTurn>,
    pub change_set: Option<OpenChangeSet>,
}

impl ConversationContext {
    pub fn new(conversation_id: &str, owner: Owner) -> Self {
        Self {
            conversation_id: conversation_id.to_owned(),
            owner,
            ..Self::default()
        }
    }

    pub fn is_first_turn(&self) -> bool {
        self.turns.is_empty()
    }

    /// One line per earlier turn for the planner. Blocks are named with their id, so a step can
    /// use an earlier block's rows (`uses`/`each`: `blk_…`).
    pub fn history(&self, n: usize) -> Vec<String> {
        let skip = self.turns.len().saturating_sub(n);
        self.turns
            .iter()
            .skip(skip)
            .map(|t| {
                let shown: Vec<String> = t
                    .blocks
                    .iter()
                    .filter_map(|b| {
                        let ui = b.get("ui").and_then(Value::as_str)?;
                        Some(match b.get("block_id").and_then(Value::as_str) {
                            Some(id) => format!("{ui} {id}"),
                            None => ui.to_owned(),
                        })
                    })
                    .collect();
                format!(
                    "Person: {} | Agent ({}): {}{}",
                    t.text,
                    t.agent,
                    t.reply.as_deref().unwrap_or("(no text)"),
                    if shown.is_empty() {
                        String::new()
                    } else {
                        format!(" [showed {}]", shown.join(", "))
                    }
                )
            })
            .collect()
    }

    /// An earlier block by id.
    pub fn block(&self, block_id: &str) -> Option<&Value> {
        self.turns
            .iter()
            .rev()
            .flat_map(|t| t.blocks.iter())
            .find(|b| b.get("block_id").and_then(Value::as_str) == Some(block_id))
    }

    /// The most recent earlier block of one of these kinds.
    pub fn last_block(&self, uis: &[&str]) -> Option<&Value> {
        self.turns.iter().rev().find_map(|t| {
            t.blocks.iter().rev().find(|b| {
                b.get("ui")
                    .and_then(Value::as_str)
                    .is_some_and(|u| uis.contains(&u))
            })
        })
    }

    /// What the person said in earlier turns, newest first.
    pub fn said_before(&self) -> impl Iterator<Item = &str> {
        self.turns.iter().rev().map(|t| t.text.as_str())
    }

    /// Ids that came up in earlier blocks, by field name (`product_id` → […]), newest first.
    pub fn entities(&self) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for t in self.turns.iter().rev() {
            for b in &t.blocks {
                collect_ids(b.get("data").unwrap_or(&Value::Null), &mut out);
            }
        }
        out
    }
}

fn is_id(s: &str) -> bool {
    let Some((prefix, rest)) = s.split_once('_') else {
        return false;
    };
    (2..=5).contains(&prefix.len())
        && prefix.bytes().all(|b| b.is_ascii_lowercase())
        && !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn collect_ids(v: &Value, out: &mut BTreeMap<String, Vec<String>>) {
    match v {
        Value::Object(o) => {
            for (k, x) in o {
                if let Some(s) = x.as_str()
                    && k.ends_with("_id")
                    && is_id(s)
                {
                    let list = out.entry(k.clone()).or_default();
                    if !list.iter().any(|e| e == s) && list.len() < 20 {
                        list.push(s.to_owned());
                    }
                }
                collect_ids(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_ids(x, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_core::TenantId;
    use acom_core::enums::ActorKind;

    fn principal(subject: &str, session: Option<&str>) -> Principal {
        Principal {
            subject: subject.into(),
            kind: ActorKind::AgentOnBehalf,
            tenant: TenantId::parse("store_northwind").unwrap(),
            surface: acom_auth::Surface::Fo,
            scopes: Default::default(),
            name: None,
            session_id: session.map(str::to_owned),
        }
    }

    #[test]
    fn conversations_belong_to_one_person_or_one_guest_session() {
        let jamie = Owner::of(&principal("cus_jamie", Some("ses_1")));
        assert!(jamie.same_person(&Owner::of(&principal("cus_jamie", Some("ses_2")))));
        assert!(!jamie.same_person(&Owner::of(&principal("cus_alex", Some("ses_1")))));
        let guest = Owner::of(&principal("ses_9", Some("ses_9")));
        assert!(guest.same_person(&Owner::of(&principal("ses_9", Some("ses_9")))));
        assert!(!guest.same_person(&Owner::of(&principal("ses_8", Some("ses_8")))));
        assert!(!guest.same_person(&jamie));
    }

    #[test]
    fn history_entities_and_change_set_summary() {
        let mut c = ConversationContext::new("conv_1", Owner::default());
        c.turns.push(PastTurn {
            text: "Run a 20% weekend sale".into(),
            reply: Some("Drafted.".into()),
            agent: "promotion".into(),
            blocks: vec![json!({"ui": "change_set", "data": {"change_set_id": "cs_1042",
                "groups": [{"rows": [{"element": "change_row:prd_a", "to": {"product_id": "prd_a"}}]}],
                "checks": [{"rule": "price_floor", "passed": false, "detail": "x", "tool": "set_sale_prices"}]}})],
        });
        assert_eq!(
            c.history(4),
            ["Person: Run a 20% weekend sale | Agent (promotion): Drafted. [showed change_set]"]
        );
        c.turns.push(PastTurn {
            text: "Review the returns".into(),
            reply: None,
            agent: "orders".into(),
            blocks: vec![
                json!({"ui": "refund_review", "block_id": "blk_7", "data": {"requests": []}}),
            ],
        });
        assert!(c.history(4)[1].ends_with("[showed refund_review blk_7]"));
        assert_eq!(c.block("blk_7").unwrap()["ui"], "refund_review");
        assert!(c.block("blk_8").is_none());
        assert_eq!(
            c.last_block(&["change_set", "refund_review"]).unwrap()["ui"],
            "refund_review"
        );
        assert_eq!(c.said_before().next(), Some("Review the returns"));
        c.turns.pop();
        let e = c.entities();
        assert_eq!(e["change_set_id"], ["cs_1042"]);
        assert_eq!(e["product_id"], ["prd_a"]);

        let cs = OpenChangeSet {
            id: "cs_1042".into(),
            version: 1,
            state: "drafting".into(),
            items: vec![OpenItem {
                position: 1,
                tool: "set_sale_prices".into(),
                preview_id: "pv_1".into(),
                input: json!({"percent_off": 20}),
            }],
            last_view: c.turns[0].blocks[0]["data"].clone(),
            number: Some(1042),
        };
        assert!(cs.is_open());
        let s = cs.summary();
        assert_eq!(s["items"][0]["args"]["percent_off"], 20);
        assert_eq!(s["failed_checks"][0]["rule"], "price_floor");
        assert_eq!(s["rows"][0]["element"], "change_row:prd_a");
    }
}
