//! `reindex` (docs/api/search.md#index): rebuilds the store's search documents, every one
//! (`full`) or only what changed (`incremental`), with a dictionary version. One at a time per
//! store; the documents switch over when it commits.

use std::collections::BTreeSet;

use acom_catalog::service::for_search::{self, IndexFacts};
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::ReindexMode;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    ANALYZER_VERSION_NOT_FOUND, NewJob, REINDEX_IN_PROGRESS, SWAP_FAILED, active,
    document_versions, full_runs, index_name, job, lock, out_of_date, record_job, write_documents,
};
use crate::lexicon::{current, index_language, version};

acom_core::define_id!(
    /// A reindex job (`search.reindex_job`).
    ReindexJobId,
    "rij"
);

/// Vectors to make again; no embedding model is connected, so they are left as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Embeddings {
    /// Make the text vectors again. Default false.
    #[schemars(example = false)]
    pub text: Option<bool>,
    /// Make the image vectors again. Default false.
    #[schemars(example = false)]
    pub image: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReindexInput {
    /// full: rebuild every document; incremental: only the products named, changed since a
    /// time, or with an out-of-date document.
    #[schemars(example = &"full")]
    pub mode: ReindexMode,
    /// incremental only: just these products.
    #[schemars(inner(regex(pattern = r"^prd_[a-z0-9_]+$")), length(min = 1, max = 1000), example = ["prd_windbreaker"])]
    pub product_ids: Option<Vec<String>>,
    /// incremental only: the products changed at or after this time (store time).
    #[schemars(example = &"2026-11-06T00:00")]
    pub since: Option<String>,
    /// The dictionary version to use (get_lexicon). Default the newest.
    #[schemars(regex(pattern = r"^v[0-9]+$"), example = &"v2")]
    pub analyzer_version: Option<String>,
    /// Vectors to make again.
    #[schemars(example = json!({"text": false, "image": false}))]
    pub embeddings: Option<Embeddings>,
}

/// The rebuild as previewed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReindexPlan {
    pub job_id: String,
    pub mode: String,
    pub language: String,
    pub analyzer_version: String,
    pub product_ids: Option<Vec<String>>,
    pub since: Option<DateTime<Utc>>,
    pub embeddings: Embeddings,
}

pub struct Reindex;

impl Operation for Reindex {
    const META: OpMeta = OpMeta {
        address: "search.index.reindex",
        tool: "reindex",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/search/index/reindex",
        summary: "Rebuild the store's search documents, all of them or only what changed, with a dictionary version; searches keep the old documents until the new ones are in. Needed after update_lexicon.",
        consumes: &[Slot::one("analyzer_version")],
        produces: &[Slot::one("reindex_job_id")],
        checks: &["index.distributed_lock", "index.resource_budget"],
        errors: &[
            REINDEX_IN_PROGRESS,
            ANALYZER_VERSION_NOT_FOUND,
            SWAP_FAILED,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = ReindexInput;
    type Output = Value;
}

/// The products a rebuild writes.
async fn scope(tx: &mut TxCx, plan: &ReindexPlan) -> Result<Vec<IndexFacts>, ToolError> {
    let facts = for_search::index_facts(tx, plan.product_ids.as_deref()).await?;
    if plan.mode == "full" || plan.product_ids.is_some() {
        return Ok(facts);
    }
    if let Some(since) = plan.since {
        return Ok(facts
            .into_iter()
            .filter(|f| f.updated_at >= since)
            .collect());
    }
    let docs = document_versions(tx).await?;
    let stale: BTreeSet<String> = out_of_date(&facts, &docs).into_iter().collect();
    Ok(facts
        .into_iter()
        .filter(|f| stale.contains(&f.id))
        .collect())
}

/// The start of the store day `now` falls on.
fn day_start(tx: &TxCx, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let zone = tx.time.zone;
    now.with_timezone(&zone)
        .date_naive()
        .and_hms_opt(0, 0, 0)?
        .and_local_timezone(zone)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

fn embeddings_note(e: Embeddings) -> Option<&'static str> {
    (e.text == Some(true) || e.image == Some(true)).then_some(
        "No embedding model is connected: vectors are left as they are and ranking uses the text.",
    )
}

impl WriteOp<Cx> for Reindex {
    type Plan = ReindexPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ReindexInput,
    ) -> Result<Planned<ReindexPlan>, ToolError> {
        let full = input.mode == ReindexMode::Full;
        if full && (input.product_ids.is_some() || input.since.is_some()) {
            return Err(ToolError::invalid_input(
                "A full reindex rebuilds every product; product_ids and since are for incremental.",
            )
            .detail(json!({"field": "mode", "reason": "enum"}))
            .next("reindex"));
        }
        let since = crate::parse_time(cx, "since", input.since.as_deref())?;
        let language = index_language(&cx.policy);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let lexicon = match &input.analyzer_version {
            Some(v) => version(&mut tx, &cx.policy, &language, v)
                .await?
                .ok_or_else(|| {
                    ToolError::new(
                        ANALYZER_VERSION_NOT_FOUND,
                        format!("The {language} dictionary has no version {v}."),
                    )
                    .detail(json!({"entity": "lexicon", "analyzer_version": v}))
                    .next("get_lexicon")
                })?,
            None => current(&mut tx, &cx.policy, &language)
                .await?
                .ok_or_else(|| {
                    ToolError::new(
                        ANALYZER_VERSION_NOT_FOUND,
                        format!("The store has no {language} dictionary to index with."),
                    )
                    .detail(json!({"entity": "lexicon", "in_language": language}))
                    .next("get_lexicon")
                })?,
        };
        let plan = ReindexPlan {
            job_id: ReindexJobId::generate().to_string(),
            mode: input.mode.as_str().to_owned(),
            language: language.clone(),
            analyzer_version: lexicon.analyzer_version.clone(),
            product_ids: input.product_ids.map(|mut ids| {
                ids.sort();
                ids.dedup();
                ids
            }),
            since,
            embeddings: input.embeddings.unwrap_or_default(),
        };
        let products = scope(&mut tx, &plan).await?;
        if let Some(ids) = &plan.product_ids {
            let found: BTreeSet<&str> = products.iter().map(|f| f.id.as_str()).collect();
            if let Some(missing) = ids.iter().find(|id| !found.contains(id.as_str())) {
                return Err(ToolError::new(
                    ErrorCode::NOT_FOUND,
                    format!("There is no product {missing}."),
                )
                .detail(json!({"entity": "product", "id": missing}))
                .next("search_products"));
            }
        }
        let free = lock(&mut tx).await?;
        let now = cx.clock.now();
        let start = day_start(&tx, now);
        let today = full_runs(&mut tx, &language, start).await?;
        let built = full_runs(&mut tx, &language, None).await?;
        let in_use = active(&mut tx, &language).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let max = cx
            .policy
            .lookup("search.index.resource_budget.full_reindex_max_per_day")
            .and_then(Value::as_i64);
        let mut checks = vec![Check {
            rule: "index.distributed_lock".into(),
            passed: free,
            detail: Some(if free {
                "No other reindex is running".into()
            } else {
                "Another reindex of this store is running".into()
            }),
            fix_tool: (!free).then(|| "get_index_status".into()),
        }];
        checks.push(match (full, max) {
            (true, Some(max)) => Check {
                rule: "index.resource_budget".into(),
                passed: today < max,
                detail: Some(format!("{} of {max} full reindexes today", today + 1)),
                fix_tool: (today >= max).then(|| "get_index_status".into()),
            },
            _ => Check {
                rule: "index.resource_budget".into(),
                passed: true,
                detail: Some(
                    "Incremental runs do not count toward the daily full reindexes".into(),
                ),
                fix_tool: None,
            },
        });
        let tenant = cx.tenant.as_str();
        let name = if full || in_use.is_none() {
            index_name(&cx.policy, tenant, &language, built + 1)
        } else {
            in_use.map(|j| j.name).unwrap_or_default()
        };
        let mut to = json!({
            "mode": plan.mode,
            "index": name,
            "in_language": language,
            "analyzer_version": plan.analyzer_version,
            "documents": products.len(),
        });
        if let Some(note) = embeddings_note(plan.embeddings) {
            to["embeddings"] = json!(note);
        }
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan,
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "reindex_job".into(),
                id: String::new(),
                from: Value::Null,
                to,
                note: Some(
                    "Runs in one step; searches use the old documents until it is done".into(),
                ),
            }],
            checks,
            basis: Vec::new(),
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Rebuilds the search index".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ReindexPlan) -> Result<Committed, ToolError> {
        let done = Committed {
            id: plan.job_id.clone(),
            produced: Map::from_iter([("reindex_job_id".to_owned(), json!(plan.job_id))]),
        };
        if job(tx, &plan.job_id).await?.is_some() {
            return Ok(done);
        }
        if !lock(tx).await? {
            return Err(ToolError::new(
                REINDEX_IN_PROGRESS,
                "Another reindex of this store is running; check it before starting one.",
            )
            .detail(json!({"entity": "reindex_job"}))
            .next("get_index_status"));
        }
        let policy = tx.policy.clone();
        if version(tx, &policy, &plan.language, &plan.analyzer_version)
            .await?
            .is_none()
        {
            return Err(ToolError::new(
                ANALYZER_VERSION_NOT_FOUND,
                format!("The dictionary version {} is gone.", plan.analyzer_version),
            )
            .next("get_lexicon"));
        }
        let products = scope(tx, plan).await?;
        let written = write_documents(tx, &products).await?;
        let tenant = tx.tenant.as_str().to_owned();
        let name = if plan.mode == "full" {
            let built = full_runs(tx, &plan.language, None).await?;
            index_name(&policy, &tenant, &plan.language, built + 1)
        } else {
            match active(tx, &plan.language).await? {
                Some(j) => j.name,
                None => index_name(&policy, &tenant, &plan.language, 1),
            }
        };
        let mut result = json!({"documents": written});
        if let Some(note) = embeddings_note(plan.embeddings) {
            result["embeddings"] = json!(note);
        }
        record_job(
            tx,
            &NewJob {
                id: &plan.job_id,
                mode: &plan.mode,
                language: &plan.language,
                analyzer_version: &plan.analyzer_version,
                name: &name,
                product_ids: plan.product_ids.as_deref(),
                since: plan.since,
                documents: written,
                result,
            },
        )
        .await?;
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ReindexPlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let j = job(&mut tx, &done.id).await?;
        let facts = for_search::index_facts(&mut tx, plan.product_ids.as_deref()).await?;
        let docs = document_versions(&mut tx).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = match &j {
            None => vec![format!("reindex job {} is missing", done.id)],
            Some(j) if j.action_status != "completed" => {
                vec![format!("reindex job {} is {}", j.id, j.action_status)]
            }
            Some(_) => Vec::new(),
        };
        if plan.mode == "full" || plan.product_ids.is_some() {
            let stale = out_of_date(&facts, &docs);
            // A product changed after the job is out of date again; one the job wrote is not.
            let started = j.as_ref().map(|j| j.start_time);
            let late: Vec<&String> = stale
                .iter()
                .filter(|id| {
                    facts
                        .iter()
                        .find(|f| &f.id == *id)
                        .is_none_or(|f| started.is_none_or(|s| f.updated_at <= s))
                })
                .collect();
            if !late.is_empty() {
                mismatches.push(format!("{} documents are not up to date", late.len()));
            }
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, _: &mut TxCx, plan: &ReindexPlan, _: &Committed) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::new("not_revocable", acom_contract::ErrorClass::UndoWindowClosed),
            format!(
                "Reindex {} rebuilt documents from the catalog; run reindex again instead of undoing it.",
                plan.job_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeddings_are_not_made_without_a_model() {
        assert!(embeddings_note(Embeddings::default()).is_none());
        assert!(
            embeddings_note(Embeddings {
                text: Some(true),
                image: None
            })
            .is_some()
        );
    }
}
