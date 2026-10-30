//! Repository tasks. `cargo xtask <task>`.
//!
//! - `check-docs`: the operation index in `docs/api/README.md` and the operation sections in
//!   the domain files must list the same tools with the same kinds. Later this also compares
//!   both against the registry (`docs/backend/testing.md` §1).
//! - `check-schema [--write]`: the migrations follow `docs/backend/database.md`; `--write`
//!   regenerates the data dictionary and `vocab/acom.jsonld`, otherwise stale files are an error.
//! - `check-seed`: the demo seed fits the migrations (columns, required values, words, keys,
//!   references), without a database.
//! - `check-eval`: the agent task set in `eval/tasks/` is well formed and names only tools that
//!   are built or specified; prints each task's derived coverage (`eval/README.md`).
//! - `hash-password <password>`: an argon2id hash for seed accounts.
//! - `dev-token <persona> [--agent]`: an internal token signed with the local key
//!   (`infra/secrets/gateway_signing_key.pem`), to call commerce without the gateway while
//!   developing. Personas: `admin`, `jamie`, `alex`, `guest`; `--agent` makes the on-behalf
//!   token a chat turn would carry.

mod eval;
mod schema;
mod seed;
mod vocab;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    match task.as_str() {
        "check-docs" => check_docs(&repo_root()),
        "check-schema" => check_schema(&repo_root(), std::env::args().any(|a| a == "--write")),
        "check-seed" => check_seed(&repo_root()),
        "check-eval" => check_eval(&repo_root()),
        "hash-password" => match std::env::args().nth(2) {
            Some(pw) => match acom_auth::password::hash(&pw) {
                Ok(h) => {
                    println!("{h}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            },
            None => {
                eprintln!("usage: cargo xtask hash-password <password>");
                ExitCode::FAILURE
            }
        },
        "dev-token" => dev_token(
            &repo_root(),
            std::env::args().nth(2).as_deref().unwrap_or("admin"),
            std::env::args().any(|a| a == "--agent"),
        ),
        _ => {
            eprintln!(
                "usage: cargo xtask check-docs | check-schema [--write] | check-seed | check-eval | hash-password <password> | dev-token <admin|jamie|alex|guest> [--agent]"
            );
            ExitCode::FAILURE
        }
    }
}

fn dev_token(root: &Path, persona: &str, agent: bool) -> ExitCode {
    use acom_auth::{Principal, Signer, Surface, scopes};
    use acom_core::TenantId;
    use acom_core::enums::ActorKind;
    let Ok(tenant) = TenantId::parse("store_northwind") else {
        return ExitCode::FAILURE;
    };
    let (subject, kind, surface, set, name) = match persona {
        "admin" => (
            "adm_admin",
            ActorKind::Operator,
            Surface::Bo,
            scopes::admin(),
            "Admin",
        ),
        "jamie" => (
            "cus_jamie",
            ActorKind::Shopper,
            Surface::Fo,
            scopes::shopper(),
            "Jamie",
        ),
        "alex" => (
            "cus_alex",
            ActorKind::Shopper,
            Surface::Fo,
            scopes::shopper(),
            "Alex",
        ),
        "guest" => (
            "ses_dev_guest",
            ActorKind::Shopper,
            Surface::Fo,
            scopes::guest(),
            "Guest",
        ),
        other => {
            eprintln!("unknown persona `{other}` (admin, jamie, alex, guest)");
            return ExitCode::FAILURE;
        }
    };
    let mut p = Principal {
        subject: subject.to_owned(),
        kind,
        tenant,
        surface,
        scopes: set,
        name: Some(name.to_owned()),
        session_id: Some("ses_dev".to_owned()),
    };
    if agent {
        p = p.on_behalf();
    }
    let key = root.join("infra/secrets/gateway_signing_key.pem");
    let signer = match Signer::from_pem_file(&key.to_string_lossy()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e} (run `make keys`)");
            return ExitCode::FAILURE;
        }
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match signer.sign(&p, "req_dev", now, 12 * 3600) {
        Ok(t) => {
            println!("{t}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
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

fn check_seed(root: &Path) -> ExitCode {
    let parsed = match schema::Schema::parse_dir(&root.join("migrations")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read migrations/: {e}");
            return ExitCode::FAILURE;
        }
    };
    let report = match seed::check(&root.join("seed/demo"), &parsed) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot read seed/demo/: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "seed: {} files, {} rows in {} tables",
        report.files,
        report.rows,
        report.rows_by_table.len()
    );
    for p in &report.problems {
        println!("- {p}");
    }
    if report.problems.is_empty() {
        println!("seed fits the schema");
        ExitCode::SUCCESS
    } else {
        println!("{} problem(s)", report.problems.len());
        ExitCode::FAILURE
    }
}

fn check_eval(root: &Path) -> ExitCode {
    let registry = match acom_registry::build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("registry does not build: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    let mut specified = std::collections::BTreeSet::new();
    let docs = match fs::read_dir(root.join("docs/api")) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("cannot read docs/api/: {e}");
            return ExitCode::FAILURE;
        }
    };
    for path in docs.filter_map(Result::ok).map(|e| e.path()) {
        if path.extension().is_some_and(|x| x == "md") {
            let text = fs::read_to_string(&path).unwrap_or_default();
            specified.extend(parse_sections(&text).into_iter().map(|(tool, _)| tool));
        }
    }
    let tools = eval::Tools {
        built: registry
            .iter()
            .map(|op| (op.meta().tool.to_owned(), op.meta().audience))
            .collect(),
        specified,
    };
    let report = match eval::check(root, &tools) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot read eval/: {e}");
            return ExitCode::FAILURE;
        }
    };
    for r in &report.rows {
        let unbuilt = if r.unbuilt.is_empty() {
            String::new()
        } else {
            format!("  needs {}", r.unbuilt.join(", "))
        };
        println!(
            "{:<9} {:<32} {:<13} {:<15} {:<9} {}{unbuilt}",
            r.set, r.id, r.job, r.kind, r.phrasing, r.coverage
        );
    }
    let mut by: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for r in &report.rows {
        *by.entry((r.set, r.coverage)).or_default() += 1;
    }
    let summary: Vec<String> = by
        .iter()
        .map(|((s, c), n)| format!("{s} {c} {n}"))
        .collect();
    println!("tasks: {} ({})", report.rows.len(), summary.join(", "));
    for p in &report.problems {
        println!("- {p}");
    }
    if report.problems.is_empty() {
        println!("task set is consistent");
        ExitCode::SUCCESS
    } else {
        println!("{} problem(s)", report.problems.len());
        ExitCode::FAILURE
    }
}

fn check_schema(root: &Path, write: bool) -> ExitCode {
    let parsed = match schema::Schema::parse_dir(&root.join("migrations")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read migrations/: {e}");
            return ExitCode::FAILURE;
        }
    };
    let problems = parsed.check(&acom_core::enums::words);
    let columns: usize = parsed.tables.values().map(|t| t.columns.len()).sum();
    println!(
        "schema: {} tables and views, {columns} columns",
        parsed.tables.len()
    );

    let terms: serde_json::Value = match fs::read_to_string(root.join("vocab/terms.json"))
        .map_err(|e| e.to_string())
        .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("cannot read vocab/terms.json: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (vocabulary, vocab_problems) = vocab::generate(&parsed, &terms, acom_core::enums::LISTS);
    let mut problems = problems;
    problems.extend(vocab_problems);
    let outputs = [
        (
            root.join("docs/backend/schema.generated.md"),
            parsed.markdown(),
        ),
        (
            root.join("docs/api/generated/schema-map.json"),
            parsed.json(),
        ),
        (root.join("vocab/acom.jsonld"), vocabulary),
    ];
    let mut stale = Vec::new();
    for (path, content) in &outputs {
        if write {
            if let Some(dir) = path.parent()
                && let Err(e) = fs::create_dir_all(dir)
            {
                eprintln!("cannot create {}: {e}", dir.display());
                return ExitCode::FAILURE;
            }
            if let Err(e) = fs::write(path, content) {
                eprintln!("cannot write {}: {e}", path.display());
                return ExitCode::FAILURE;
            }
        } else if fs::read_to_string(path).ok().as_deref() != Some(content.as_str()) {
            stale.push(
                path.strip_prefix(root)
                    .unwrap_or(path)
                    .display()
                    .to_string(),
            );
        }
    }

    for p in &problems {
        println!("- {p}");
    }
    for s in &stale {
        println!("- {s} is out of date (run `cargo xtask check-schema --write`)");
    }
    if problems.is_empty() && stale.is_empty() {
        println!(
            "{}",
            if write {
                "schema follows the rules; dictionary written"
            } else {
                "schema follows the rules; dictionary is current"
            }
        );
        ExitCode::SUCCESS
    } else {
        println!("{} problem(s)", problems.len() + stale.len());
        ExitCode::FAILURE
    }
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
