//! Writes seed rows into the database (`acom-commerce seed`).
//!
//! Rows go in the order given, in batches of consecutive rows for the same table, store, and
//! column set: `INSERT INTO t (cols) SELECT cols FROM jsonb_populate_recordset(NULL::t, $1)`.
//! Postgres converts each JSON value to the column's type, so the loader needs no per-table
//! code. Every batch first sets `app.tenant_id`, since every store table forces row-level
//! security. The whole load is one transaction: a store is either fully seeded or not at all.

use std::collections::BTreeMap;

use serde_json::Value;
use sqlx::PgPool;

use crate::SeedRows;

/// Rows per statement; keeps each JSON parameter well under a few megabytes.
const BATCH: usize = 1000;

/// The schemas that hold store data (`migrations/`).
pub const SCHEMAS: &[&str] = &[
    "platform",
    "catalog",
    "merchandising",
    "promotion",
    "customer",
    "sales",
    "fulfillment",
    "messaging",
    "search",
    "analytics",
    "agent",
];

fn word(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn table_name(t: &str) -> Result<&str, String> {
    match t.split_once('.') {
        Some((schema, table)) if word(schema) && word(table) => Ok(t),
        _ => Err(format!("`{t}` is not a schema.table name")),
    }
}

/// The store a row belongs to: `tenant_id`, or `id` for `platform.online_store`.
fn tenant_of<'a>(table: &str, row: &'a Value) -> Result<&'a str, String> {
    let key = if table == "platform.online_store" {
        "id"
    } else {
        "tenant_id"
    };
    row.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{table}: a row has no `{key}`"))
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// Rows written per table, in table-name order.
    pub tables: BTreeMap<String, usize>,
}

impl Report {
    pub fn total(&self) -> usize {
        self.tables.values().sum()
    }
}

/// Whether the store already exists (the seed job then does nothing).
///
/// `platform.online_store` forces row-level security, so the check runs with `app.tenant_id` set
/// to the store; without it the row is invisible even to the owner role, the store looks missing,
/// and a second load fails on the primary key.
pub async fn is_seeded(pool: &PgPool, tenant: &str) -> Result<bool, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
        .bind(tenant)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let seeded =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM platform.online_store WHERE id = $1)")
            .bind(tenant)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    tx.rollback().await.map_err(|e| e.to_string())?;
    Ok(seeded)
}

/// Empties every store table (`TRUNCATE … CASCADE`). Needs the owner role.
pub async fn reset(pool: &PgPool) -> Result<usize, String> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT quote_ident(table_schema) || '.' || quote_ident(table_name)
         FROM information_schema.tables
         WHERE table_type = 'BASE TABLE' AND table_schema = ANY($1)",
    )
    .bind(SCHEMAS)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    if tables.is_empty() {
        return Ok(0);
    }
    // Names come from the catalog, quoted by Postgres.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "TRUNCATE {} CASCADE",
        tables.join(", ")
    )))
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(tables.len())
}

/// Writes `rows` in one transaction.
pub async fn load(pool: &PgPool, rows: &SeedRows) -> Result<Report, String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let mut report = Report::default();
    let mut i = 0;
    while i < rows.len() {
        let (table, first) = &rows[i];
        let table = table_name(table)?;
        let tenant = tenant_of(table, first)?;
        let cols: Vec<&String> = first
            .as_object()
            .ok_or_else(|| format!("{table}: a row is not a map"))?
            .keys()
            .collect();
        if let Some(bad) = cols.iter().find(|c| !word(c)) {
            return Err(format!("{table}: `{bad}` is not a column name"));
        }
        // The run of rows that share table, store, and columns.
        let mut j = i + 1;
        while j < rows.len() && j - i < BATCH {
            let (t, r) = &rows[j];
            let same = t == table
                && tenant_of(t, r).ok() == Some(tenant)
                && r.as_object().is_some_and(|o| {
                    o.len() == cols.len() && cols.iter().all(|c| o.contains_key(*c))
                });
            if !same {
                break;
            }
            j += 1;
        }
        let batch: Vec<&Value> = rows[i..j].iter().map(|(_, r)| r).collect();
        sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
            .bind(tenant)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        let list = cols
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        // `table` and every column were checked to be plain lowercase words.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} ({list}) SELECT {list} FROM jsonb_populate_recordset(NULL::{table}, $1)"
        )))
        .bind(serde_json::to_value(&batch).map_err(|e| e.to_string())?)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("{table} (rows {}–{}): {e}", i + 1, j))?;
        *report.tables.entry(table.to_owned()).or_default() += j - i;
        i = j;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_and_tenants_are_checked() {
        assert!(table_name("catalog.product").is_ok());
        assert!(table_name("catalog.product; drop").is_err());
        assert!(table_name("product").is_err());
        assert_eq!(
            tenant_of("platform.online_store", &json!({"id": "store_a"})).unwrap(),
            "store_a"
        );
        assert!(tenant_of("catalog.brand", &json!({"id": "brn_a"})).is_err());
    }
}
