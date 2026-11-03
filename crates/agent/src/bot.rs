//! Ailism Bot's analysis (docs/api/platform.md#bot, steps 2–3). The gateway hands a cycle here
//! with the on-behalf token of the operator the bot works for. The bot reads the store with read
//! tools only, in a fixed order per area it looks at, finds warning signs by rule, and asks the
//! analysing model for a report: findings and proposals. What it returns is the body commerce
//! records as the cycle's end (`POST /v1/platform/bot/cycles/{id}/report`); commerce filters the
//! proposals and opens the tasks. Nothing here writes the store.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::{Value, json};

use crate::checker;
use crate::client::ToolClient;
use crate::runtime::{Runtime, compact};
use acom_llm::{ChatRequest, Message};

/// Characters one read keeps in the stored snapshot, and in the model's prompt.
const SNAPSHOT_CHARS: usize = 4000;
const PROMPT_CHARS: usize = 1400;
/// Boards whose reported posts one cycle reads.
const BOARDS: usize = 4;
/// The longest report the model may write.
const REPORT_TOKENS: u32 = 1600;

/// Areas in the order the bot looks at them (`platform.yml bot.areas`).
pub const AREAS: &[&str] = &[
    "pricing",
    "inventory",
    "promotions",
    "merchandising",
    "catalog_content",
    "returns",
    "questions_reviews",
    "community",
];

/// One read of the snapshot.
#[derive(Debug, Clone)]
struct Read {
    area: &'static str,
    tool: &'static str,
    args: Value,
    result: Result<Value, String>,
}

fn day(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// The reads of one area, given the store's date today.
fn reads_for(area: &str, today: NaiveDate) -> Vec<(&'static str, Value)> {
    let d = |n: i64| day(today + Duration::days(n));
    match area {
        "pricing" => vec![
            ("search_products", json!({"sort": "price", "limit": 30})),
            ("get_policy", json!({"key": "price_floor"})),
            (
                "list_active_promotions",
                json!({"valid_from": d(0), "valid_through": d(0), "limit": 20}),
            ),
        ],
        "inventory" => vec![(
            "search_products",
            json!({"sort": "inventory_level", "limit": 15}),
        )],
        "promotions" => vec![
            (
                "list_active_promotions",
                json!({"valid_from": d(0), "valid_through": d(7), "limit": 20}),
            ),
            (
                "search_products",
                json!({"sort": "-inventory_level", "limit": 10}),
            ),
            (
                "explain_sales_change",
                json!({"valid_from": d(-1), "valid_through": d(-1),
                                            "compare_mode": "same_weekday_last_4_weeks"}),
            ),
        ],
        "merchandising" => vec![
            (
                "list_display_slots",
                json!({"page": "main", "valid_from": d(0), "valid_through": d(7)}),
            ),
            ("list_curations", json!({"status": "active", "limit": 10})),
        ],
        "catalog_content" => vec![("search_products", json!({"sort": "newest", "limit": 20}))],
        "returns" => vec![(
            "search_return_requests",
            json!({"status": ["requested"], "sort": "-amount", "limit": 15}),
        )],
        "questions_reviews" => vec![
            (
                "list_questions",
                json!({"answered": false, "status": "visible", "limit": 15}),
            ),
            ("list_reviews", json!({"sort": "rating", "limit": 10})),
        ],
        "community" => vec![("list_boards", json!({"limit": 10}))],
        _ => Vec::new(),
    }
}

fn items(v: &Value) -> Vec<Value> {
    v.get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn name_of(i: &Value) -> String {
    ["name", "title", "headline", "text", "id"]
        .iter()
        .find_map(|k| i.get(*k).and_then(Value::as_str))
        .unwrap_or("?")
        .chars()
        .take(60)
        .collect()
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key)
        .and_then(|x| x.as_f64().or_else(|| x.as_str()?.parse().ok()))
}

/// Warning signs found by rule, never by the model: each says what and how many.
fn anomalies(reads: &[Read]) -> Vec<Value> {
    // Boards by name, for the reported posts on each.
    let boards: Vec<(String, String)> = reads
        .iter()
        .filter(|r| r.tool == "list_boards")
        .filter_map(|r| r.result.as_ref().ok())
        .flat_map(items)
        .filter_map(|b| Some((b.get("id")?.as_str()?.to_owned(), name_of(&b))))
        .collect();
    let mut out = Vec::new();
    let mut add = |area: &str, code: &str, text: String, severity: &str| {
        out.push(json!({"area": area, "code": code, "text": text, "severity": severity}));
    };
    for r in reads {
        let Ok(v) = &r.result else {
            continue;
        };
        let list = items(v);
        match (r.area, r.tool) {
            ("pricing", "search_products") => {
                let under: Vec<String> = list
                    .iter()
                    .filter(|i| matches!((num(i, "price"), num(i, "cost")), (Some(p), Some(c)) if p < c))
                    .map(name_of)
                    .collect();
                if !under.is_empty() {
                    add(
                        "pricing",
                        "price_below_cost",
                        format!("{} priced under cost: {}", under.len(), under.join(", ")),
                        "high",
                    );
                }
            }
            ("inventory", "search_products") => {
                let out_of: Vec<String> = list
                    .iter()
                    .filter(|i| {
                        i.get("has_variant")
                            .and_then(Value::as_array)
                            .is_some_and(|vs| {
                                vs.iter().any(|v| {
                                    v.get("availability").and_then(Value::as_str)
                                        == Some("out_of_stock")
                                })
                            })
                    })
                    .map(name_of)
                    .collect();
                if !out_of.is_empty() {
                    add(
                        "inventory",
                        "sizes_out_of_stock",
                        format!(
                            "{} products have sizes out of stock: {}",
                            out_of.len(),
                            out_of.join(", ")
                        ),
                        "medium",
                    );
                }
            }
            ("promotions", "explain_sales_change") => {
                let rate = v.pointer("/summary/change_rate").and_then(Value::as_f64);
                if let Some(rate) = rate.filter(|r| *r < 0.0) {
                    add(
                        "promotions",
                        "sales_down",
                        format!(
                            "Sales yesterday were {:.0}% under the usual day",
                            -rate * 100.0
                        ),
                        "medium",
                    );
                }
            }
            ("catalog_content", "search_products") => {
                let bare: Vec<String> = list
                    .iter()
                    .filter(|i| i.get("image").is_none_or(Value::is_null))
                    .map(name_of)
                    .collect();
                if !bare.is_empty() {
                    add(
                        "catalog_content",
                        "no_photo",
                        format!(
                            "{} new products have no photo: {}",
                            bare.len(),
                            bare.join(", ")
                        ),
                        "low",
                    );
                }
            }
            ("returns", "search_return_requests") => {
                let n = v
                    .get("total")
                    .and_then(Value::as_u64)
                    .unwrap_or(list.len() as u64);
                if n > 0 {
                    add(
                        "returns",
                        "returns_waiting",
                        format!("{n} return requests wait for a decision"),
                        "medium",
                    );
                }
            }
            ("questions_reviews", "list_questions") => {
                let n = v
                    .get("total")
                    .and_then(Value::as_u64)
                    .unwrap_or(list.len() as u64);
                if n > 0 {
                    add(
                        "questions_reviews",
                        "questions_unanswered",
                        format!("{n} product questions wait for an answer"),
                        "medium",
                    );
                }
            }
            ("community", "list_posts") if !list.is_empty() => {
                let id = r.args.get("board_id").and_then(Value::as_str).unwrap_or("");
                let board = boards
                    .iter()
                    .find(|(b, _)| b == id)
                    .map_or(id, |(_, name)| name.as_str());
                let n = list.len();
                add(
                    "community",
                    "reported_posts",
                    format!(
                        "{n} reported post{} on {board}",
                        if n == 1 { "" } else { "s" }
                    ),
                    "medium",
                );
            }
            _ => {}
        }
    }
    out
}

/// The report the model writes, guided by schema.
fn report_schema(areas: &[&str], most: u32) -> Value {
    let area = json!({"type": "string", "enum": areas});
    let level = json!({"type": "string", "enum": ["low", "medium", "high"]});
    json!({"type": "object", "additionalProperties": false,
      "required": ["summary", "findings", "proposals"],
      "properties": {
        "summary": {"type": "string", "maxLength": 700},
        "findings": {"type": "array", "maxItems": 8, "items": {"type": "object",
          "additionalProperties": false, "required": ["area", "text", "severity"],
          "properties": {"area": area, "text": {"type": "string", "maxLength": 300}, "severity": level}}},
        "proposals": {"type": "array", "maxItems": most, "items": {"type": "object",
          "additionalProperties": false,
          "required": ["area", "title", "request", "reason", "evidence", "expected_effect", "risk", "about"],
          "properties": {
            "area": area,
            "title": {"type": "string", "maxLength": 120},
            "request": {"type": "string", "maxLength": 400},
            "reason": {"type": "string", "maxLength": 400},
            "evidence": {"type": "array", "maxItems": 4, "items": {"type": "string", "maxLength": 160}},
            "expected_effect": {"type": "object", "additionalProperties": false,
              "required": ["metric", "direction", "horizon_days"],
              "properties": {"metric": {"type": "string", "maxLength": 40},
                             "direction": {"type": "string", "enum": ["up", "down"]},
                             "horizon_days": {"type": "integer", "minimum": 1, "maximum": 60}}},
            "risk": level,
            "about": {"type": "array", "maxItems": 6, "items": {"type": "object",
              "additionalProperties": false, "required": ["entity", "id"],
              "properties": {"entity": {"type": "string", "maxLength": 40},
                             "id": {"type": "string", "maxLength": 80}}}}}}}}})
}

/// Evidence lines with a number the snapshot does not hold are dropped: the bot does not
/// invent figures. A summary the length limit stopped keeps its finished sentences.
fn grounded(report: &mut Value, sources: &[&Value]) {
    if let Some(summary) = report.get("summary").and_then(Value::as_str)
        && !summary.trim_end().ends_with(['.', '!', '?'])
        && let Some(kept) = checker::whole_sentences(summary)
    {
        report["summary"] = json!(kept);
    }
    let Some(proposals) = report.get_mut("proposals").and_then(Value::as_array_mut) else {
        return;
    };
    for p in proposals {
        if let Some(ev) = p.get_mut("evidence").and_then(Value::as_array_mut) {
            ev.retain(|e| {
                e.as_str()
                    .is_some_and(|t| checker::unsupported_numbers(t, sources, &[]).is_empty())
            });
        }
    }
}

fn failed(why: String, snapshot: Value, anomalies: Value, usage: Value) -> Value {
    json!({"state": "failed", "error": why, "snapshot": snapshot, "anomalies": anomalies, "usage": usage})
}

/// Runs the analysis of one cycle and returns the body of its report.
pub async fn analyze(rt: &Runtime, tools: &dyn ToolClient) -> Value {
    let store = match tools.store().await {
        Ok(s) => s,
        Err(e) => {
            return failed(
                format!("The store could not be read: {}", e.message),
                json!({}),
                json!([]),
                json!({}),
            );
        }
    };
    let now: DateTime<Utc> = store.now;
    let today = now.with_timezone(&store.time.zone).date_naive();
    let status = match tools.read("get_bot_status", &json!({})).await {
        Ok(s) => s,
        Err(e) => {
            return failed(
                format!("The bot's settings could not be read: {}", e.message),
                json!({}),
                json!([]),
                json!({}),
            );
        }
    };
    let settings = &status["settings"];
    let areas: Vec<&str> = AREAS
        .iter()
        .copied()
        .filter(|a| settings["areas"][*a].as_bool().unwrap_or(false))
        .collect();
    let most = settings["max_proposals_per_cycle"]
        .as_u64()
        .unwrap_or(3)
        .clamp(1, 10) as u32;
    let model_alias = settings["model"].as_str().unwrap_or("glimmer").to_owned();

    // Observe: every area's reads, in order; the boards' reported posts after the boards.
    let mut reads: Vec<Read> = Vec::new();
    for area in &areas {
        let area: &'static str = AREAS
            .iter()
            .find(|a| *a == area)
            .copied()
            .unwrap_or("pricing");
        for (tool, args) in reads_for(area, today) {
            let result = tools.read(tool, &args).await.map_err(|e| e.message);
            let boards: Vec<String> = match (&result, tool) {
                (Ok(v), "list_boards") => items(v)
                    .iter()
                    .filter_map(|b| b.get("id").and_then(Value::as_str).map(str::to_owned))
                    .take(BOARDS)
                    .collect(),
                _ => Vec::new(),
            };
            reads.push(Read {
                area,
                tool,
                args,
                result,
            });
            for board in boards {
                let args = json!({"board_id": board, "reported": true, "limit": 10});
                let result = tools.read("list_posts", &args).await.map_err(|e| e.message);
                reads.push(Read {
                    area,
                    tool: "list_posts",
                    args,
                    result,
                });
            }
        }
    }
    let open_tasks = tools
        .read(
            "list_tasks",
            &json!({"state": ["needs_you", "working"], "limit": 20}),
        )
        .await
        .unwrap_or(Value::Null);
    let feedback = tools
        .read("list_bot_feedback", &json!({"state": "open", "limit": 20}))
        .await
        .unwrap_or(Value::Null);
    let recent = tools
        .read("list_bot_decisions", &json!({"limit": 15}))
        .await
        .unwrap_or(Value::Null);

    let signs = json!(anomalies(&reads));
    let snapshot_reads: Vec<Value> = reads
        .iter()
        .map(|r| match &r.result {
            Ok(v) => json!({"area": r.area, "tool": r.tool, "args": r.args, "result": compact(v, SNAPSHOT_CHARS)}),
            Err(e) => json!({"area": r.area, "tool": r.tool, "args": r.args, "error": e}),
        })
        .collect();
    let snapshot = json!({"at": store.time.format(now), "areas": areas, "reads": snapshot_reads,
                          "open_tasks": compact(&open_tasks, SNAPSHOT_CHARS)});
    let usage = |calls: u32, input: u32, output: u32| {
        json!({"model": model_alias, "model_calls": calls, "input_tokens": input,
               "output_tokens": output, "reads": reads.len()})
    };
    if areas.is_empty() {
        return json!({"state": "done", "snapshot": snapshot, "anomalies": signs, "usage": usage(0, 0, 0),
            "report": {"summary": "Every area is off, so there was nothing to look at.", "findings": [], "proposals": []}});
    }

    // Analyse.
    let Some(model) = rt.models().get(&model_alias) else {
        return failed(
            format!("The model {model_alias} is not configured."),
            snapshot,
            signs,
            usage(0, 0, 0),
        );
    };
    let directives: Vec<Value> = settings["directives"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let shown_feedback: Vec<Value> = items(&feedback)
        .iter()
        .map(|f| json!({"kind": f["kind"], "text": f["text"], "decision_id": f["decision_id"]}))
        .collect();
    let shown_recent: Vec<Value> = items(&recent)
        .iter()
        .map(|d| {
            json!({"id": d["id"], "area": d["area"], "title": d["title"], "state": d["state"],
                        "about": d["about"], "at": d["at"]})
        })
        .collect();
    let shown_tasks: Vec<Value> = items(&open_tasks)
        .iter()
        .map(|t| json!({"title": t["title"], "state": t["state"], "tag": t["tag"]}))
        .collect();
    let prompt_reads: Vec<Value> = reads
        .iter()
        .map(|r| match &r.result {
            Ok(v) => json!({"area": r.area, "tool": r.tool, "args": r.args, "result": compact(v, PROMPT_CHARS)}),
            Err(e) => json!({"area": r.area, "tool": r.tool, "error": e}),
        })
        .collect();
    let input = json!({
        "now": store.time.format(now), "currency": store.currency, "areas": areas,
        "most_proposals": most, "directives": directives, "open_feedback": shown_feedback,
        "your_recent_decisions": shown_recent, "open_tasks": shown_tasks,
        "warning_signs": signs, "snapshot": prompt_reads});
    let req = ChatRequest::new(
        "bot_analysis",
        vec![
            Message::system(rt.profiles().prompts.bot.trim()),
            Message::user(input.to_string()),
        ],
    )
    .json("bot_report", report_schema(&areas, most))
    .max_tokens(REPORT_TOKENS);
    let mut tries = 0u32;
    let (mut input_tokens, mut output_tokens) = (0u32, 0u32);
    let report = loop {
        tries += 1;
        match model.chat(&req, None).await {
            Ok(r) => {
                input_tokens += r.usage.input_tokens;
                output_tokens += r.usage.output_tokens;
                match r.json() {
                    Ok(v) if v.get("proposals").is_some_and(Value::is_array) => break Ok(v),
                    Ok(_) | Err(_) if tries < 2 => continue,
                    Ok(_) => break Err("The report had no proposals list.".to_owned()),
                    Err(e) => break Err(format!("The report was not readable: {e}")),
                }
            }
            Err(e) if tries < 2 => {
                tracing::warn!(error = %e, "bot analysis: model call failed; trying once more");
            }
            Err(e) => break Err(format!("The model did not answer: {e}")),
        }
    };
    let usage = usage(tries, input_tokens, output_tokens);
    match report {
        Ok(mut report) => {
            let sources: Vec<&Value> = reads
                .iter()
                .filter_map(|r| r.result.as_ref().ok())
                .collect();
            grounded(&mut report, &sources);
            json!({"state": "done", "snapshot": snapshot, "anomalies": signs, "report": report, "usage": usage})
        }
        Err(why) => failed(why, snapshot, signs, usage),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(area: &'static str, tool: &'static str, result: Value) -> Read {
        Read {
            area,
            tool,
            args: json!({"board_id": "brd_style"}),
            result: Ok(result),
        }
    }

    #[test]
    fn warning_signs_come_from_rules_with_their_counts() {
        let reads = vec![
            read(
                "pricing",
                "search_products",
                json!({"items": [
                {"name": "Breton tee", "price": 19.0, "cost": 20.0},
                {"name": "Wool overcoat", "price": 289.0, "cost": 120.0}]}),
            ),
            read(
                "inventory",
                "search_products",
                json!({"items": [
                {"name": "Light rain parka", "has_variant": [{"availability": "out_of_stock"}]}]}),
            ),
            read(
                "returns",
                "search_return_requests",
                json!({"items": [{}], "total": 12}),
            ),
            read(
                "community",
                "list_boards",
                json!({"items": [{"id": "brd_style", "name": "Style talk"}]}),
            ),
            read(
                "community",
                "list_posts",
                json!({"items": [{"id": "pst_1"}]}),
            ),
            read("community", "list_posts", json!({"items": []})),
        ];
        let a = anomalies(&reads);
        let codes: Vec<&str> = a.iter().map(|x| x["code"].as_str().unwrap()).collect();
        assert_eq!(
            codes,
            [
                "price_below_cost",
                "sizes_out_of_stock",
                "returns_waiting",
                "reported_posts"
            ]
        );
        assert_eq!(a[0]["text"], "1 priced under cost: Breton tee");
        assert_eq!(a[2]["text"], "12 return requests wait for a decision");
        assert_eq!(a[3]["text"], "1 reported post on Style talk");
    }

    #[test]
    fn evidence_without_a_figure_from_the_store_is_dropped() {
        let snap = json!({"items": [{"name": "Linen shirt", "inventory_level": 38}]});
        let mut report =
            json!({"proposals": [{"evidence": ["38 left", "sells 99 a day", "no number here"]}]});
        grounded(&mut report, &[&snap]);
        assert_eq!(
            report["proposals"][0]["evidence"],
            json!(["38 left", "no number here"])
        );
    }

    #[test]
    fn every_area_reads_something_and_the_schema_names_only_the_areas_on() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        for a in AREAS {
            assert!(!reads_for(a, today).is_empty(), "{a}");
        }
        let s = report_schema(&["pricing", "community"], 2);
        assert_eq!(s["properties"]["proposals"]["maxItems"], 2);
        assert_eq!(
            s["properties"]["proposals"]["items"]["properties"]["area"]["enum"],
            json!(["pricing", "community"])
        );
    }
}
