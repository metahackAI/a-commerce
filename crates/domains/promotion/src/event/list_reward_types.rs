//! `list_reward_types` (docs/api/promotion.md#event): the store's reward types
//! (`promotion.yml event.reward_types`) with how each is sent and whether it has a value.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::rules::registry;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListRewardTypesInput {
    /// Only the ones turned on. Default true.
    #[schemars(example = true)]
    pub enabled_only: Option<bool>,
}

pub struct ListRewardTypes;

impl Operation for ListRewardTypes {
    const META: OpMeta = OpMeta {
        address: "promotion.event.list_reward_types",
        tool: "list_reward_types",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/reward-types",
        summary: "The kinds of event rewards this store gives, how each is sent (ship, issue, grant, email, manual) and whether it has a value.",
        consumes: &[],
        produces: &[Slot::one("reward_type")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListRewardTypesInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListRewardTypes {
    async fn read(&self, cx: &Cx, input: ListRewardTypesInput) -> Result<Value, ToolError> {
        let enabled_only = input.enabled_only.unwrap_or(true);
        let items: Vec<Value> = registry(&cx.policy, "reward_types")?
            .into_iter()
            .filter(|i| !enabled_only || i.enabled)
            .map(|i| {
                json!({"id": i.id, "name": i.name, "fulfillment": i.fulfillment,
                       "has_value": i.has_value.unwrap_or(false), "enabled": i.enabled,
                       "builtin": i.builtin})
            })
            .collect();
        let total = items.len();
        Ok(json!({"items": items, "total": total}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn takes_the_spec_inputs() {
        let s = closed_schema_for::<ListRewardTypesInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }
}
