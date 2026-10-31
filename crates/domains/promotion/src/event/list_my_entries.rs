//! `list_my_entries` (docs/api/promotion.md#event): the signed-in shopper's entries and wins,
//! with what a store rule still waits for before a reward is sent.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::enums::EntryStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{entries_of, rewards};
use crate::check_id;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListMyEntriesInput {
    /// Only the entries of this event.
    #[schemars(regex(pattern = r"^evt_[a-z0-9_]+$"), example = &"evt_0311")]
    pub event_id: Option<String>,
    /// Only entries in these statuses.
    #[schemars(example = ["won"])]
    pub status: Option<Vec<EntryStatus>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListMyEntries;

impl Operation for ListMyEntries {
    const META: OpMeta = OpMeta {
        address: "promotion.event.list_my_entries",
        tool: "list_my_entries",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/events/entries/mine",
        summary: "The signed-in shopper's event entries and wins, and what they still need to send before a held reward goes out.",
        consumes: &[],
        produces: &[Slot::many("entry_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListMyEntriesInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListMyEntries {
    async fn read(&self, cx: &Cx, input: ListMyEntriesInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let customer = cx
            .principal
            .customer_id()
            .map(str::to_owned)
            .ok_or_else(|| {
                ToolError::new(ErrorCode::FORBIDDEN, "Sign in to see your entries.")
                    .detail(json!({"reason": "owner"}))
            })?;
        if let Some(e) = &input.event_id {
            check_id("event_id", "evt", e, false)?;
        }
        let statuses: Vec<&str> = input.status.iter().flatten().map(|s| s.as_str()).collect();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mine = entries_of(&mut tx, &customer, input.event_id.as_deref()).await?;
        let paid = rewards(&mut tx, input.event_id.as_deref(), Some(&customer)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matching: Vec<_> = mine
            .iter()
            .filter(|x| statuses.is_empty() || statuses.contains(&x.entry_status.as_str()))
            .collect();
        let total = matching.len() as i64;
        let items: Vec<Value> = matching
            .iter()
            .skip(usize::try_from(page.offset).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit).unwrap_or(20))
            .map(|x| {
                let r = paid.iter().find(|r| r.entry_id == x.id);
                let reward = match (x.entry_status.as_str(), r) {
                    (_, Some(r)) => json!({"rank": r.rank, "reward_type": r.reward_type, "reward_status": r.reward_status}),
                    ("won", None) => json!({"rank": x.rank, "reward_type": null, "reward_status": "pending"}),
                    _ => Value::Null,
                };
                json!({"id": x.id, "event_id": x.event_id, "entry_status": x.entry_status,
                       "reward": reward,
                       "compliance_pending": r.filter(|r| r.reward_status == "compliance_pending")
                           .map(|r| r.compliance_requirements.clone()).unwrap_or_default(),
                       "waitlist_position": x.waitlist_position,
                       "entered_at": cx.time.format(x.start_time)})
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
    fn filters_with_the_entry_words() {
        let s = closed_schema_for::<ListMyEntriesInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            s["properties"]["status"]["items"]["enum"],
            json!(["entered", "invalid", "won", "lost", "waitlist", "cancelled"])
        );
    }
}
