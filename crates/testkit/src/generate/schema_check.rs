//! Test-only: the checks of `cargo xtask check-seed`, applied to rows in memory.
//!
//! Reads `migrations/*.sql` (one column or constraint per line inside `CREATE TABLE`) for
//! columns, NOT NULL without a default, `CHECK (col IN (...))` word lists, column references
//! and primary keys. Then every row: the table and its columns exist, required columns are
//! present, words are from their list, primary keys are unique, and every reference points
//! at a row loaded earlier (tenant included).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::Value;

#[derive(Debug, Default)]
pub(crate) struct Column {
    pub name: String,
    pub required: bool,
    pub words: Option<Vec<String>>,
    pub references: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct Table {
    pub columns: Vec<Column>,
    pub primary_key: Vec<String>,
}

fn inner(s: &str) -> Option<&str> {
    let start = s.find('(')?;
    let mut depth = 0;
    for (i, c) in s[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start + 1..start + i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn in_list(expr: &str) -> Option<(String, Vec<String>)> {
    let pos = expr.find(" IN (")?;
    let col = expr[..pos]
        .trim()
        .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .next()?
        .to_owned();
    let rest = &expr[pos + 5..];
    let words = rest[..rest.find(')')?]
        .split(',')
        .map(|w| w.trim().trim_matches('\'').to_owned())
        .collect();
    Some((col, words))
}

fn target(after_references: &str) -> String {
    after_references
        .trim()
        .split(|c: char| c == '(' || c.is_whitespace())
        .next()
        .unwrap_or("")
        .to_owned()
}

pub(crate) fn parse(dir: &Path) -> BTreeMap<String, Table> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sql"))
        .collect();
    files.sort();
    let mut tables = BTreeMap::new();
    for f in files {
        let sql = fs::read_to_string(f).unwrap();
        let mut current: Option<(String, Table)> = None;
        for line in sql.lines() {
            let t = line.trim().trim_end_matches(',');
            if let Some(rest) = t.strip_prefix("CREATE TABLE ") {
                let name = rest.split([' ', '(']).next().unwrap().to_owned();
                current = Some((name, Table::default()));
                continue;
            }
            let Some((_, table)) = current.as_mut() else {
                continue;
            };
            if t.starts_with(')') {
                let (name, table) = current.take().unwrap();
                tables.insert(name, table);
                continue;
            }
            if let Some(c) = t.strip_prefix("CONSTRAINT ") {
                if let Some(p) = c.find("PRIMARY KEY") {
                    table.primary_key = inner(&c[p..])
                        .unwrap()
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .collect();
                } else if let Some(p) = c.find("FOREIGN KEY") {
                    let r = c.find(" REFERENCES ").unwrap();
                    let to = target(&c[r + 12..]);
                    for col in inner(&c[p..]).unwrap().split(',').map(str::trim) {
                        if col != "tenant_id" {
                            let col = table.columns.iter_mut().find(|x| x.name == col).unwrap();
                            col.references = Some(to.clone());
                        }
                    }
                } else if let Some(p) = c.find("CHECK")
                    && let Some((col, words)) = inner(&c[p..]).and_then(in_list)
                    && let Some(col) = table.columns.iter_mut().find(|x| x.name == col)
                {
                    col.words = Some(words);
                }
                continue;
            }
            let (col, rest) = t.split_once(' ').unwrap();
            let rest = format!(" {rest}");
            table.columns.push(Column {
                name: col.to_owned(),
                required: rest.contains(" NOT NULL") && !rest.contains(" DEFAULT "),
                words: None,
                references: rest.find(" REFERENCES ").map(|r| target(&rest[r + 12..])),
            });
        }
    }
    tables
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Problems of `rows` (in load order) against `tables`.
pub(crate) fn check(tables: &BTreeMap<String, Table>, rows: &[(String, Value)]) -> Vec<String> {
    let mut problems = Vec::new();
    let mut keys: BTreeSet<(String, Vec<String>)> = BTreeSet::new();
    for (i, (name, row)) in rows.iter().enumerate() {
        let at = format!("row {i} {name}");
        let Some(table) = tables.get(name) else {
            problems.push(format!("{at}: unknown table"));
            continue;
        };
        let Some(map) = row.as_object() else {
            problems.push(format!("{at}: not an object"));
            continue;
        };
        for key in map.keys() {
            if !table.columns.iter().any(|c| &c.name == key) {
                problems.push(format!("{at}: unknown column `{key}`"));
            }
        }
        for c in &table.columns {
            let value = map.get(&c.name).filter(|v| !v.is_null());
            if value.is_none() && c.required {
                problems.push(format!("{at}: required column `{}` is missing", c.name));
            }
            let Some(value) = value else { continue };
            if let (Some(words), Some(word)) = (&c.words, scalar(value))
                && !words.contains(&word)
            {
                problems.push(format!("{at}: `{}` = `{word}` is not in {words:?}", c.name));
            }
            if let (Some(to), Some(id)) = (&c.references, scalar(value)) {
                let mut key = if to == "platform.online_store" {
                    Vec::new()
                } else {
                    vec![map.get("tenant_id").and_then(scalar).unwrap_or_default()]
                };
                key.push(id.clone());
                if !keys.contains(&(to.clone(), key)) {
                    problems.push(format!(
                        "{at}: `{}` = `{id}` has no earlier row in {to}",
                        c.name
                    ));
                }
            }
        }
        let pk: Vec<String> = table
            .primary_key
            .iter()
            .map(|k| map.get(k).and_then(scalar).unwrap_or_default())
            .collect();
        if !keys.insert((name.clone(), pk.clone())) {
            problems.push(format!("{at}: duplicate primary key {pk:?}"));
        }
    }
    problems
}
