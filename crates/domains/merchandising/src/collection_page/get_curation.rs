//! `get_curation` (docs/api/merchandising.md#collection_page).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::COLLECTION_PAGE_NOT_FOUND;
use crate::check_id;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCurationInput {
    /// The curated page.
    #[schemars(regex(pattern = r"^col_[a-z0-9_]+$"), example = &"col_winter_edit")]
    pub collection_page_id: String,
    /// Only these fields (`id` always comes back).
    #[schemars(example = ["name", "status"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetCuration;

impl Operation for GetCuration {
    const META: OpMeta = OpMeta {
        address: "merchandising.collection_page.get",
        tool: "get_curation",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/merchandising/collection-pages/{collection_page_id}",
        summary: "One curated page: sections and their products, linked banners, search keywords and its state.",
        consumes: &[Slot::one("collection_page_id")],
        produces: &[],
        checks: &[],
        errors: &[COLLECTION_PAGE_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCurationInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCuration {
    async fn read(&self, _cx: &Cx, input: GetCurationInput) -> Result<Value, ToolError> {
        check_id("collection_page_id", "col", &input.collection_page_id)?;
        // `fields` narrows a stored page; there is none to narrow yet.
        let _fields = input.fields;
        Err(ToolError::new(
            COLLECTION_PAGE_NOT_FOUND,
            format!("There is no curated page {}.", input.collection_page_id),
        )
        .detail(json!({"entity": "collection_page", "id": input.collection_page_id}))
        .next("list_curations"))
    }
}
