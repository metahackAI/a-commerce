//! The `index` resource (docs/api/search.md#index): `reindex` and `get_index_status`, one file
//! each, and the store's search index itself.
//!
//! The index is PostgreSQL's own: one search document per product (`search.search_document`,
//! weighted full text and filter columns) and a record of every rebuild (`search.reindex_job`).
//! There is no outside engine. A rebuild runs in one transaction, so searches keep reading the
//! old documents until the new ones are all written and committed (the alias swap of
//! `index.reindex: blue_green_alias_swap`). The index in use is the one the latest completed job
//! built, with that job's dictionary version.
//!
//! Search reads a product's document only while it is up to date (built from the product's
//! current version); products without one, or whose document is out of date, are ranked from the
//! catalog text in the same query, so a stale index never hides a product.

use std::collections::{BTreeMap, BTreeSet};

use acom_catalog::service::for_search::{self, IndexFacts};
use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_policy::PolicySet;
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::model;
use crate::repo::db;

mod get_index_status;
mod reindex;

pub use get_index_status::*;
pub use reindex::*;

pub const REINDEX_IN_PROGRESS: ErrorCode =
    ErrorCode::new("reindex_in_progress", ErrorClass::InvalidState);
pub const ANALYZER_VERSION_NOT_FOUND: ErrorCode =
    ErrorCode::new("analyzer_version_not_found", ErrorClass::NotFound);
pub const SWAP_FAILED: ErrorCode = ErrorCode::new("swap_failed", ErrorClass::UpstreamUnavailable);
pub const JOB_NOT_FOUND: ErrorCode = ErrorCode::new("job_not_found", ErrorClass::NotFound);

/// One rebuild of the index.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct JobRow {
    pub id: String,
    pub reindex_mode: String,
    pub in_language: String,
    pub analyzer_version: String,
    pub name: String,
    pub product_group_ids: Option<Vec<String>>,
    pub changed_since: Option<DateTime<Utc>>,
    pub action_status: String,
    pub document_count: i32,
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub agent_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
}

const JOB_SELECT: &str = "SELECT id, reindex_mode, in_language, analyzer_version, name,
    product_group_ids, changed_since, action_status, document_count, result, error, agent_id,
    start_time, end_time FROM search.reindex_job WHERE tenant_id = $1";

pub async fn job(tx: &mut TxCx, id: &str) -> Result<Option<JobRow>, ToolError> {
    sqlx::query_as::<_, JobRow>(sqlx::AssertSqlSafe(format!("{JOB_SELECT} AND id = $2")))
        .bind(tx.tenant.as_str())
        .bind(id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)
}

/// The newest job, whatever its outcome.
pub async fn latest_job(tx: &mut TxCx) -> Result<Option<JobRow>, ToolError> {
    sqlx::query_as::<_, JobRow>(sqlx::AssertSqlSafe(format!(
        "{JOB_SELECT} ORDER BY start_time DESC, id DESC LIMIT 1"
    )))
    .bind(tx.tenant.as_str())
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The job that built the index in use for a language: the newest completed one.
pub async fn active(tx: &mut TxCx, language: &str) -> Result<Option<JobRow>, ToolError> {
    sqlx::query_as::<_, JobRow>(sqlx::AssertSqlSafe(format!(
        "{JOB_SELECT} AND in_language = $2 AND action_status = 'completed'
         ORDER BY end_time DESC, id DESC LIMIT 1"
    )))
    .bind(tx.tenant.as_str())
    .bind(language)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Completed full rebuilds of a language since `from` (all of them when `None`).
pub async fn full_runs(
    tx: &mut TxCx,
    language: &str,
    from: Option<DateTime<Utc>>,
) -> Result<i64, ToolError> {
    sqlx::query_scalar(
        "SELECT count(*) FROM search.reindex_job
         WHERE tenant_id = $1 AND in_language = $2 AND reindex_mode = 'full'
           AND action_status = 'completed' AND ($3::timestamptz IS NULL OR start_time >= $3)",
    )
    .bind(tx.tenant.as_str())
    .bind(language)
    .bind(from)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Takes the store's reindex lock until the transaction ends (`index.distributed_lock`), or
/// `false` when another reindex holds it.
pub async fn lock(tx: &mut TxCx) -> Result<bool, ToolError> {
    sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtext('search.reindex'), hashtext($1))")
        .bind(tx.tenant.as_str())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)
}

/// An index name after `search.yml index.naming` (`{type}-{tenant}-{lang}-{version}`).
pub fn index_name(policy: &PolicySet, tenant: &str, language: &str, n: i64) -> String {
    policy
        .lookup("search.index.naming")
        .and_then(Value::as_str)
        .unwrap_or("{type}-{tenant}-{lang}-{version}")
        .replace("{type}", "products")
        .replace("{tenant}", tenant)
        .replace("{lang}", language)
        .replace("{version}", &format!("v{n}"))
}

/// The stable name searches read through: the index name without its version.
pub fn alias_name(policy: &PolicySet, tenant: &str, language: &str) -> String {
    index_name(policy, tenant, language, 0)
        .trim_end_matches("-v0")
        .to_owned()
}

/// Every product's document version, by product.
pub async fn document_versions(tx: &mut TxCx) -> Result<BTreeMap<String, i32>, ToolError> {
    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT product_group_id, source_version FROM search.search_document WHERE tenant_id = $1",
    )
    .bind(tx.tenant.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().collect())
}

/// The products whose document is missing or built from an older version of the product.
pub fn out_of_date(facts: &[IndexFacts], documents: &BTreeMap<String, i32>) -> Vec<String> {
    facts
        .iter()
        .filter(|f| documents.get(&f.id) != Some(&f.version))
        .map(|f| f.id.clone())
        .collect()
}

/// Writes (or rewrites) the documents of these products from the catalog, with the weights
/// search ranks by (name A; brand, categories, tags B; material C; description D). Returns how
/// many it wrote.
pub async fn write_documents(tx: &mut TxCx, facts: &[IndexFacts]) -> Result<usize, ToolError> {
    if facts.is_empty() {
        return Ok(0);
    }
    let ids: Vec<String> = facts.iter().map(|f| f.id.clone()).collect();
    let wanted: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    let texts: Vec<_> = for_search::product_texts(tx, false)
        .await?
        .into_iter()
        .filter(|t| wanted.contains(t.id.as_str()))
        .collect();
    let links = for_search::categories(tx, Some(&ids)).await?;
    let docs: BTreeMap<String, model::Doc> = model::docs(&texts, &links)
        .into_iter()
        .map(|d| (d.id.clone(), d))
        .collect();
    let mut categories: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for l in &links {
        categories
            .entry(l.product_id.as_str())
            .or_default()
            .insert(l.category_id.as_str());
    }
    let rows: Vec<Value> = facts
        .iter()
        .filter_map(|f| {
            let d = docs.get(&f.id)?;
            Some(json!({
                "id": f.id, "name": d.name, "labels": d.labels, "material": d.material,
                "description": d.description, "brand_id": f.brand_id,
                "category_ids": categories.get(f.id.as_str()).map(|c| c.iter().collect::<Vec<_>>()).unwrap_or_default(),
                "keywords": f.keywords, "sizes": f.available_sizes, "colors": f.available_colors,
                "price_min": f.price_min, "price_max": f.price_max,
                "price_currency": f.price_currency, "approval_status": f.approval_status,
                "source_version": f.version,
            }))
        })
        .collect();
    let now = tx.clock.now();
    sqlx::query(
        "INSERT INTO search.search_document (tenant_id, product_group_id, search_vector, brand_id,
           category_ids, keywords, available_sizes, available_colors, price_min, price_max,
           price_currency, approval_status, source_version, indexed_at)
         SELECT $1, r.id,
                setweight(to_tsvector('english', r.name), 'A')
                || setweight(to_tsvector('english', r.labels), 'B')
                || setweight(to_tsvector('english', r.material), 'C')
                || setweight(to_tsvector('english', r.description), 'D'),
                r.brand_id,
                ARRAY(SELECT jsonb_array_elements_text(r.category_ids)),
                ARRAY(SELECT jsonb_array_elements_text(r.keywords)),
                ARRAY(SELECT jsonb_array_elements_text(r.sizes)),
                ARRAY(SELECT jsonb_array_elements_text(r.colors)),
                r.price_min, r.price_max, r.price_currency, r.approval_status, r.source_version, $3
         FROM jsonb_to_recordset($2) AS r(id text, name text, labels text, material text,
              description text, brand_id text, category_ids jsonb, keywords jsonb, sizes jsonb,
              colors jsonb, price_min numeric, price_max numeric, price_currency text,
              approval_status text, source_version integer)
         ON CONFLICT (tenant_id, product_group_id) DO UPDATE SET
           search_vector = EXCLUDED.search_vector, brand_id = EXCLUDED.brand_id,
           category_ids = EXCLUDED.category_ids, keywords = EXCLUDED.keywords,
           available_sizes = EXCLUDED.available_sizes, available_colors = EXCLUDED.available_colors,
           price_min = EXCLUDED.price_min, price_max = EXCLUDED.price_max,
           price_currency = EXCLUDED.price_currency, approval_status = EXCLUDED.approval_status,
           source_version = EXCLUDED.source_version, indexed_at = EXCLUDED.indexed_at,
           updated_at = now(), version = search.search_document.version + 1",
    )
    .bind(tx.tenant.as_str())
    .bind(Value::Array(rows.clone()))
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "writing search documents failed");
        ToolError::new(
            SWAP_FAILED,
            "The new search documents could not be written; search keeps the index it had.",
        )
        .detail(json!({"upstream": "search_index", "retryable": true}))
        .next("get_index_status")
    })?;
    Ok(rows.len())
}

/// A finished job, as it is recorded.
pub struct NewJob<'a> {
    pub id: &'a str,
    pub mode: &'a str,
    pub language: &'a str,
    pub analyzer_version: &'a str,
    pub name: &'a str,
    pub product_ids: Option<&'a [String]>,
    pub since: Option<DateTime<Utc>>,
    pub documents: usize,
    pub result: Value,
}

pub async fn record_job(tx: &mut TxCx, j: &NewJob<'_>) -> Result<(), ToolError> {
    let now = tx.clock.now();
    let agent = tx.principal.subject.clone();
    sqlx::query(
        "INSERT INTO search.reindex_job (tenant_id, id, reindex_mode, in_language,
           analyzer_version, name, product_group_ids, changed_since, action_status,
           document_count, result, agent_id, start_time, end_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'completed', $9, $10, $11, $12, $12)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(j.id)
    .bind(j.mode)
    .bind(j.language)
    .bind(j.analyzer_version)
    .bind(j.name)
    .bind(j.product_ids)
    .bind(j.since)
    .bind(i32::try_from(j.documents).unwrap_or(i32::MAX))
    .bind(&j.result)
    .bind(&agent)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// A job as the API shows it.
pub fn job_json(time: &acom_core::StoreTime, j: &JobRow) -> Value {
    json!({
        "id": j.id,
        "mode": j.reindex_mode,
        "action_status": j.action_status,
        "progress": match j.action_status.as_str() {
            "completed" => json!(1.0),
            "failed" => Value::Null,
            _ => json!(0.0),
        },
        "analyzer_version": j.analyzer_version,
        "index": j.name,
        "documents": j.document_count,
        "product_ids": j.product_group_ids,
        "since": j.changed_since.map(|t| time.format(t)),
        "start_time": time.format(j.start_time),
        "end_time": j.end_time.map(|t| time.format(t)),
        "result": j.result,
        "error": j.error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::policies;

    use acom_contract::{ApprovalNeed, ReadOp, WriteOp};
    use acom_core::enums::ActorKind;
    use serde_json::json;

    use crate::lexicon::{GetLexicon, UpdateLexicon};
    use crate::testing;

    fn input<T: serde::de::DeserializeOwned>(v: Value) -> T {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn dictionaries_and_rebuilds_are_refused_before_anything_is_read() {
        let cx = testing::cx(ActorKind::Operator, "adm_sj");
        let e = UpdateLexicon
            .preview(&cx, input(json!({"in_language": "ko", "synonyms": []})))
            .await
            .unwrap_err();
        assert_eq!(e.code, "unsupported_language");
        assert_eq!(e.next_tool.as_deref(), Some("get_lexicon"));
        let e = UpdateLexicon
            .preview(
                &cx,
                input(json!({"in_language": "en", "synonyms": [
                    {"terms": ["anorak", "parka"], "bidirectional": false},
                    {"terms": ["parka", "anorak"], "bidirectional": false}]})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "circular_synonym");
        let e = UpdateLexicon
            .preview(
                &cx,
                input(json!({"in_language": "en", "synonyms": [{"terms": ["coat", "COAT"]}]})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["rule"], "lexicon.no_self_reference");
        let e = Reindex
            .preview(
                &cx,
                input(json!({"mode": "full", "product_ids": ["prd_windbreaker"]})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        assert!(serde_json::from_value::<ReindexInput>(json!({"mode": "nightly"})).is_err());
    }

    /// Integration: needs the seeded demo store (`DATABASE_URL`). Everything it writes is rolled
    /// back, so the store keeps reading the catalog text.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn a_new_dictionary_takes_effect_with_the_reindex_that_uses_it() {
        let cx = testing::db(ActorKind::Operator, "adm_sj").await;
        let status = GetIndexStatus.read(&cx, input(json!({}))).await.unwrap();
        assert_eq!(status["serving"], "catalog_text");
        assert_eq!(status["analyzer_version"], "v1");
        assert!(status["active_index"].is_null());
        let products = status["products"].as_i64().unwrap();
        assert!(products >= 18, "{status}");
        let lexicon = GetLexicon
            .read(&cx, input(json!({"in_language": "en"})))
            .await
            .unwrap();
        assert_eq!(lexicon["analyzer_version"], "v1");
        assert_eq!(lexicon["source"], "policy_default");
        assert_eq!(lexicon["reindex_pending"], false);
        let e = GetLexicon
            .read(&cx, input(json!({"in_language": "ko"})))
            .await
            .unwrap_err();
        assert_eq!(e.code, "lexicon_not_found");
        let e = GetIndexStatus
            .read(&cx, input(json!({"reindex_job_id": "rij_nope"})))
            .await
            .unwrap_err();
        assert_eq!(e.code, "job_not_found");

        let full = Reindex
            .preview(&cx, input(json!({"mode": "full"})))
            .await
            .unwrap();
        assert_eq!(full.approval, ApprovalNeed::NotNeeded);
        assert_eq!(full.changes[0].to["documents"], products);
        assert_eq!(
            full.changes[0].to["index"],
            "products-store_northwind-en-v1"
        );
        assert!(full.checks[0].passed);
        assert_eq!(
            full.checks[1].detail.as_deref(),
            Some("1 of 2 full reindexes today")
        );

        let update = UpdateLexicon
            .preview(
                &cx,
                input(json!({"in_language": "en",
                    "synonyms": [{"terms": ["jacket", "coat", "parka"]},
                                 {"terms": ["windbreaker", "wind jacket"], "bidirectional": false}],
                    "boost_rules": [{"term": "fur", "effect": "exclude"}]})),
            )
            .await
            .unwrap();
        assert_eq!(update.changes[0].to["analyzer_version"], "v2");
        assert_eq!(update.checks[2].fix_tool.as_deref(), Some("reindex"));

        let mut tx = cx.begin().await.unwrap();
        let policy = cx.policy.clone();
        UpdateLexicon.commit(&mut tx, &update.plan).await.unwrap();
        let newest = crate::lexicon::current(&mut tx, &policy, "en")
            .await
            .unwrap();
        assert_eq!(newest.unwrap().analyzer_version, "v2");
        let used = crate::lexicon::in_use(&mut tx, &policy, "en")
            .await
            .unwrap();
        assert_eq!(used.unwrap().analyzer_version, "v1");
        // Before the reindex, the excluded word still finds products.
        let before = crate::product::text_ranks(&cx, &mut tx, Some("fur jacket"), false)
            .await
            .unwrap()
            .unwrap();
        assert!(!before.is_empty());

        let plan = ReindexPlan {
            analyzer_version: "v2".into(),
            ..full.plan.clone()
        };
        let done = Reindex.commit(&mut tx, &plan).await.unwrap();
        assert_eq!(Reindex.commit(&mut tx, &plan).await.unwrap(), done);
        let j = job(&mut tx, &done.id).await.unwrap().unwrap();
        assert_eq!(j.action_status, "completed");
        assert_eq!(j.name, "products-store_northwind-en-v1");
        assert_eq!(i64::from(j.document_count), products);
        let docs = document_versions(&mut tx).await.unwrap();
        assert_eq!(i64::try_from(docs.len()).unwrap(), products);
        let used = crate::lexicon::in_use(&mut tx, &policy, "en")
            .await
            .unwrap();
        assert_eq!(used.unwrap().analyzer_version, "v2");
        // The excluded word now finds nothing; a jacket query still ranks from the documents.
        let after = crate::product::text_ranks(&cx, &mut tx, Some("fur jacket"), false)
            .await
            .unwrap()
            .unwrap();
        assert!(after.is_empty());
        let jackets = crate::product::text_ranks(&cx, &mut tx, Some("windbreaker"), false)
            .await
            .unwrap()
            .unwrap();
        assert!(
            jackets.iter().any(|(id, _)| id == "prd_windbreaker"),
            "{jackets:?}"
        );
        // A version the index uses cannot be undone.
        let e = UpdateLexicon
            .undo(
                &mut tx,
                &update.plan,
                &acom_contract::Committed {
                    id: "v2".into(),
                    produced: Default::default(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "not_revocable");
        // Rolled back on drop.
    }

    #[test]
    fn index_names_follow_the_policy() {
        let p = policies();
        assert_eq!(
            index_name(&p, "store_northwind", "en", 42),
            "products-store_northwind-en-v42"
        );
        assert_eq!(
            alias_name(&p, "store_northwind", "en"),
            "products-store_northwind-en"
        );
    }
}
