//! `get_event` (docs/api/promotion.md#event).

use std::collections::BTreeMap;

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::enums::RewardStatus;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{draw_record_ids, entry_counts, event_json, known_event, rewards};
use crate::pick;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEventInput {
    /// The event.
    #[schemars(regex(pattern = r"^evt_[a-z0-9_]+$"), example = &"evt_0311")]
    pub event_id: String,
    /// Only these fields of the event (`id` always comes back).
    #[schemars(example = ["status", "reward_tiers", "rewards"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetEvent;

impl Operation for GetEvent {
    const META: OpMeta = OpMeta {
        address: "promotion.event.get",
        tool: "get_event",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/events/{event_id}",
        summary: "One event: entry mechanic, selection method, reward ranks, who may enter, entry limit, how many entered, its draw records and how its rewards stand.",
        consumes: &[Slot::one("event_id")],
        produces: &[Slot::many("draw_record_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetEventInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetEvent {
    async fn read(&self, cx: &Cx, input: GetEventInput) -> Result<Value, ToolError> {
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let e = known_event(&mut tx, &input.event_id, "search_events").await?;
        let counts = entry_counts(&mut tx, &e.id).await?;
        let draws = draw_record_ids(&mut tx, &e.id).await?;
        let paid = rewards(&mut tx, Some(&e.id), None).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut by_status: BTreeMap<&str, i64> =
            RewardStatus::ALL.iter().map(|s| (s.as_str(), 0)).collect();
        for r in &paid {
            if let Some(n) = by_status.get_mut(r.reward_status.as_str()) {
                *n += 1;
            }
        }
        let mut out = event_json(&cx.time, &e, now);
        out["entrant_count"] = json!(counts.values().sum::<i64>());
        out["entries"] = json!(counts);
        out["draw_record_ids"] = json!(draws);
        out["rewards"] = json!(by_status);
        out["version"] = json!(e.version);
        Ok(pick(out, input.fields.as_deref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn takes_the_spec_inputs() {
        let s = closed_schema_for::<GetEventInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(s["required"], json!(["event_id"]));
    }
}
