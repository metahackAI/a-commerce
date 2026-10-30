//! Choosing the specialist agent for a turn (docs/backend/agent.md §2).
//!
//! The worker model picks one agent id from the surface's agents under a closed schema
//! (`{agent, confidence}`). Below the configured confidence the conversation's last agent, or
//! else the surface's default, takes the turn. A surface with one agent, or a button intent
//! (which continues the conversation's work), needs no model call.

use acom_llm::Message;
use serde_json::{Value, json};

use crate::profile::AgentProfile;

/// The routing decision and how it was made, for the step log.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub agent: String,
    pub confidence: f64,
    pub reason: &'static str,
}

/// The agent whose `route_when_said` phrases the request contains, when exactly one agent's do:
/// "Why did sales drop yesterday?" goes to the insights agent without a model call.
pub fn by_phrase(agents: &[&AgentProfile], request: &str) -> Option<Route> {
    let text = request.to_lowercase();
    let mut hits = agents
        .iter()
        .filter(|a| a.route_when_said.iter().any(|p| text.contains(p.as_str())));
    let first = hits.next()?;
    if hits.next().is_some() {
        return None;
    }
    Some(Route {
        agent: first.id.clone(),
        confidence: 1.0,
        reason: "the request names this agent's work",
    })
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
        assert!(
            by_phrase(
                &bo,
                "Run a 20% weekend sale on summer outerwear for VIP members and put it on the main page."
            )
            .is_none(),
            "a sale to set up is not a sales question"
        );
        assert!(by_phrase(&bo, "Go through the returns over $200").is_none());
        let fo = p.for_surface(ChatSurface::Fo);
        assert!(by_phrase(&fo, "Why did sales drop yesterday?").is_none());
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
