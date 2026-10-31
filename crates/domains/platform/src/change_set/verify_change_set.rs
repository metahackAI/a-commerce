//! `verify_change_set` (`platform.change_set.verify`): what the runner found when it read a set
//! back.

use acom_changeset::{CHANGE_SET_NOT_FOUND, CHANGE_SET_NOT_RUN, read};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerifyChangeSetInput {
    /// The change set whose read-back to show.
    #[schemars(regex(pattern = r"^cs_[a-z0-9_]+$"), example = &"cs_1042")]
    pub change_set_id: String,
}

pub struct VerifyChangeSet;

impl Operation for VerifyChangeSet {
    const META: OpMeta = OpMeta {
        address: "platform.change_set.verify",
        tool: "verify_change_set",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/change-sets/{change_set_id}/verification",
        summary: "What the runner found when it read every change back after running a set: matched or not, per item. Changes nothing; safe to call again.",
        consumes: &[Slot::one("change_set_id")],
        produces: &[],
        checks: &[],
        errors: &[CHANGE_SET_NOT_FOUND, CHANGE_SET_NOT_RUN],
        audience: Audience::Operator,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = VerifyChangeSetInput;
    type Output = Value;
}

impl ReadOp<Cx> for VerifyChangeSet {
    async fn read(&self, cx: &Cx, input: VerifyChangeSetInput) -> Result<Value, ToolError> {
        read::verification(cx, &input.change_set_id).await
    }
}
