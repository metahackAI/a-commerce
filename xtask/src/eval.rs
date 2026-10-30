//! `cargo xtask check-eval`: the agent task set (`eval/tasks/{design,held_out}/*.yml`) is well
//! formed, and every tool a task names exists.
//!
//! A tool exists when the registry has it (built) or `docs/api/` specifies it (spec only). A task's
//! coverage is derived, not written by hand: `ready` when every required tool is built, `spec`
//! when some are only specified, `data` when the task records a data gap. A name in neither place
//! is a typo or an invented tool and fails the check.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use acom_contract::Audience;
use serde_json::Value;

const SETS: [&str; 2] = ["design", "held_out"];
const JOBS: [&str; 4] = ["merchandiser", "marketer", "operations", "owner"];
const KINDS: [&str; 3] = ["read", "write", "write_approval"];
const PHRASING: [&str; 3] = ["verbatim", "adapted", "ours"];

#[derive(Debug, Default)]
pub struct EvalReport {
    pub rows: Vec<Row>,
    pub problems: Vec<String>,
}

#[derive(Debug)]
pub struct Row {
    pub set: &'static str,
    pub id: String,
    pub job: String,
    pub kind: String,
    pub phrasing: String,
    pub coverage: &'static str,
    pub unbuilt: Vec<String>,
}

pub struct Tools {
    /// Built tools and who may call them.
    pub built: BTreeMap<String, Audience>,
    /// Tools specified in `docs/api/`.
    pub specified: BTreeSet<String>,
}

pub fn check(root: &Path, tools: &Tools) -> std::io::Result<EvalReport> {
    let sources = source_ids(&fs::read_to_string(root.join("eval/README.md"))?);
    let mut report = EvalReport::default();
    let mut seen: BTreeMap<String, &'static str> = BTreeMap::new();
    for set in SETS {
        let dir = root.join("eval/tasks").join(set);
        let mut files: Vec<_> = fs::read_dir(&dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "yml"))
            .collect();
        files.sort();
        for path in files {
            let file = format!(
                "{set}/{}",
                path.file_name().unwrap_or_default().to_string_lossy()
            );
            let doc: Value = match serde_norway::from_str(&fs::read_to_string(&path)?) {
                Ok(v) => v,
                Err(e) => {
                    report.problems.push(format!("{file}: not valid YAML: {e}"));
                    continue;
                }
            };
            let Some(tasks) = doc.as_array() else {
                report
                    .problems
                    .push(format!("{file}: expected a list of tasks"));
                continue;
            };
            for task in tasks {
                let mut p = Vec::new();
                let row = check_task(set, task, tools, &sources, &mut p);
                if let Some(prev) = seen.insert(row.id.clone(), set) {
                    p.push(format!("id also used in {prev}"));
                }
                report
                    .problems
                    .extend(p.into_iter().map(|m| format!("{file} {}: {m}", row.id)));
                report.rows.push(row);
            }
        }
    }
    Ok(report)
}

fn check_task(
    set: &'static str,
    t: &Value,
    tools: &Tools,
    sources: &BTreeSet<String>,
    p: &mut Vec<String>,
) -> Row {
    let s = |k: &str| {
        t.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let id = s("id");
    if id.is_empty() {
        p.push("missing id".into());
    }
    let job = s("job");
    if !JOBS.contains(&job.as_str()) {
        p.push(format!("job `{job}` is not one of {JOBS:?}"));
    }
    let kind = s("kind");
    if !KINDS.contains(&kind.as_str()) {
        p.push(format!("kind `{kind}` is not one of {KINDS:?}"));
    }
    for k in ["persona", "surface", "frequency"] {
        if s(k).is_empty() {
            p.push(format!("missing {k}"));
        }
    }

    let phrasing = t
        .pointer("/phrasing/how")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if !PHRASING.contains(&phrasing.as_str()) {
        p.push(format!(
            "phrasing.how `{phrasing}` is not one of {PHRASING:?}"
        ));
    }
    let mut refs = names(t.get("sources"));
    if phrasing != "ours" {
        match t.pointer("/phrasing/from").and_then(Value::as_str) {
            Some(r) => refs.push(r.to_owned()),
            None => p.push(format!("phrasing `{phrasing}` needs `from`")),
        }
        if t.pointer("/phrasing/original")
            .and_then(Value::as_str)
            .is_none()
        {
            p.push(format!("phrasing `{phrasing}` needs `original`"));
        }
    }
    for r in refs {
        if !sources.contains(&r) {
            p.push(format!("source {r} is not listed in eval/README.md"));
        }
    }

    if names(t.get("turns")).is_empty() {
        p.push("no turns".into());
    }
    if t.pointer("/expect/facts")
        .and_then(Value::as_array)
        .is_none()
    {
        p.push("missing expect.facts".into());
    }
    if t.pointer("/expect/must_not_call")
        .and_then(Value::as_array)
        .is_none()
    {
        p.push("missing expect.must_not_call (use [] for none)".into());
    }

    let required = names(t.pointer("/tools/required"));
    let mut named = required.clone();
    named.extend(names(t.pointer("/tools/allowed")));
    named.extend(names(t.pointer("/expect/must_not_call")));
    if let Some(items) = t
        .pointer("/expect/change_set/items")
        .and_then(Value::as_array)
    {
        for i in items {
            let tool = i.get("tool").and_then(Value::as_str).unwrap_or_default();
            if !required.iter().any(|r| r == tool) {
                p.push(format!("change set item `{tool}` is not in tools.required"));
            }
        }
    }
    let surface = s("surface");
    for n in &named {
        match tools.built.get(n) {
            Some(a) if !visible(*a, &surface) => {
                p.push(format!(
                    "`{n}` is built but not callable on surface `{surface}`"
                ));
            }
            Some(_) => {}
            None if tools.specified.contains(n) => {}
            None => p.push(format!("`{n}` is neither built nor in docs/api")),
        }
    }

    let unbuilt: Vec<String> = required
        .iter()
        .filter(|n| !tools.built.contains_key(*n))
        .cloned()
        .collect();
    let data_gap = names(t.get("gaps")).iter().any(|g| g.starts_with("data:"));
    let coverage = if !unbuilt.is_empty() {
        "spec"
    } else if data_gap {
        "data"
    } else {
        "ready"
    };
    Row {
        set,
        id,
        job,
        kind,
        phrasing,
        coverage,
        unbuilt,
    }
}

fn visible(a: Audience, surface: &str) -> bool {
    match surface {
        "bo" => matches!(a, Audience::Operator | Audience::Both),
        "fo" => matches!(a, Audience::Shopper | Audience::Both),
        _ => false,
    }
}

fn names(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// `| R12 | ...` rows of the sources table in `eval/README.md`.
fn source_ids(readme: &str) -> BTreeSet<String> {
    readme
        .lines()
        .filter_map(|l| {
            let id = l.strip_prefix("| ")?.split_once(' ')?.0;
            (id.len() > 1 && id.starts_with('R') && id[1..].bytes().all(|b| b.is_ascii_digit()))
                .then(|| id.to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> Tools {
        Tools {
            built: [
                ("search_products".to_owned(), Audience::Both),
                ("add_to_cart".to_owned(), Audience::Shopper),
            ]
            .into(),
            specified: ["get_sales_overview".to_owned()].into(),
        }
    }

    fn task(yaml: &str) -> (Row, Vec<String>) {
        let t: Value = serde_norway::from_str(yaml).unwrap();
        let mut p = Vec::new();
        let sources = ["R1".to_owned()].into();
        (check_task("design", &t, &tools(), &sources, &mut p), p)
    }

    const BASE: &str = "id: t\njob: owner\npersona: adm_sj\nsurface: bo\nfrequency: daily\nkind: read\nsources: [R1]\nturns: [hi]\nexpect: {facts: [], must_not_call: []}\n";

    #[test]
    fn derives_coverage_from_where_tools_live() {
        let (r, p) = task(&format!(
            "{BASE}phrasing: {{how: ours}}\ntools: {{required: [search_products], allowed: []}}"
        ));
        assert_eq!((r.coverage, p.len()), ("ready", 0), "{p:?}");
        let (r, _) = task(&format!(
            "{BASE}phrasing: {{how: ours}}\ntools: {{required: [get_sales_overview], allowed: []}}"
        ));
        assert_eq!(
            (r.coverage, r.unbuilt),
            ("spec", vec!["get_sales_overview".to_owned()])
        );
        let (r, _) = task(&format!(
            "{BASE}phrasing: {{how: ours}}\ntools: {{required: [], allowed: []}}\ngaps: [\"data: none\"]"
        ));
        assert_eq!(r.coverage, "data");
    }

    #[test]
    fn rejects_unknown_tools_wrong_surface_and_unsourced_phrasing() {
        let (_, p) = task(&format!(
            "{BASE}phrasing: {{how: verbatim, from: R9}}\ntools: {{required: [add_to_cart, make_it_so], allowed: []}}"
        ));
        let joined = p.join("\n");
        assert!(joined.contains("`make_it_so` is neither built"), "{joined}");
        assert!(
            joined.contains("`add_to_cart` is built but not callable on surface `bo`"),
            "{joined}"
        );
        assert!(joined.contains("source R9 is not listed"), "{joined}");
        assert!(joined.contains("needs `original`"), "{joined}");
    }

    #[test]
    fn reads_source_table_rows() {
        let ids = source_ids("| ID | x |\n|---|---|\n| R1 | a |\n| R12 | b |\n| Rx | c |\n");
        assert_eq!(ids, ["R1".to_owned(), "R12".to_owned()].into());
    }
}
