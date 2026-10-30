//! The plan (docs/backend/agent.md §4): a short list of tool calls the planner model writes as
//! JSON under a schema (guided decoding), checked here before anything runs.
//!
//! Ids never pass through the model: a step names earlier steps in `uses`, and the executor
//! fills the consumed slots from what those steps produced.

use std::collections::BTreeSet;

use acom_llm::Message;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::tools::ToolSpec;

/// One line of "what I understood".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Understood {
    /// `who`, `products`, `benefit`, `when`, `where`, or `other`.
    pub aspect: String,
    pub text: String,
}

/// Whom the request is about. Nothing is remembered about someone else (a gift).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum About {
    #[default]
    #[serde(rename = "self")]
    Myself,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: String,
    pub tool: String,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub uses: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// A short title for the change set, when the plan writes.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub interpretation: Vec<Understood>,
    #[serde(default)]
    pub about: About,
    #[serde(default)]
    pub steps: Vec<PlanStep>,
    /// The block to answer with, or `text`.
    #[serde(default)]
    pub reply_plan: Option<String>,
}

const ASPECTS: &[&str] = &["who", "products", "benefit", "when", "where", "other"];

/// The JSON Schema the planner must follow: tool names and blocks are closed lists.
pub fn plan_schema(tools: &[&ToolSpec], blocks: &[String], max_steps: usize) -> Value {
    let names: Vec<&str> = tools.iter().map(|t| t.tool.as_str()).collect();
    let mut replies: Vec<&str> = blocks.iter().map(String::as_str).collect();
    replies.push("text");
    json!({
        "type": "object", "additionalProperties": false,
        "required": ["interpretation", "about", "steps", "reply_plan"],
        "properties": {
            "title": {"type": "string", "description": "Short title of the change, a few words."},
            "interpretation": {"type": "array", "maxItems": 6, "items": {
                "type": "object", "additionalProperties": false, "required": ["aspect", "text"],
                "properties": {"aspect": {"type": "string", "enum": ASPECTS},
                               "text": {"type": "string"}}}},
            "about": {"type": "string", "enum": ["self", "other"]},
            "steps": {"type": "array", "maxItems": max_steps, "items": {
                "type": "object", "additionalProperties": false,
                "required": ["id", "tool", "args", "uses"],
                "properties": {
                    "id": {"type": "string", "description": "s1, s2, …"},
                    "tool": {"type": "string", "enum": names},
                    "args": {"type": "object"},
                    "uses": {"type": "array", "items": {"type": "string"},
                             "description": "Ids of earlier steps whose results this step needs."}}}},
            "reply_plan": {"type": "string", "enum": replies}
        }
    })
}

/// Checks a plan against the candidates and makes it safe to run: unknown tools and steps are
/// refused, step ids are made unique, `uses` may only point backwards, `args` is an object.
pub fn validate(raw: &Value, tools: &[&ToolSpec], blocks: &[String]) -> Result<Plan, String> {
    let mut plan: Plan = serde_json::from_value(raw.clone())
        .map_err(|e| format!("the plan is not readable: {e}"))?;
    let known: BTreeSet<&str> = tools.iter().map(|t| t.tool.as_str()).collect();
    let unknown: Vec<&str> = plan
        .steps
        .iter()
        .map(|s| s.tool.as_str())
        .filter(|t| !known.contains(t))
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "the plan uses tools that are not available: {}",
            unknown.join(", ")
        ));
    }
    let mut seen: Vec<String> = Vec::new();
    for (n, step) in plan.steps.iter_mut().enumerate() {
        if step.id.trim().is_empty() || seen.contains(&step.id) {
            step.id = format!("s{}", n + 1);
        }
        step.uses.retain(|u| seen.contains(u));
        if !step.args.is_object() {
            step.args = match &step.args {
                Value::String(s) => acom_llm::parse::extract_json(s).unwrap_or(json!({})),
                _ => json!({}),
            };
        }
        seen.push(step.id.clone());
    }
    plan.interpretation.retain(|u| !u.text.trim().is_empty());
    for u in &mut plan.interpretation {
        if !ASPECTS.contains(&u.aspect.as_str()) {
            u.aspect = "other".into();
        }
    }
    if plan
        .reply_plan
        .as_deref()
        .is_some_and(|r| r != "text" && !blocks.iter().any(|b| b == r))
    {
        plan.reply_plan = None;
    }
    Ok(plan)
}

impl Plan {
    /// The distinct domains of its steps.
    pub fn domains<'a>(&self, tools: &[&'a ToolSpec]) -> BTreeSet<&'a str> {
        self.steps
            .iter()
            .filter_map(|s| tools.iter().find(|t| t.tool == s.tool))
            .map(|t| t.domain())
            .collect()
    }

    /// The interpretation as stored with a change set.
    pub fn interpretation_values(&self) -> Vec<Value> {
        self.interpretation
            .iter()
            .map(|u| json!({"aspect": u.aspect, "text": u.text}))
            .collect()
    }
}

/// Everything the planner is told.
#[derive(Debug, Clone, Default)]
pub struct PlanInput<'a> {
    pub shared_prompt: &'a str,
    pub instructions: &'a str,
    /// "Fri 2026-11-06 11:02 (America/New_York, UTC-05:00)".
    pub now: String,
    pub currency: &'a str,
    pub request: String,
    /// Elements the person pointed at, and the button intent, as JSON lines.
    pub pointed: Vec<Value>,
    /// Earlier turns, oldest first.
    pub history: Vec<String>,
    /// The open change set, summarized.
    pub open_change_set: Option<Value>,
    pub tools: Vec<&'a ToolSpec>,
    pub blocks: &'a [String],
    /// What already ran this turn (for a revision).
    pub done: Vec<Value>,
    /// Why the plan must change (a failed step, a mismatch).
    pub problem: Option<String>,
    /// A smaller model's draft, for the planner model to improve.
    pub draft: Option<Value>,
}

/// Tool descriptions for the prompt: kind, links, and the input schema without examples noise.
pub fn describe_tool(t: &ToolSpec) -> Value {
    let mut schema = t.input_schema.clone();
    strip_keys(&mut schema, &["examples", "$schema", "title"]);
    let mut d = Map::new();
    d.insert("tool".into(), json!(t.tool));
    d.insert("kind".into(), json!(t.kind_words()));
    d.insert("domain".into(), json!(t.domain()));
    d.insert("summary".into(), json!(t.summary));
    if !t.consumes.is_empty() {
        d.insert(
            "consumes".into(),
            json!(
                t.consumes
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            ),
        );
    }
    if !t.produces.is_empty() {
        d.insert(
            "produces".into(),
            json!(
                t.produces
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            ),
        );
    }
    d.insert("input".into(), schema);
    Value::Object(d)
}

fn strip_keys(v: &mut Value, keys: &[&str]) {
    match v {
        Value::Object(o) => {
            for k in keys {
                o.remove(*k);
            }
            o.values_mut().for_each(|c| strip_keys(c, keys));
        }
        Value::Array(a) => a.iter_mut().for_each(|c| strip_keys(c, keys)),
        _ => {}
    }
}

/// The planner's messages.
pub fn messages(input: &PlanInput<'_>) -> Vec<Message> {
    let system = format!(
        "{}\n\n{}",
        input.shared_prompt.trim(),
        input.instructions.trim()
    );
    let mut user = format!(
        "Store clock: {}. Store currency: {}.\n\nRequest: {}\n",
        input.now,
        if input.currency.is_empty() {
            "unknown"
        } else {
            input.currency
        },
        input.request
    );
    if !input.pointed.is_empty() {
        user.push_str(&format!(
            "\nThe person pointed at (use these ids for \"this\"): {}\n",
            Value::Array(input.pointed.clone())
        ));
    }
    if !input.history.is_empty() {
        user.push_str("\nEarlier in this conversation:\n");
        for h in &input.history {
            user.push_str(&format!("- {h}\n"));
        }
    }
    if let Some(cs) = &input.open_change_set {
        user.push_str(&format!(
            "\nOpen change set (a correction repeats the tool of the item it changes): {cs}\n"
        ));
    }
    if !input.done.is_empty() {
        user.push_str(&format!(
            "\nAlready done this turn (repeat a step only to change it): {}\n",
            Value::Array(input.done.clone())
        ));
    }
    if let Some(p) = &input.problem {
        user.push_str(&format!("\nThe last plan needs a change: {p}\n"));
    }
    if let Some(d) = &input.draft {
        user.push_str(&format!(
            "\nA first draft of the plan, to check and improve: {d}\n"
        ));
    }
    let tools: Vec<Value> = input.tools.iter().map(|t| describe_tool(t)).collect();
    user.push_str(&format!(
        "\nTools you may use:\n{}\n\nBlocks you may answer with: {}, or text.\n",
        Value::Array(tools),
        input.blocks.join(", ")
    ));
    vec![Message::system(system), Message::user(user)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::fixtures::manifest;

    fn blocks() -> Vec<String> {
        vec!["change_set".into(), "product_table".into()]
    }

    #[test]
    fn schema_closes_tools_and_blocks() {
        let m = manifest();
        let tools: Vec<&ToolSpec> = m.iter().take(3).collect();
        let s = plan_schema(&tools, &blocks(), 6);
        assert_eq!(
            s["properties"]["steps"]["items"]["properties"]["tool"]["enum"],
            json!(["get_policy", "search_products", "list_tiers"])
        );
        assert_eq!(
            s["properties"]["reply_plan"]["enum"],
            json!(["change_set", "product_table", "text"])
        );
        assert_eq!(s["properties"]["steps"]["maxItems"], 6);
    }

    #[test]
    fn validation_refuses_unknown_tools_and_repairs_structure() {
        let m = manifest();
        let tools: Vec<&ToolSpec> = m.iter().collect();
        let e = validate(
            &json!({"steps": [{"id": "s1", "tool": "approve_change_set", "args": {}}]}),
            &tools,
            &blocks(),
        )
        .unwrap_err();
        assert!(e.contains("approve_change_set"));

        let plan = validate(
            &json!({
                "interpretation": [{"aspect": "who", "text": "VIP members"}, {"aspect": "mood", "text": "happy"},
                                   {"aspect": "when", "text": " "}],
                "about": "self",
                "steps": [
                    {"id": "s1", "tool": "search_products", "args": "{\"category\": \"summer_outerwear\"}", "uses": ["s3"]},
                    {"id": "s1", "tool": "set_sale_prices", "args": {"percent_off": 20}, "uses": ["s1", "s9"]}
                ],
                "reply_plan": "gallery"
            }),
            &tools,
            &blocks(),
        )
        .unwrap();
        assert_eq!(plan.steps[0].uses, Vec::<String>::new());
        assert_eq!(plan.steps[0].args, json!({"category": "summer_outerwear"}));
        assert_eq!(plan.steps[1].id, "s2");
        assert_eq!(plan.steps[1].uses, ["s1"]);
        assert_eq!(plan.interpretation.len(), 2);
        assert_eq!(plan.interpretation[1].aspect, "other");
        assert_eq!(plan.reply_plan, None);
        assert_eq!(
            plan.domains(&tools).into_iter().collect::<Vec<_>>(),
            ["promotion", "search"]
        );
    }

    #[test]
    fn about_other_reads_and_defaults_to_self() {
        let p: Plan = serde_json::from_value(json!({"about": "other", "steps": []})).unwrap();
        assert_eq!(p.about, About::Other);
        let p: Plan = serde_json::from_value(json!({"steps": []})).unwrap();
        assert_eq!(p.about, About::Myself);
    }

    #[test]
    fn prompt_carries_clock_tools_and_open_change_set() {
        let m = manifest();
        let tools: Vec<&ToolSpec> = m.iter().take(2).collect();
        let b = blocks();
        let msgs = messages(&PlanInput {
            shared_prompt: "Plan.",
            instructions: "Promotions.",
            now: "Fri 2026-11-06 11:02".into(),
            currency: "USD",
            request: "Give the overshirt 15% instead.".into(),
            open_change_set: Some(json!({"id": "cs_1042", "version": 1})),
            tools,
            blocks: &b,
            ..Default::default()
        });
        assert_eq!(msgs.len(), 2);
        let user = &msgs[1].content;
        assert!(user.contains("Fri 2026-11-06 11:02"));
        assert!(user.contains("cs_1042"));
        assert!(user.contains("search_products"));
        assert!(msgs[0].content.contains("Promotions."));
    }
}
