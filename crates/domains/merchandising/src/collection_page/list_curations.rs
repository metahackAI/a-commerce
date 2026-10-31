//! `list_curations` (docs/api/merchandising.md#collection_page).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::DisplayStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::parse_time;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListCurationsInput {
    /// Only pages in this state: draft, scheduled, active or ended.
    #[schemars(example = &"active")]
    pub status: Option<DisplayStatus>,
    /// Only pages for this audience: a tier id (`list_tiers`) or `everyone`.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"everyone")]
    pub audience: Option<String>,
    /// Judge the state at this moment (store time without an offset). Default: now.
    #[schemars(example = &"2026-11-07T10:00")]
    pub as_of: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListCurations;

impl Operation for ListCurations {
    const META: OpMeta = OpMeta {
        address: "merchandising.collection_page.list",
        tool: "list_curations",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/merchandising/collection-pages",
        summary: "Curated pages (collections of products in sections) with their state, period and audience.",
        consumes: &[],
        produces: &[Slot::many("collection_page_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListCurationsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListCurations {
    async fn read(&self, cx: &Cx, input: ListCurationsInput) -> Result<Value, ToolError> {
        Page::new(input.limit, input.cursor.as_deref())?;
        if let Some(s) = &input.as_of {
            parse_time(&cx.time, "as_of", s)?;
        }
        // No curated page is stored yet, whatever the filters.
        let _filters = (input.status, input.audience);
        Ok(json!({"items": [], "total": 0}))
    }
}
