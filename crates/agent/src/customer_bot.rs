//! Ailism Customer Bot's shopper (docs/api/platform.md#customer_bot): the model that plays one
//! member of a demo store. The gateway's workers do everything a browser does; they ask here only
//! for what a person decides: who a new shopper is, the persona a member shops as, the next thing
//! to do on a visit, and the review of an item received. No tools are called here.

use std::time::Instant;

use acom_contract::{ErrorCode, ToolError};
use acom_llm::{ChatRequest, Message};
use serde_json::{Value, json};

use crate::runtime::Runtime;

/// The longest answer a task may write.
const ANSWER_TOKENS: u32 = 900;

/// The tasks and the shape of each answer.
fn schema(task: &str) -> Option<Value> {
    let text = |most: u32| json!({"type": "string", "maxLength": most});
    let list = |most: u32| json!({"type": "array", "items": {"type": "string", "maxLength": 60}, "maxItems": most});
    Some(match task {
        "identity" => json!({
            "type": "object", "additionalProperties": false,
            "required": ["name", "email_local", "street_address", "address_locality",
                         "address_region", "postal_code", "telephone"],
            "properties": {"name": text(80), "email_local": text(40), "street_address": text(120),
                           "address_locality": text(60), "address_region": text(2),
                           "postal_code": text(10), "telephone": text(20)}}),
        "persona" => json!({
            "type": "object", "additionalProperties": false,
            "required": ["description", "age_range", "location", "occupation", "style", "colors",
                         "sizes", "budget_per_order", "categories", "shopping_style",
                         "chattiness", "voice", "interests"],
            "properties": {
                "description": text(200), "age_range": text(20), "location": text(60),
                "occupation": text(60), "style": list(5), "colors": list(5),
                "sizes": {"type": "object", "additionalProperties": false,
                          "required": ["top", "bottom", "shoes"],
                          "properties": {"top": text(10), "bottom": text(10), "shoes": text(10)}},
                "budget_per_order": {"type": "integer", "minimum": 20, "maximum": 1000},
                "categories": list(6),
                "shopping_style": {"type": "string", "enum": ["decisive", "browser", "bargain_hunter", "researcher", "gift_giver"]},
                "chattiness": {"type": "string", "enum": ["brief", "normal", "chatty"]},
                "voice": text(160), "interests": list(5)}}),
        "start" => json!({
            "type": "object", "additionalProperties": false,
            "required": ["goal", "message", "reason"],
            "properties": {"goal": text(200), "message": text(300), "reason": text(160)}}),
        "next" => json!({
            "type": "object", "additionalProperties": false,
            "required": ["action", "message", "reason", "goal"],
            "properties": {
                "action": {"type": "string", "enum": ["say", "place_order", "leave"]},
                "message": text(300), "reason": text(160), "goal": text(200)}}),
        "review" => json!({
            "type": "object", "additionalProperties": false,
            "required": ["rating", "body", "fit"],
            "properties": {
                "rating": {"type": "integer", "minimum": 1, "maximum": 5}, "body": text(600),
                "fit": {"type": "string", "enum": ["true_to_size", "runs_small", "runs_large", "none"]}}}),
        _ => return None,
    })
}

fn invalid(message: impl Into<String>) -> ToolError {
    ToolError::new(ErrorCode::INVALID_INPUT, message.into())
        .detail(json!({"field": "task", "reason": "invalid"}))
}

/// One decision: `{task, model, input}` → `{output, usage}`. The answer is the model's JSON in
/// the task's shape; a call that fails or does not read is tried once more.
pub async fn think(rt: &Runtime, body: &Value) -> Result<Value, ToolError> {
    let task = body.get("task").and_then(Value::as_str).unwrap_or_default();
    let shape = schema(task).ok_or_else(|| invalid(format!("{task:?} is not a task.")))?;
    let alias = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("glimmer");
    let model = rt.models().get(alias).ok_or_else(|| {
        ToolError::new(
            ErrorCode::UPSTREAM_UNAVAILABLE,
            format!("The model {alias} is not configured."),
        )
    })?;
    let input = body.get("input").cloned().unwrap_or(Value::Null);
    let req = ChatRequest::new(
        "customer_bot",
        vec![
            Message::system(rt.profiles().prompts.customer_bot.trim()),
            Message::user(json!({"task": task, "input": input}).to_string()),
        ],
    )
    .json(&format!("customer_bot_{task}"), shape)
    .max_tokens(ANSWER_TOKENS);
    let started = Instant::now();
    let (mut input_tokens, mut output_tokens, mut calls) = (0u32, 0u32, 0u32);
    let output = loop {
        calls += 1;
        match model.chat(&req, None).await {
            Ok(r) => {
                input_tokens += r.usage.input_tokens;
                output_tokens += r.usage.output_tokens;
                match r.json() {
                    Ok(v) if v.is_object() => break Ok(v),
                    _ if calls < 2 => {}
                    _ => break Err("The answer was not readable.".to_owned()),
                }
            }
            Err(e) if calls < 2 => {
                tracing::warn!(error = %e, task, "customer bot: model call failed; trying once more");
            }
            Err(e) => break Err(format!("The model did not answer: {e}")),
        }
    };
    let usage = json!({"model": alias, "model_calls": calls, "input_tokens": input_tokens,
                       "output_tokens": output_tokens,
                       "ms": u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)});
    match output {
        Ok(v) => Ok(json!({"output": v, "usage": usage})),
        Err(why) => {
            Err(ToolError::new(ErrorCode::UPSTREAM_UNAVAILABLE, why)
                .detail(json!({"usage": usage})))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_task_has_a_closed_shape_and_others_are_refused() {
        for t in ["identity", "persona", "start", "next", "review"] {
            let s = schema(t).unwrap();
            assert_eq!(s["additionalProperties"], false, "{t}");
            assert!(
                s["required"].as_array().is_some_and(|r| !r.is_empty()),
                "{t}"
            );
        }
        assert!(schema("approve").is_none());
    }
}
