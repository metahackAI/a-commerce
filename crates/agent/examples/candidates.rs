//! The tool search's candidates for requests, as a turn would get them: no model, no network.
//!
//! `cargo run -p acom-agent --example candidates -- <manifest.json> <agent> <request>...`
//!
//! `manifest.json` is `GET /v1/tools` for the person asking; the agent's `tool_scope.always` is
//! pinned as the runtime pins it. Prints one line per request: the candidates, best first.

#![allow(clippy::expect_used)] // A measuring tool: a bad input ends the run.

use std::path::Path;

use acom_agent::Profiles;
use acom_agent::tools::Manifest;
use acom_agent::toolsearch;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(manifest), Some(agent)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: candidates <manifest.json> <agent> <request>...");
        std::process::exit(2);
    };
    let manifest: Manifest =
        serde_json::from_str(&std::fs::read_to_string(manifest).expect("manifest")).expect("json");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let profiles = Profiles::load(&root.join("agents")).expect("agents/");
    let agent = profiles.get(agent).expect("agent");
    let pinned: Vec<&str> = agent.tool_scope.always.iter().map(String::as_str).collect();
    for request in &args[3..] {
        let found = toolsearch::search(
            &manifest.items,
            agent,
            request,
            &pinned,
            &profiles.runtime.tool_search,
        );
        let names: Vec<&str> = found.iter().map(|c| c.tool.as_str()).collect();
        println!("{request} → {}", names.join(", "));
    }
}
