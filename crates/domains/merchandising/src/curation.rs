//! `list_curations`, `get_curation` (docs/api/merchandising.md#collection_page).
//!
//! Curated pages (`CollectionPage`, `col_`) have no table yet (`migrations/0006_merchandising.sql`
//! stops at categories, slots, banners and bookings), so the store has none: the list is empty
//! and every id is unknown. The reads keep the spec's inputs and shapes so callers stay the
//! same when the table arrives.

use acom_contract::{
    ActionType, Audience, ErrorClass, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp,
    Slot, ToolError,
};
use acom_core::enums::DisplayStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{check_id, parse_time};

pub const COLLECTION_PAGE_NOT_FOUND: ErrorCode =
    ErrorCode::new("collection_page_not_found", ErrorClass::NotFound);

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
