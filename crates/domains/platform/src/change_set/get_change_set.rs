//! `get_change_set` (`platform.change_set.get`): one change set expanded.

use acom_changeset::{CHANGE_SET_NOT_FOUND, read};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetChangeSetInput {
    /// The change set.
    #[schemars(regex(pattern = r"^cs_[a-z0-9_]+$"), example = &"cs_1042")]
    pub change_set_id: String,
    /// Top-level fields to return, for example only the state and version.
    #[schemars(example = ["state", "version", "undo_until"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetChangeSet;

impl Operation for GetChangeSet {
    const META: OpMeta = OpMeta {
        address: "platform.change_set.get",
        tool: "get_change_set",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/change-sets/{change_set_id}",
        summary: "One change set expanded: state, version, request and interpretation, each item's changes, checks and approval need, what was left out, and the undo deadline. Read it before editing a set by talking.",
        consumes: &[Slot::one("change_set_id")],
        produces: &[Slot::many("preview_id")],
        checks: &[],
        errors: &[CHANGE_SET_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetChangeSetInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetChangeSet {
    async fn read(&self, cx: &Cx, input: GetChangeSetInput) -> Result<Value, ToolError> {
        read::get(cx, &input.change_set_id, &input.fields.unwrap_or_default()).await
    }
}
