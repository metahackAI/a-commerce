//! `list_policies` (`platform.policy.list`): the loaded policy files and the snapshot version.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{PolicyFile, leaves, overrides};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListPoliciesInput {
    /// Only the files of this API domain, for example `promotion` or `sales`.
    #[schemars(example = &"promotion")]
    pub domain: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PolicyFileInfo {
    /// The file name, without `.yml`.
    pub file: String,
    /// The file's own version number.
    pub version: Option<i64>,
    /// The API domain its rules belong to.
    pub domain: String,
    /// What the file decides.
    pub summary: String,
    /// How many leaf values the store overrides.
    pub overridden_keys: usize,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PolicyIndex {
    pub items: Vec<PolicyFileInfo>,
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Hash of the rules in effect; every preview records it.
    pub policy_version: String,
}

pub struct ListPolicies;

impl Operation for ListPolicies {
    const META: OpMeta = OpMeta {
        address: "platform.policy.list",
        tool: "list_policies",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/policies/index",
        summary: "The loaded policy files (name, version, domain, summary, how many keys the store overrides) and the policy snapshot version. Values come from get_policy.",
        consumes: &[],
        produces: &[Slot::many("policy_key")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListPoliciesInput;
    type Output = PolicyIndex;
}

impl ReadOp<Cx> for ListPolicies {
    async fn read(&self, cx: &Cx, input: ListPoliciesInput) -> Result<PolicyIndex, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let ov = overrides(cx).await?;
        let all: Vec<PolicyFileInfo> = PolicyFile::ALL
            .iter()
            .filter(|f| input.domain.as_deref().is_none_or(|d| f.domain() == d))
            .map(|f| {
                let overridden_keys = ov.get(f.name()).map_or(0, |p| {
                    let mut l = Vec::new();
                    leaves(p, "", &mut l);
                    l.len()
                });
                PolicyFileInfo {
                    file: f.name().to_owned(),
                    version: cx
                        .policy
                        .raw(f.name())
                        .and_then(|d| d.get("version"))
                        .and_then(Value::as_i64),
                    domain: f.domain().to_owned(),
                    summary: f.summary().to_owned(),
                    overridden_keys,
                }
            })
            .collect();
        let total = all.len();
        let items: Vec<PolicyFileInfo> = all
            .into_iter()
            .skip(usize::try_from(page.offset).unwrap_or(0))
            .take(usize::try_from(page.limit).unwrap_or(20))
            .collect();
        Ok(PolicyIndex {
            next_cursor: page.next_cursor(items.len(), total as i64),
            total: total as u64,
            items,
            policy_version: cx.policy.version().to_owned(),
        })
    }
}
