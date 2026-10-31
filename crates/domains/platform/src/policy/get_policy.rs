//! `get_policy` (`platform.policy.get`): the store's policy values under a key.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, ReadResult,
    Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{PolicyFile, display, has_path, leaves, overrides};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPolicyInput {
    /// A policy key or the start of one, dot-separated, without the file name. Returns every
    /// leaf value under it.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*(\.[a-z0-9_]+)*$"), example = &"max_discount")]
    pub key: String,
    /// Narrows to one file when the same key exists in several.
    #[schemars(example = &"promotion")]
    pub file: Option<PolicyFile>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PolicyValue {
    /// Full key path inside the file.
    pub key: String,
    /// The value as stored: ratios 0–1, money `{value, currency}`, conditions `{field, op, value}`.
    pub value: Value,
    /// The value as people read it (`30%`, `$200.00`).
    pub display: String,
    /// The policy file the value is in.
    pub file: String,
    /// `true` when the store's own override sets it.
    pub overridden_by_store: bool,
}

pub struct GetPolicy;

impl Operation for GetPolicy {
    const META: OpMeta = OpMeta {
        address: "platform.policy.get",
        tool: "get_policy",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/policies",
        summary: "The store's policy values under a key (defaults merged with the store's overrides), with a readable display for each. Use before planning a change; preview checks cite the same keys.",
        consumes: &[Slot::one("policy_key")],
        produces: &[Slot::many("policy_key")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPolicyInput;
    type Output = ReadResult<PolicyValue>;
}

impl ReadOp<Cx> for GetPolicy {
    async fn read(
        &self,
        cx: &Cx,
        input: GetPolicyInput,
    ) -> Result<ReadResult<PolicyValue>, ToolError> {
        let ov = overrides(cx).await?;
        let files: Vec<PolicyFile> = match input.file {
            Some(f) => vec![f],
            None => PolicyFile::ALL.to_vec(),
        };
        let mut items = Vec::new();
        for f in files {
            let Some(doc) = cx.policy.raw(f.name()) else {
                continue;
            };
            let mut all = Vec::new();
            leaves(doc, "", &mut all);
            for (key, value) in all {
                let under = key == input.key || key.starts_with(&format!("{}.", input.key));
                if !under || key == "version" {
                    continue;
                }
                let overridden = ov.get(f.name()).is_some_and(|p| has_path(p, &key));
                items.push(PolicyValue {
                    display: display(&key, &value),
                    key,
                    value,
                    file: f.name().to_owned(),
                    overridden_by_store: overridden,
                });
            }
        }
        if items.is_empty() {
            return Err(ToolError::new(
                ErrorCode::NOT_FOUND,
                format!("No policy key starts with `{}`.", input.key),
            )
            .detail(json!({"entity": "policy", "id": input.key}))
            .next("list_policies"));
        }
        Ok(ReadResult::all(items))
    }
}
