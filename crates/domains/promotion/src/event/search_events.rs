//! `search_events` (docs/api/promotion.md#event).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::EventStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::events;
use crate::{INVALID_FILTER, audience};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchEventsInput {
    /// Only events in these statuses now.
    #[schemars(example = ["running", "scheduled"])]
    pub status: Option<Vec<EventStatus>>,
    /// Only events with this entry mechanic (`list_event_types`).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"check_in")]
    pub entry_mechanic: Option<String>,
    /// Only events with this selection method.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"random_draw")]
    pub selection_method: Option<String>,
    /// Only events for this tier id, or `everyone` for those open to all.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"everyone")]
    pub valid_for_member_tier: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchEvents;

impl Operation for SearchEvents {
    const META: OpMeta = OpMeta {
        address: "promotion.event.search",
        tool: "search_events",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/events",
        summary: "Find events by status, entry mechanic, selection method or tier.",
        consumes: &[],
        produces: &[Slot::many("event_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchEventsInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchEvents {
    async fn read(&self, cx: &Cx, input: SearchEventsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let rows = events(&mut tx).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let statuses = input.status.unwrap_or_default();
        let matching: Vec<_> = rows
            .iter()
            .filter(|e| statuses.is_empty() || statuses.contains(&e.status(now)))
            .filter(|e| {
                input
                    .entry_mechanic
                    .as_ref()
                    .is_none_or(|m| &e.entry_mechanic == m)
            })
            .filter(|e| {
                input
                    .selection_method
                    .as_ref()
                    .is_none_or(|m| &e.selection_method == m)
            })
            .filter(|e| {
                input
                    .valid_for_member_tier
                    .as_deref()
                    .is_none_or(|t| audience(e.valid_for_member_tier.as_deref()) == t)
            })
            .collect();
        let total = matching.len() as i64;
        let items: Vec<Value> = matching
            .iter()
            .skip(usize::try_from(page.offset).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit).unwrap_or(20))
            .map(|e| {
                json!({"id": e.id, "name": e.name, "status": e.status(now),
                       "entry_mechanic": e.entry_mechanic, "selection_method": e.selection_method,
                       "valid_for_member_tier": audience(e.valid_for_member_tier.as_deref()),
                       "start_date": cx.time.format(e.start_date),
                       "end_date": cx.time.format(e.end_date)})
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(items.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn filters_with_word_enums() {
        let s = closed_schema_for::<SearchEventsInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert!(serde_json::from_value::<SearchEventsInput>(json!({"status": ["open"]})).is_err());
    }
}
