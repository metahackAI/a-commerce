//! Choosing the specialist agent for a turn (docs/backend/agent.md §2).
//!
//! The worker model picks one agent id from the surface's agents under a closed schema
//! (`{agent, confidence}`). Below the configured confidence the conversation's last agent, or
//! else the surface's default, takes the turn. A surface with one agent, or a button intent
//! (which continues the conversation's work), needs no model call.

use acom_llm::Message;
use serde_json::{Value, json};

use crate::profile::{AgentProfile, RouteHint};

/// The routing decision and how it was made, for the step log.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub agent: String,
    pub confidence: f64,
    pub reason: &'static str,
}

/// The agent whose `route_when_said` phrase found in the request is the longest ("why did
/// sales" beats "sale"); `None` when no phrase matches or two agents tie.
pub fn by_phrase(agents: &[&AgentProfile], request: &str) -> Option<Route> {
    let text = request.to_lowercase();
    // Each agent's longest phrase found in the request; the longest of those wins ("why did
    // sales" over "sale"), and a tie between agents is left to the model.
    let mut best: Vec<(usize, &AgentProfile)> = agents
        .iter()
        .filter_map(|a| {
            a.route_when_said
                .iter()
                .filter(|p| text.contains(p.as_str()))
                .map(String::len)
                .max()
                .map(|n| (n, *a))
        })
        .collect();
    best.sort_by_key(|b| std::cmp::Reverse(b.0));
    match best.as_slice() {
        [] => None,
        [(n, _), (m, _), ..] if n == m => None,
        [(_, a), ..] => Some(Route {
            agent: a.id.clone(),
            confidence: 1.0,
            reason: "the request names this agent's work",
        }),
    }
}

/// Whether choosing needs the model.
pub fn needs_model(
    agents: &[&AgentProfile],
    is_button_intent: bool,
    previous: Option<&str>,
) -> bool {
    agents.len() > 1 && !(is_button_intent && previous.is_some())
}

pub fn schema(agents: &[&AgentProfile]) -> Value {
    let ids: Vec<&str> = agents.iter().map(|a| a.id.as_str()).collect();
    json!({
        "type": "object", "additionalProperties": false, "required": ["agent", "confidence"],
        "properties": {
            "agent": {"type": "string", "enum": ids},
            "confidence": {"type": "number", "minimum": 0, "maximum": 1}
        }
    })
}

pub fn messages(
    prompt: &str,
    agents: &[&AgentProfile],
    request: &str,
    previous: Option<&str>,
) -> Vec<Message> {
    let list: Vec<Value> = agents
        .iter()
        .map(|a| json!({"agent": a.id, "description": a.description}))
        .collect();
    let mut user = format!("Agents: {}\n\nRequest: {request}\n", Value::Array(list));
    if let Some(p) = previous {
        user.push_str(&format!("The conversation's agent so far: {p}\n"));
    }
    vec![Message::system(prompt.trim()), Message::user(user)]
}

/// A routing hint the request matches (`agents/runtime.yml router.hints`): a phrase it says, or
/// a kind of file attached. Only when the hints name exactly one agent of this surface.
pub fn hinted(
    hints: &[RouteHint],
    agents: &[&AgentProfile],
    request: &str,
    attached: &[String],
) -> Option<Route> {
    let text = request.to_lowercase();
    let mut chosen: Vec<&str> = hints
        .iter()
        .filter(|h| agents.iter().any(|a| a.id == h.agent))
        .filter(|h| {
            h.when_said.iter().any(|p| text.contains(&p.to_lowercase()))
                || h.when_attached.iter().any(|k| attached.contains(k))
        })
        .map(|h| h.agent.as_str())
        .collect();
    chosen.sort_unstable();
    chosen.dedup();
    match chosen.as_slice() {
        [one] => Some(Route {
            agent: (*one).to_owned(),
            confidence: 1.0,
            reason: "matched a routing hint",
        }),
        _ => None,
    }
}

/// The decision from the model's answer (or its absence).
pub fn decide(
    answer: Option<&Value>,
    agents: &[&AgentProfile],
    default: &str,
    previous: Option<&str>,
    min_confidence: f64,
) -> Route {
    let fallback = |reason| Route {
        agent: previous
            .filter(|p| agents.iter().any(|a| a.id == *p))
            .unwrap_or(default)
            .to_owned(),
        confidence: 0.0,
        reason,
    };
    if agents.len() == 1 {
        return Route {
            agent: agents[0].id.clone(),
            confidence: 1.0,
            reason: "only agent for this surface",
        };
    }
    let Some(answer) = answer else {
        return fallback("kept the conversation's agent");
    };
    let agent = answer.get("agent").and_then(Value::as_str);
    let confidence = answer
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    match agent {
        Some(a) if agents.iter().any(|p| p.id == a) && confidence >= min_confidence => Route {
            agent: a.to_owned(),
            confidence,
            reason: "chosen by the model",
        },
        Some(a) if agents.iter().any(|p| p.id == a) => Route {
            confidence,
            ..fallback("the model was unsure")
        },
        _ => fallback("the model named no known agent"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::ChatSurface;
    use crate::profile::fixtures::repo;

    #[test]
    fn confident_answers_win_and_unsure_ones_fall_back() {
        let p = repo();
        let bo = p.for_surface(ChatSurface::Bo);
        let r = decide(
            Some(&json!({"agent": "orders", "confidence": 0.9})),
            &bo,
            "promotion",
            None,
            0.5,
        );
        assert_eq!(
            (r.agent.as_str(), r.reason),
            ("orders", "chosen by the model")
        );
        let r = decide(
            Some(&json!({"agent": "orders", "confidence": 0.2})),
            &bo,
            "promotion",
            Some("promotion"),
            0.5,
        );
        assert_eq!(r.agent, "promotion");
        let r = decide(
            Some(&json!({"agent": "pirate"})),
            &bo,
            "promotion",
            Some("orders"),
            0.5,
        );
        assert_eq!(r.agent, "orders");
        let r = decide(None, &bo, "promotion", Some("shopper"), 0.5);
        assert_eq!(
            r.agent, "promotion",
            "an agent of another surface is never kept"
        );
    }

    #[test]
    fn sales_questions_go_to_the_insights_agent_by_phrase() {
        let p = repo();
        let bo = p.for_surface(ChatSurface::Bo);
        let r = by_phrase(&bo, "Why did sales drop yesterday?").unwrap();
        assert_eq!((r.agent.as_str(), r.confidence), ("insights", 1.0));
        assert_eq!(
            by_phrase(&bo, "How was revenue last week?").map(|r| r.agent),
            Some("insights".to_owned())
        );
        assert_eq!(
            by_phrase(
                &bo,
                "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."
            )
            .map(|r| r.agent),
            Some("promotion".to_owned()),
            "a sale to set up is not a sales question: the longer phrase wins"
        );
        let to = |text: &str| by_phrase(&bo, text).map(|r| r.agent);
        assert_eq!(
            to("Go through the return requests over $200 and suggest what to do with each."),
            Some("orders".to_owned())
        );
        assert_eq!(
            to("Which products are low on stock?"),
            Some("catalog".to_owned())
        );
        assert_eq!(
            to("How many VIP members do we have?"),
            Some("insights".to_owned())
        );
        assert_eq!(to("What needs me today?"), Some("promotion".to_owned()));
        assert_eq!(to("Hello there"), None);
        let fo = p.for_surface(ChatSurface::Fo);
        assert!(by_phrase(&fo, "Why did sales drop yesterday?").is_none());
    }

    #[test]
    fn supplier_sheets_go_to_the_catalog_agent_without_the_model() {
        let p = repo();
        let bo = p.for_surface(ChatSurface::Bo);
        let hints = &p.runtime.router.hints;
        let r = hinted(
            hints,
            &bo,
            "List the 30 products in this supplier sheet. Use our usual pricing and write the descriptions in our tone.",
            &[],
        )
        .unwrap();
        assert_eq!(
            (r.agent.as_str(), r.reason),
            ("catalog", "matched a routing hint")
        );
        let r = hinted(hints, &bo, "Add these", &["sheet".to_owned()]).unwrap();
        assert_eq!(r.agent, "catalog");
        assert!(hinted(hints, &bo, "Run a 20% weekend sale", &[]).is_none());
        let fo = p.for_surface(ChatSurface::Fo);
        assert!(
            hinted(hints, &fo, "supplier sheet", &[]).is_none(),
            "only this surface's agents"
        );
    }

    #[test]
    fn one_agent_or_a_button_needs_no_model() {
        let p = repo();
        let fo = p.for_surface(ChatSurface::Fo);
        assert!(!needs_model(&fo, false, None));
        assert_eq!(decide(None, &fo, "shopper", None, 0.5).agent, "shopper");
        let bo = p.for_surface(ChatSurface::Bo);
        assert!(needs_model(&bo, false, Some("promotion")));
        assert!(!needs_model(&bo, true, Some("promotion")));
        let s = schema(&bo);
        assert!(s["properties"]["agent"]["enum"].as_array().unwrap().len() >= 2);
        let m = messages("Pick one.", &bo, "Refund order 20402", Some("orders"));
        assert!(m[1].content.contains("orders"));
    }
}
