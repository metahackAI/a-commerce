//! Candidate recall of the tool search on the eval task set, as a turn runs it.
//!
//! `cargo run -p acom-agent --example toolsearch_eval -- <manifest.json> <tasks dir>`
//!
//! `manifest.json` is `GET /v1/tools` for the task's operator. Each turn is searched on its own,
//! with the agent's `tool_scope.always` pinned as the runtime does. The agent is the one the
//! request's phrases route to, or else the one whose scope holds the most required tools (the
//! router's model is not called, so only the search is measured, and a task the model really
//! routes elsewhere is measured under the wrong agent). Prints one JSON line per task and a summary
//! line.

#![allow(clippy::expect_used)] // A measuring tool: a bad input ends the run.

use std::collections::BTreeSet;
use std::path::Path;

use acom_agent::profile::ChatSurface;
use acom_agent::tools::Manifest;
use acom_agent::{Profiles, router, toolsearch};
use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(manifest), Some(dir)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: toolsearch_eval <manifest.json> <tasks dir>");
        std::process::exit(2);
    };
    let manifest: Manifest =
        serde_json::from_str(&std::fs::read_to_string(manifest).expect("manifest")).expect("json");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let profiles = Profiles::load(&root.join("agents")).expect("agents/");
    let bo: Vec<_> = profiles
        .agents
        .iter()
        .filter(|a| a.surface == ChatSurface::Bo)
        .collect();
    let cfg = &profiles.runtime.tool_search;

    let mut files: Vec<_> = std::fs::read_dir(dir)
        .expect("tasks dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yml"))
        .collect();
    files.sort();
    let (mut wanted, mut got, mut tasks_all, mut tasks_seen) = (0, 0, 0, 0);
    for f in files {
        let tasks: Value =
            serde_norway::from_str(&std::fs::read_to_string(&f).expect("read")).expect("yaml");
        for t in tasks.as_array().into_iter().flatten() {
            let list = |p: &str| -> Vec<String> {
                t.pointer(p)
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let required = list("/tools/required");
            let built: Vec<&String> = required
                .iter()
                .filter(|r| manifest.items.iter().any(|m| &m.tool == *r))
                .collect();
            if built.is_empty() {
                continue;
            }
            let fallback = bo.iter().copied().max_by_key(|a| {
                built
                    .iter()
                    .filter(|r| manifest.items.iter().any(|m| &m.tool == **r && a.allows(m)))
                    .count()
            });
            let mut pooled: BTreeSet<String> = BTreeSet::new();
            let mut per_turn = Vec::new();
            let mut agent_ids = Vec::new();
            let mut first_turn: Vec<String> = Vec::new();
            for (i, turn) in list("/turns").iter().enumerate() {
                let agent = router::by_phrase(&bo, turn)
                    .and_then(|r| profiles.get(&r.agent))
                    .or(fallback)
                    .expect("an agent");
                let pinned: Vec<&str> =
                    agent.tool_scope.always.iter().map(String::as_str).collect();
                let found = toolsearch::search(&manifest.items, agent, turn, &pinned, cfg);
                let names: Vec<String> = found.iter().map(|c| c.tool.clone()).collect();
                pooled.extend(names.iter().cloned());
                if i == 0 {
                    first_turn = names.clone();
                }
                per_turn.push(names);
                agent_ids.push(agent.id.clone());
            }
            let found: Vec<_> = built.iter().filter(|r| pooled.contains(**r)).collect();
            let first: Vec<_> = built.iter().filter(|r| first_turn.contains(**r)).collect();
            wanted += built.len();
            got += found.len();
            tasks_all += 1;
            tasks_seen += usize::from(found.len() == built.len());
            println!(
                "{}",
                json!({
                    "id": t.get("id"),
                    "agents": agent_ids,
                    "built": built,
                    "found": found,
                    "first_turn_found": first,
                    "per_turn": per_turn,
                })
            );
        }
    }
    println!(
        "{}",
        json!({"summary": {"required": wanted, "found": got, "tasks": tasks_all, "tasks_with_all": tasks_seen}})
    );
}
