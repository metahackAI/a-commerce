//! The named demo rows: `seed/demo/01_platform.yml` ... `08_messaging.yml`.
//!
//! Each file is `tables: {schema.table: [row, ...]}`. Files load in name order, tables in the
//! order written and rows in the order listed; foreign keys hold in that order
//! (`seed/demo/README.md`).

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::SeedRows;

/// Reads every named seed file in `dir` (a `.yml` file whose name starts with a digit; the
/// generator spec `generate.yml` is not one) and returns its rows in load order.
pub fn load(dir: &Path) -> Result<SeedRows, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut files: Vec<_> = entries
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
    if files.is_empty() {
        return Err(format!("{}: no named seed files", dir.display()));
    }

    let mut rows = SeedRows::new();
    for path in files {
        let name = path.display().to_string();
        let text = fs::read_to_string(&path).map_err(|e| format!("{name}: {e}"))?;
        let doc: Value =
            serde_norway::from_str(&text).map_err(|e| format!("{name}: not valid YAML: {e}"))?;
        let tables = doc
            .get("tables")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{name}: no `tables:` map"))?;
        for (table, list) in tables {
            let list = list
                .as_array()
                .ok_or_else(|| format!("{name}: `{table}` is not a list of rows"))?;
            for (i, row) in list.iter().enumerate() {
                if !row.is_object() {
                    return Err(format!("{name}: {table}[{i}] is not a map"));
                }
                rows.push((table.clone(), row.clone()));
            }
        }
    }
    Ok(rows)
}
