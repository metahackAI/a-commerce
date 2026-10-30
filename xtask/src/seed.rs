//! `cargo xtask check-seed`: the demo seed (`seed/demo/*.yml`) fits the migrations.
//!
//! Without a database: every table and column exists, required columns are present, word
//! columns hold words from their `CHECK` list, primary keys are unique, and every reference
//! points at a row seeded earlier in load order (tenant included).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::schema::Schema;

#[derive(Debug, Default)]
pub struct SeedReport {
    pub files: usize,
    pub rows: usize,
    pub rows_by_table: BTreeMap<String, usize>,
    pub problems: Vec<String>,
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

pub fn check(dir: &Path, schema: &Schema) -> std::io::Result<SeedReport> {
    let mut files: Vec<_> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "yml")
                && p.file_name().is_some_and(|n| {
                    n.to_string_lossy()
                        .starts_with(|c: char| c.is_ascii_digit())
                })
        })
        .collect();
    files.sort();

    let mut report = SeedReport::default();
    // (table, tenant, id-tuple) of every row loaded so far, for primary keys and references.
    let mut keys: BTreeSet<(String, Vec<String>)> = BTreeSet::new();
    for path in files {
        report.files += 1;
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let doc: Value = match serde_norway::from_str(&fs::read_to_string(&path)?) {
            Ok(v) => v,
            Err(e) => {
                report.problems.push(format!("{file}: not valid YAML: {e}"));
                continue;
            }
        };
        let Some(tables) = doc["tables"].as_object() else {
            report.problems.push(format!("{file}: no `tables:` map"));
            continue;
        };
        for (table_name, rows) in tables {
            let Some(table) = schema.tables.get(table_name).filter(|t| t.kind == "table") else {
                report
                    .problems
                    .push(format!("{file}: unknown table `{table_name}`"));
                continue;
            };
            for (i, row) in rows.as_array().into_iter().flatten().enumerate() {
                let at = format!("{file} {table_name}[{i}]");
                let Some(row) = row.as_object() else {
                    report.problems.push(format!("{at}: row is not a map"));
                    continue;
                };
                report.rows += 1;
                *report.rows_by_table.entry(table_name.clone()).or_default() += 1;
                for key in row.keys() {
                    if !table.columns.iter().any(|c| &c.name == key) {
                        report
                            .problems
                            .push(format!("{at}: unknown column `{key}`"));
                    }
                }
                for c in &table.columns {
                    let value = row.get(&c.name).filter(|v| !v.is_null());
                    if value.is_none() && !c.nullable && !c.has_default {
                        report
                            .problems
                            .push(format!("{at}: required column `{}` is missing", c.name));
                    }
                    let Some(value) = value else { continue };
                    if let Some(words) = &c.values
                        && let Some(word) = scalar(value)
                        && !words.contains(&word)
                    {
                        report.problems.push(format!(
                            "{at}: `{}` = `{word}` is not one of {words:?}",
                            c.name
                        ));
                    }
                    if let Some(target) = &c.references {
                        let Some(id) = scalar(value) else { continue };
                        let tenant = if target == "platform.online_store" {
                            Vec::new()
                        } else {
                            vec![row.get("tenant_id").and_then(scalar).unwrap_or_default()]
                        };
                        let mut key = tenant;
                        key.push(id.clone());
                        if !keys.contains(&(target.clone(), key)) {
                            report.problems.push(format!(
                                "{at}: `{}` = `{id}` has no row in {target} (seeded earlier)",
                                c.name
                            ));
                        }
                    }
                }
                let pk: Vec<String> = table
                    .primary_key
                    .iter()
                    .map(|k| row.get(k).and_then(scalar).unwrap_or_default())
                    .collect();
                if !table.primary_key.is_empty() && !keys.insert((table_name.clone(), pk.clone())) {
                    report
                        .problems
                        .push(format!("{at}: duplicate primary key {pk:?}"));
                }
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQL: &str = r#"
CREATE TABLE platform.online_store (
  id text PRIMARY KEY CHECK (id ~ '^store_[a-z0-9_]+$')
);
CREATE TABLE catalog.offer (
  tenant_id text NOT NULL REFERENCES platform.online_store(id),
  id text NOT NULL,
  price numeric(12,2) NOT NULL,
  availability text NOT NULL DEFAULT 'in_stock',
  CONSTRAINT offer_pkey PRIMARY KEY (tenant_id, id),
  CONSTRAINT offer_availability_check CHECK (availability IN ('in_stock', 'out_of_stock'))
);
"#;

    #[test]
    fn finds_unknown_columns_missing_values_bad_words_duplicates_and_dangling_references() {
        let mut schema = Schema::default();
        schema.parse_sql("t.sql", SQL);
        let dir = std::env::temp_dir().join(format!("acom-seed-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("01_all.yml"),
            "tables:\n  platform.online_store:\n    - {id: store_a}\n  catalog.offer:\n    - {tenant_id: store_a, id: off_1, price: 1.00}\n    - {tenant_id: store_a, id: off_1, price: 2.00, availability: sold}\n    - {tenant_id: store_b, id: off_2, colour: red}\n",
        )
        .unwrap();
        let report = check(&dir, &schema).unwrap();
        let all = report.problems.join("\n");
        fs::remove_dir_all(&dir).ok();
        assert_eq!(report.rows, 4);
        assert!(all.contains("duplicate primary key"), "{all}");
        assert!(all.contains("`availability` = `sold`"), "{all}");
        assert!(all.contains("unknown column `colour`"), "{all}");
        assert!(all.contains("required column `price` is missing"), "{all}");
        assert!(all.contains("`tenant_id` = `store_b` has no row"), "{all}");
    }
}
