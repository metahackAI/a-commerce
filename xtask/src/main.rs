//! Repository tasks. `cargo xtask <task>`.
//!
//! - `check-docs`: the operation index in `docs/api/README.md` and the operation sections in
//!   the domain files must list the same tools with the same kinds. Later this also compares
//!   both against the registry (`docs/backend/testing.md` §1).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    match task.as_str() {
        "check-docs" => check_docs(&repo_root()),
        _ => {
            eprintln!("usage: cargo xtask check-docs");
            ExitCode::FAILURE
        }
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    domain: String,
    kind: String,
}

fn check_docs(root: &Path) -> ExitCode {
    let api = root.join("docs/api");
    let readme = match fs::read_to_string(api.join("README.md")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read docs/api/README.md: {e}");
            return ExitCode::FAILURE;
        }
    };
    let index = parse_index(&readme);

    let mut sections: BTreeMap<String, Entry> = BTreeMap::new();
    let domains: Vec<String> = {
        let mut d: Vec<String> = index.values().map(|e| e.domain.clone()).collect();
        d.sort();
        d.dedup();
        d
    };
    let mut problems = Vec::new();
    for domain in &domains {
        match fs::read_to_string(api.join(format!("{domain}.md"))) {
            Ok(text) => {
                for (tool, kind) in parse_sections(&text) {
                    if let Some(prev) = sections.insert(
                        tool.clone(),
                        Entry {
                            domain: domain.clone(),
                            kind,
                        },
                    ) {
                        problems.push(format!(
                            "`{tool}` has sections in both {}.md and {domain}.md",
                            prev.domain
                        ));
                    }
                }
            }
            Err(e) => problems.push(format!("cannot read docs/api/{domain}.md: {e}")),
        }
    }

    for (tool, e) in &index {
        match sections.get(tool) {
            None => problems.push(format!(
                "`{tool}` is in the index ({}) but has no section in {}.md",
                e.kind, e.domain
            )),
            Some(s) if s.domain != e.domain => problems.push(format!(
                "`{tool}` is indexed under {} but specified in {}.md",
                e.domain, s.domain
            )),
            Some(s) if s.kind != e.kind => problems.push(format!(
                "`{tool}` is {} in the index but {} in {}.md",
                e.kind, s.kind, s.domain
            )),
            Some(_) => {}
        }
    }
    for (tool, s) in &sections {
        if !index.contains_key(tool) {
            problems.push(format!(
                "`{tool}` has a section in {}.md but is not in the index",
                s.domain
            ));
        }
    }

    println!(
        "index: {} operations in {} domains; sections: {}",
        index.len(),
        domains.len(),
        sections.len()
    );
    if problems.is_empty() {
        println!("docs are consistent");
        ExitCode::SUCCESS
    } else {
        for p in &problems {
            println!("- {p}");
        }
        println!("{} problem(s)", problems.len());
        ExitCode::FAILURE
    }
}

/// Reads `## 4.` of the API README: `**[domain](domain.md)**` headers, then table rows whose
/// cells list `` `tool` KIND `` (bold marks MVP). Person-only endpoints are skipped.
fn parse_index(readme: &str) -> BTreeMap<String, Entry> {
    let mut out = BTreeMap::new();
    let mut in_index = false;
    let mut domain = String::new();
    for line in readme.lines() {
        if line.starts_with("## ") {
            in_index = line.starts_with("## 4.");
            continue;
        }
        if !in_index {
            continue;
        }
        if let Some(rest) = line.strip_prefix("**[") {
            domain = rest.split(']').next().unwrap_or_default().to_owned();
            continue;
        }
        if !line.starts_with('|') || domain.is_empty() {
            continue;
        }
        let Some(ops) = line.split('|').nth(2) else {
            continue;
        };
        for (tool, after) in backticked(ops) {
            if !is_tool_name(&tool) {
                continue;
            }
            if let Some(kind) = leading_kind(after.trim_start_matches('*').trim_start()) {
                out.insert(
                    tool,
                    Entry {
                        domain: domain.clone(),
                        kind,
                    },
                );
            }
        }
    }
    out
}

/// `### `tool` — Kind: W$  (MVP)` → (tool, "W$").
fn parse_sections(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("### `")?;
            let (tool, tail) = rest.split_once('`')?;
            let kind = leading_kind(tail.split_once("Kind:")?.1.trim_start())?;
            is_tool_name(tool).then(|| (tool.to_owned(), kind))
        })
        .collect()
}

/// Every `` `x` `` in `s` with the text that follows it.
fn backticked(s: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('`') {
        let after_open = &rest[start + 1..];
        let Some(end) = after_open.find('`') else {
            break;
        };
        out.push((after_open[..end].to_owned(), &after_open[end + 1..]));
        rest = &after_open[end + 1..];
    }
    out
}

fn leading_kind(s: &str) -> Option<String> {
    if s.starts_with("W$") {
        Some("W$".into())
    } else if s.starts_with('W') {
        Some("W".into())
    } else if s.starts_with('R') {
        Some("R".into())
    } else {
        None
    }
}

fn is_tool_name(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_index_rows_and_sections() {
        let readme = "## 4. 인덱스\n**[sales](sales.md)**\n| resource | operations |\n|---|---|\n\
            | order | **`prepare_order`** W(프리뷰만) · `get_order` R · 사람 전용 `POST /orders` |\n## 5. x\n";
        let idx = parse_index(readme);
        assert_eq!(idx.len(), 2);
        assert_eq!(idx["prepare_order"].kind, "W");
        assert_eq!(idx["get_order"].domain, "sales");

        let secs = parse_sections("### `refund_return` — Kind: W$  (MVP)\n### Other\n");
        assert_eq!(secs, vec![("refund_return".to_owned(), "W$".to_owned())]);
    }
}
