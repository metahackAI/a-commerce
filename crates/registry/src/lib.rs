//! The one place that knows every domain.
//!
//! Each domain crate exposes `register`; this crate calls them all and builds the registry the
//! commerce service serves over REST and MCP. A test holds every registered operation to its
//! spec in `docs/api/` (tool name, address, kind, method, path).

use acom_contract::{Registry, RegistryBuilder};
use acom_store::Cx;

/// Every operation of every domain. Fails when an input schema is not closed and flat.
pub fn build() -> Result<Registry<Cx>, Vec<String>> {
    let mut b = RegistryBuilder::default();
    acom_catalog::register(&mut b);
    acom_search::register(&mut b);
    acom_merchandising::register(&mut b);
    acom_promotion::register(&mut b);
    acom_customer::register(&mut b);
    acom_sales::register(&mut b);
    acom_fulfillment::register(&mut b);
    acom_messaging::register(&mut b);
    acom_platform::register(&mut b);
    b.build()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use acom_contract::{Kind, Method};

    use super::*;

    struct Spec {
        kind: String,
        address: String,
        method: String,
        path: String,
    }

    fn backticks(line: &str) -> Vec<&str> {
        line.split('`').skip(1).step_by(2).collect()
    }

    /// `### \`tool\` — Kind: X` and the `REST` line under it, from every domain file.
    fn specs() -> BTreeMap<String, Spec> {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/api");
        let mut out = BTreeMap::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "md") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let Some(rest) = line.strip_prefix("### `") else {
                    continue;
                };
                let Some((tool, tail)) = rest.split_once('`') else {
                    continue;
                };
                let Some(kind) = tail.split("Kind:").nth(1) else {
                    continue;
                };
                let kind = kind.trim();
                let kind = if kind.starts_with("W$") {
                    "W$"
                } else if kind.starts_with('W') {
                    "W"
                } else {
                    "R"
                };
                let Some(rest_line) = lines[i + 1..]
                    .iter()
                    .take(4)
                    .find(|l| l.starts_with("- ") && l.contains("REST"))
                else {
                    continue;
                };
                let ticks = backticks(rest_line);
                let (Some(address), Some(route)) = (ticks.first(), ticks.get(1)) else {
                    continue;
                };
                let (method, path) = route.split_once(' ').unwrap_or(("", route));
                let path = path.split('?').next().unwrap_or(path);
                out.insert(
                    tool.to_owned(),
                    Spec {
                        kind: kind.to_owned(),
                        address: (*address).to_owned(),
                        method: method.to_owned(),
                        path: path.to_owned(),
                    },
                );
            }
        }
        out
    }

    fn method(m: Method) -> &'static str {
        match m {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
        }
    }

    /// Path parameter names may differ between documents (`{id}`, `{change_set_id}`).
    fn shape(path: &str) -> String {
        path.split('/')
            .map(|s| if s.starts_with('{') { "{}" } else { s })
            .collect::<Vec<_>>()
            .join("/")
    }

    #[test]
    fn every_operation_matches_its_spec() {
        let registry = build().unwrap_or_else(|e| panic!("registry: {e:#?}"));
        let specs = specs();
        let policy = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../policies/approval.yml"
        ))
        .unwrap();
        let mut problems = Vec::new();
        for op in registry.iter() {
            let m = op.meta();
            let route = format!("{} {}", method(m.method), m.path);
            if m.kind == Kind::Person {
                let listed = policy
                    .lines()
                    .filter_map(|l| l.split("endpoint: \"").nth(1))
                    .filter_map(|l| l.split('"').next())
                    .any(|e| shape(e) == shape(&route));
                if !listed {
                    problems.push(format!(
                        "{}: person-only `{route}` is not in approval.yml person_only",
                        m.tool
                    ));
                }
                continue;
            }
            let Some(s) = specs.get(m.tool) else {
                problems.push(format!("{}: no section in docs/api", m.tool));
                continue;
            };
            if s.kind != m.kind.spec_mark() {
                problems.push(format!(
                    "{}: kind {} but the spec says {}",
                    m.tool,
                    m.kind.spec_mark(),
                    s.kind
                ));
            }
            if s.address != m.address {
                problems.push(format!(
                    "{}: address {} but the spec says {}",
                    m.tool, m.address, s.address
                ));
            }
            if s.method != method(m.method) || s.path != m.path {
                problems.push(format!(
                    "{}: `{route}` but the spec says `{} {}`",
                    m.tool, s.method, s.path
                ));
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn routes_do_not_collide() {
        let registry = build().unwrap();
        let mut seen = BTreeMap::new();
        for op in registry.iter() {
            let m = op.meta();
            let key = (method(m.method), shape(m.path));
            if let Some(other) = seen.insert(key.clone(), m.tool) {
                panic!("{} and {} share {} {}", other, m.tool, key.0, key.1);
            }
        }
    }
}
