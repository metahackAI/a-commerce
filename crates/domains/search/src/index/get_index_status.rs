//! `get_index_status` (docs/api/search.md#index): the index searches use, its dictionary
//! version, how many documents are out of date, and a reindex job.

use acom_catalog::service::for_search;
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    JOB_NOT_FOUND, active, alias_name, document_versions, job, job_json, latest_job, out_of_date,
};
use crate::lexicon::{current, in_use, index_language};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetIndexStatusInput {
    /// A reindex job to report on. Default the newest.
    #[schemars(regex(pattern = r"^rij_[a-z0-9_]+$"), example = &"rij_01jc8")]
    pub reindex_job_id: Option<String>,
}

pub struct GetIndexStatus;

impl Operation for GetIndexStatus {
    const META: OpMeta = OpMeta {
        address: "search.index.get_status",
        tool: "get_index_status",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/search/index/status",
        summary: "The search index in use with its dictionary version, how many products' documents are missing or out of date, and the state of a reindex job.",
        consumes: &[Slot::one("reindex_job_id")],
        produces: &[],
        checks: &[],
        errors: &[JOB_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetIndexStatusInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetIndexStatus {
    async fn read(&self, cx: &Cx, input: GetIndexStatusInput) -> Result<Value, ToolError> {
        let language = index_language(&cx.policy);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let wanted = match &input.reindex_job_id {
            Some(id) => Some(job(&mut tx, id).await?.ok_or_else(|| {
                ToolError::new(JOB_NOT_FOUND, format!("There is no reindex job {id}."))
                    .detail(json!({"entity": "reindex_job", "id": id}))
                    .next("reindex")
            })?),
            None => latest_job(&mut tx).await?,
        };
        let built = active(&mut tx, &language).await?;
        let used = in_use(&mut tx, &cx.policy, &language).await?;
        let newest = current(&mut tx, &cx.policy, &language).await?;
        let facts = for_search::index_facts(&mut tx, None).await?;
        let docs = document_versions(&mut tx).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let stale = out_of_date(&facts, &docs);
        let used_version = used.map(|l| l.analyzer_version);
        let newest_version = newest.map(|l| l.analyzer_version);
        let serving = if docs.is_empty() {
            "catalog_text"
        } else {
            "index"
        };
        Ok(json!({
            "active_alias": built.as_ref().map(|_| alias_name(&cx.policy, cx.tenant.as_str(), &language)),
            "active_index": built.as_ref().map(|j| j.name.clone()),
            "in_language": language,
            "analyzer_version": used_version,
            "latest_analyzer_version": newest_version,
            "reindex_pending": newest_version.is_some() && newest_version != used_version,
            "serving": serving,
            "products": facts.len(),
            "documents": docs.len(),
            "out_of_date": stale.len(),
            "note": if serving == "catalog_text" {
                "No index has been built: searches rank the catalog text directly."
            } else {
                "Products whose document is missing or out of date are ranked from the catalog text until the next reindex."
            },
            "job": wanted.map(|j| job_json(&cx.time, &j)),
        }))
    }
}
