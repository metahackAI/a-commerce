//! `cargo xtask check-layout [domain…]` and `cargo xtask check-coverage [domain…]`.
//!
//! The API is a hierarchy, `domain.resource.operation` (`catalog.offer.set_price`), and the code
//! follows it: an operation lives in `crates/domains/{domain}/src/{resource}/{tool}.rs`, one file
//! per operation, one folder per resource (docs/backend/README.md §2.2). `check-layout` finds each
//! registered operation's `OpMeta` in the sources and says where it is and where it belongs.
//!
//! `check-coverage` holds the registry to the whole spec: every tool of the index in
//! `docs/api/README.md` §4 and every person-only endpoint of §2.1 is registered (the change-set
//! decisions approve, reject and undo are runtime endpoints of `acom-http`, not registry
//! operations).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use acom_contract::Kind;

/// Where an operation's `OpMeta` was found.
#[derive(Debug)]
struct Found {
    address: String,
    file: PathBuf,
}

/// `OpMeta { … }` blocks in the domain crates, by tool name.
fn find_metas(root: &Path) -> BTreeMap<String, Vec<Found>> {
    let mut out: BTreeMap<String, Vec<Found>> = BTreeMap::new();
    let mut files = Vec::new();
    collect(&root.join("crates/domains"), &mut files);
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let mut rest = text.as_str();
        while let Some(i) = rest.find("OpMeta {") {
            let block = &rest[i..];
            let end = block.find("};").unwrap_or(block.len());
            let body = &block[..end];
            let field = |name: &str| {
                body.split_once(&format!("{name}: \""))
                    .and_then(|(_, t)| t.split_once('"'))
                    .map(|(v, _)| v.to_owned())
            };
            if let (Some(tool), Some(address)) = (field("tool"), field("address")) {
                out.entry(tool).or_default().push(Found {
                    address,
                    file: file.strip_prefix(root).unwrap_or(&file).to_path_buf(),
                });
            }
            rest = &block[end.min(block.len())..];
            if end == block.len() {
                break;
            }
        }
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The file an operation belongs in: `crates/domains/{domain}/src/{resource}/{tool}.rs`.
pub fn home(address: &str, tool: &str) -> Option<PathBuf> {
    let mut parts = address.split('.');
    let (domain, resource) = (parts.next()?, parts.next()?);
    parts.next()?;
    Some(PathBuf::from(format!(
        "crates/domains/{domain}/src/{resource}/{tool}.rs"
    )))
}

fn wanted(args: &[String], domain: &str) -> bool {
    args.is_empty() || args.iter().any(|a| a == domain)
}

pub fn check_layout(root: &Path, domains: &[String]) -> ExitCode {
    let registry = match acom_registry::build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("the registry does not build: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    let metas = find_metas(root);
    let mut by_domain: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    for op in registry.iter() {
        let m = op.meta();
        let domain = m.address.split('.').next().unwrap_or_default().to_owned();
        if !wanted(domains, &domain) {
            continue;
        }
        let entry = by_domain.entry(domain).or_default();
        entry.0 += 1;
        let Some(want) = home(m.address, m.tool) else {
            entry.1.push(format!(
                "`{}` has address {} (not domain.resource.operation)",
                m.tool, m.address
            ));
            continue;
        };
        let found: Vec<&Found> = metas
            .get(m.tool)
            .map(|v| v.iter().filter(|f| f.address == m.address).collect())
            .unwrap_or_default();
        match found.as_slice() {
            [] => entry
                .1
                .push(format!("`{}`: no OpMeta found in crates/domains", m.tool)),
            [f] if f.file == want => {}
            [f] => entry.1.push(format!(
                "`{}` is in {}; it belongs in {}",
                m.tool,
                f.file.display(),
                want.display()
            )),
            many => entry.1.push(format!(
                "`{}` has {} OpMeta blocks ({})",
                m.tool,
                many.len(),
                many.iter()
                    .map(|f| f.file.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
    let mut bad = 0;
    for (domain, (total, problems)) in &by_domain {
        println!(
            "{domain:14} {:3}/{total:<3} operations where their address says",
            total - problems.len()
        );
        for p in problems {
            println!("    {p}");
        }
        bad += problems.len();
    }
    if bad == 0 {
        println!("layout follows domain/resource/tool");
        ExitCode::SUCCESS
    } else {
        println!("{bad} operation(s) out of place");
        ExitCode::FAILURE
    }
}

/// Person-only endpoints of `docs/api/README.md` §2.1, as `METHOD /path` with `{}` for parameters.
fn person_endpoints(readme: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in readme.lines() {
        if line.starts_with("### ") {
            inside = line.starts_with("### 2.1");
            continue;
        }
        if !inside {
            continue;
        }
        let Some(cell) = line.strip_prefix("| `") else {
            continue;
        };
        let Some((endpoint, _)) = cell.split_once('`') else {
            continue;
        };
        if let Some((method, path)) = endpoint.split_once(' ') {
            out.push((method.to_owned(), path.to_owned()));
        }
    }
    out
}

fn shape(path: &str) -> String {
    let mut s = String::new();
    let mut depth = 0;
    for c in path.chars() {
        match c {
            '{' => {
                depth += 1;
                if depth == 1 {
                    s.push_str("{}");
                }
            }
            '}' => depth -= 1,
            _ if depth > 0 => {}
            _ => s.push(c),
        }
    }
    s
}

/// The change-set decisions served by `acom-http`'s runtime router, not the registry.
const RUNTIME: [(&str, &str); 3] = [
    ("POST", "/v1/platform/change-sets/{}/approve"),
    ("POST", "/v1/platform/change-sets/{}/reject"),
    ("POST", "/v1/platform/change-sets/{}/undo"),
];

pub fn check_coverage(root: &Path, domains: &[String]) -> ExitCode {
    let readme = match fs::read_to_string(root.join("docs/api/README.md")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read docs/api/README.md: {e}");
            return ExitCode::FAILURE;
        }
    };
    let registry = match acom_registry::build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("the registry does not build: {e:?}");
            return ExitCode::FAILURE;
        }
    };
    let built: BTreeSet<&str> = registry.iter().map(|op| op.meta().tool).collect();
    let persons: BTreeSet<(String, String)> = registry
        .iter()
        .filter(|op| op.meta().kind == Kind::Person)
        .map(|op| {
            let m = op.meta();
            (format!("{:?}", m.method).to_uppercase(), shape(m.path))
        })
        .chain(
            RUNTIME
                .iter()
                .map(|(m, p)| ((*m).to_owned(), (*p).to_owned())),
        )
        .collect();

    let index = crate::parse_index(&readme);
    let mut per: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
    for (tool, e) in &index {
        if !wanted(domains, &e.domain) {
            continue;
        }
        let entry = per.entry(e.domain.clone()).or_default();
        entry.0 += 1;
        if !built.contains(tool.as_str()) {
            entry.1.push(format!("{tool} ({})", e.kind));
        }
    }
    let mut missing = 0;
    for (domain, (total, absent)) in &per {
        println!(
            "{domain:14} {:3}/{total:<3} tools built",
            total - absent.len()
        );
        if !absent.is_empty() {
            println!("    missing: {}", absent.join(", "));
        }
        missing += absent.len();
    }
    let endpoints = person_endpoints(&readme);
    let absent: Vec<String> = endpoints
        .iter()
        .filter(|(_, p)| {
            let domain = p
                .trim_start_matches("/v1/")
                .split('/')
                .next()
                .unwrap_or_default();
            wanted(domains, domain)
        })
        .filter(|(m, p)| !persons.contains(&(m.clone(), shape(p))))
        .map(|(m, p)| format!("{m} {p}"))
        .collect();
    println!(
        "person-only    {:3}/{:<3} endpoints built",
        endpoints.len() - absent.len().min(endpoints.len()),
        endpoints.len()
    );
    for a in &absent {
        println!("    missing: {a}");
    }
    missing += absent.len();
    if missing == 0 {
        println!("every specified operation is built");
        ExitCode::SUCCESS
    } else {
        println!("{missing} operation(s) not built yet");
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_operation_lives_where_its_address_says() {
        assert_eq!(
            home("catalog.offer.set_price", "set_price"),
            Some(PathBuf::from(
                "crates/domains/catalog/src/offer/set_price.rs"
            ))
        );
        assert_eq!(
            home("sales.gift_order.accept", "accept_gift_order"),
            Some(PathBuf::from(
                "crates/domains/sales/src/gift_order/accept_gift_order.rs"
            ))
        );
        assert_eq!(home("catalog", "x"), None);
    }

    #[test]
    fn person_endpoints_are_read_from_section_2_1() {
        let readme = "### 2.1 x\n| a | b |\n|---|---|\n| `POST /v1/sales/orders` | `{preview_id}` |\n### 2.2 y\n| `GET /v1/no` | |\n";
        assert_eq!(
            person_endpoints(readme),
            [("POST".to_owned(), "/v1/sales/orders".to_owned())]
        );
        assert_eq!(shape("/v1/x/{id}/y/{other_id}"), "/v1/x/{}/y/{}");
    }
}
