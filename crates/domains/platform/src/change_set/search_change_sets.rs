//! `search_change_sets` (`platform.change_set.search`): change sets by state, creator and time.

use acom_changeset::read;
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::ChangeSetState;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::parse_time;

/// How to order change sets.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
pub enum ChangeSetSort {
    /// Newest first (the default).
    #[serde(rename = "-created_at")]
    NewestFirst,
    /// Oldest first.
    #[serde(rename = "created_at")]
    OldestFirst,
    /// Highest number first.
    #[serde(rename = "-number")]
    NumberDescending,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchChangeSetsInput {
    /// Only these states. All when left out.
    #[schemars(example = ["waiting_for_approval"])]
    pub state: Option<Vec<ChangeSetState>>,
    /// The operator or system actor who asked for the set.
    #[schemars(regex(pattern = r"^(adm|sys)_[a-z0-9_]+$"), example = &"adm_sj")]
    pub created_by: Option<String>,
    /// Created at or after this time (store time).
    #[schemars(example = &"2026-11-06T00:00")]
    pub created_from: Option<String>,
    /// Created at or before this time (store time).
    #[schemars(example = &"2026-11-06T23:59")]
    pub created_through: Option<String>,
    /// Only sets that can be undone right now (live and before their deadline).
    #[schemars(example = true)]
    pub undoable_only: Option<bool>,
    /// Order of the results.
    #[schemars(example = &"-created_at")]
    pub sort: Option<ChangeSetSort>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchChangeSets;

impl Operation for SearchChangeSets {
    const META: OpMeta = OpMeta {
        address: "platform.change_set.search",
        tool: "search_change_sets",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/change-sets",
        summary: "Find change sets by state, creator and time: what is waiting for approval, what ran yesterday, what can still be undone.",
        consumes: &[],
        produces: &[Slot::many("change_set_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchChangeSetsInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchChangeSets {
    async fn read(&self, cx: &Cx, input: SearchChangeSetsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let filter = read::Filter {
            states: input.state.unwrap_or_default(),
            created_by: input.created_by,
            created_from: parse_time(cx, "created_from", input.created_from.as_deref())?,
            created_through: parse_time(cx, "created_through", input.created_through.as_deref())?,
            undoable_only: input.undoable_only.unwrap_or(false),
            sort: match input.sort {
                Some(ChangeSetSort::OldestFirst) => "created_at",
                Some(ChangeSetSort::NumberDescending) => "-number",
                _ => "-created_at",
            }
            .to_owned(),
            limit: page.limit,
            offset: page.offset,
        };
        let mut out = read::search(cx, &filter).await?;
        // The engine pages by offset; give the caller our cursor format.
        let total = out["total"].as_i64().unwrap_or(0);
        let returned = out["items"].as_array().map_or(0, Vec::len);
        out["next_cursor"] = page
            .next_cursor(returned, total)
            .map_or(Value::Null, Value::String);
        if out["next_cursor"].is_null()
            && let Some(o) = out.as_object_mut()
        {
            o.remove("next_cursor");
        }
        Ok(out)
    }
}
