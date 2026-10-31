//! `get_lexicon` (docs/api/search.md#lexicon): a language's newest dictionary, and whether the
//! index in use already has it.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{LEXICON_NOT_FOUND, current, in_use};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetLexiconInput {
    /// The language, as a two- or three-letter code.
    #[schemars(regex(pattern = r"^[a-z]{2,3}$"), example = &"en")]
    pub in_language: String,
}

pub struct GetLexicon;

impl Operation for GetLexicon {
    const META: OpMeta = OpMeta {
        address: "search.lexicon.get",
        tool: "get_lexicon",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/search/lexicon",
        summary: "The store's search dictionary for a language (synonyms, new words, boost and exclude rules) with its analyzer version, and whether the index in use already has it.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[LEXICON_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetLexiconInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetLexicon {
    async fn read(&self, cx: &Cx, input: GetLexiconInput) -> Result<Value, ToolError> {
        let lang = input.in_language;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let newest = current(&mut tx, &cx.policy, &lang).await?;
        let used = in_use(&mut tx, &cx.policy, &lang).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let Some(l) = newest else {
            return Err(ToolError::new(
                LEXICON_NOT_FOUND,
                format!("The store has no search dictionary for `{lang}`."),
            )
            .detail(json!({"entity": "lexicon", "in_language": lang}))
            .next("update_lexicon"));
        };
        let in_use_version = used.map(|u| u.analyzer_version);
        let mut out = l.json();
        out["saved_at"] = json!(l.saved_at.map(|t| cx.time.format(t)));
        out["in_use_version"] = json!(in_use_version);
        out["reindex_pending"] = json!(in_use_version.as_deref() != Some(&l.analyzer_version));
        Ok(out)
    }
}
