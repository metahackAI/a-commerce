//! `list_event_types` (docs/api/promotion.md#event): the store's entry mechanics and selection
//! methods (`promotion.yml event.entry_mechanics`, `event.selection_methods`) and which pairs it
//! allows.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::rules::{Combinations, registry};

/// Which registry to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventTypeKind {
    /// How shoppers enter.
    EntryMechanic,
    /// How winners are chosen.
    SelectionMethod,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListEventTypesInput {
    /// entry_mechanic or selection_method; both when left out.
    #[schemars(example = &"entry_mechanic")]
    pub kind: Option<EventTypeKind>,
    /// Only the ones turned on. Default true.
    #[schemars(example = true)]
    pub enabled_only: Option<bool>,
}

pub struct ListEventTypes;

impl Operation for ListEventTypes {
    const META: OpMeta = OpMeta {
        address: "promotion.event.list_types",
        tool: "list_event_types",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/event-types",
        summary: "How shoppers can enter an event in this store (entry mechanics) and how winners can be chosen (selection methods), and which pairs are allowed. Read before create_event.",
        consumes: &[],
        produces: &[Slot::one("entry_mechanic"), Slot::one("selection_method")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListEventTypesInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListEventTypes {
    async fn read(&self, cx: &Cx, input: ListEventTypesInput) -> Result<Value, ToolError> {
        let enabled_only = input.enabled_only.unwrap_or(true);
        let mut items = Vec::new();
        for (kind, key) in [
            (EventTypeKind::EntryMechanic, "entry_mechanics"),
            (EventTypeKind::SelectionMethod, "selection_methods"),
        ] {
            if input.kind.is_some_and(|k| k != kind) {
                continue;
            }
            let word = match kind {
                EventTypeKind::EntryMechanic => "entry_mechanic",
                EventTypeKind::SelectionMethod => "selection_method",
            };
            for i in registry(&cx.policy, key)? {
                if enabled_only && !i.enabled {
                    continue;
                }
                let mut item = json!({"kind": word, "id": i.id, "name": i.name,
                                      "enabled": i.enabled, "builtin": i.builtin});
                if let Some(s) = i.entry_payload_schema {
                    item["entry_payload_schema"] = s;
                }
                items.push(item);
            }
        }
        let total = items.len();
        Ok(
            json!({"items": items, "allowed_combinations": Combinations::read(&cx.policy)?.json(),
                  "total": total}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn takes_the_spec_inputs() {
        let s = closed_schema_for::<ListEventTypesInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            s["properties"]["kind"]["enum"],
            json!(["entry_mechanic", "selection_method"])
        );
    }
}
