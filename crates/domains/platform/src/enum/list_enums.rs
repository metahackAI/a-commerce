//! `list_enums` (`platform.enum.list`): every word list with labels and order.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ReadResult, ToolError,
};
use acom_core::enums::LISTS;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ENUM_NOT_FOUND, TERMINAL, applies_to, label};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListEnumsInput {
    /// Only these lists.
    #[schemars(example = ["order_status", "availability"])]
    pub names: Option<Vec<String>>,
    /// Only the lists a table uses, as `schema.table`.
    #[schemars(example = &"sales.sales_order")]
    pub applies_to: Option<String>,
}

pub struct ListEnums;

impl Operation for ListEnums {
    const META: OpMeta = OpMeta {
        address: "platform.enum.list",
        tool: "list_enums",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/enums",
        summary: "Every word list (order_status, availability, change_set_state, …) with labels and order, so values can be chosen and checked correctly.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[ENUM_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListEnumsInput;
    type Output = ReadResult<Value>;
}

impl ReadOp<Cx> for ListEnums {
    async fn read(&self, _: &Cx, input: ListEnumsInput) -> Result<ReadResult<Value>, ToolError> {
        if let Some(missing) = input
            .names
            .iter()
            .flatten()
            .find(|n| !LISTS.iter().any(|(l, _)| l == n))
        {
            return Err(ToolError::new(
                ENUM_NOT_FOUND,
                format!("There is no word list named `{missing}`."),
            )
            .next("list_enums"));
        }
        let items: Vec<Value> = LISTS
            .iter()
            .filter(|(name, _)| {
                input
                    .names
                    .as_ref()
                    .is_none_or(|ns| ns.iter().any(|n| n == name))
            })
            .filter_map(|(name, words)| {
                let tables = applies_to(name);
                if input
                    .applies_to
                    .as_deref()
                    .is_some_and(|t| !tables.iter().any(|x| x == t))
                {
                    return None;
                }
                let terminal = TERMINAL
                    .iter()
                    .find(|(n, _)| n == name)
                    .map_or(&[][..], |(_, t)| *t);
                let values: Vec<Value> = words
                    .iter()
                    .enumerate()
                    .map(|(i, w)| {
                        let mut v = json!({"value": w, "label": label(w), "order": i + 1});
                        if terminal.contains(w) {
                            v["terminal"] = json!(true);
                        }
                        v
                    })
                    .collect();
                Some(json!({"name": name, "applies_to": tables, "values": values}))
            })
            .collect();
        Ok(ReadResult::all(items))
    }
}
