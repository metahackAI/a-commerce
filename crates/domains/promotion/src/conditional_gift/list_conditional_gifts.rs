//! `list_conditional_gifts` (docs/api/promotion.md#conditional_gift).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{GiftStatus, gifts};
use crate::{INVALID_FILTER, audience};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListConditionalGiftsInput {
    /// Only gifts in these statuses now. scheduled: not started; running: orders get it now;
    /// ended: over. All when left out.
    #[schemars(example = ["running", "scheduled"])]
    pub status: Option<Vec<GiftStatus>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListConditionalGifts;

impl Operation for ListConditionalGifts {
    const META: OpMeta = OpMeta {
        address: "promotion.conditional_gift.list",
        tool: "list_conditional_gifts",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/conditional-gifts",
        summary: "The gifts added to orders over a threshold, with their thresholds, gifts, period and status.",
        consumes: &[],
        produces: &[Slot::many("conditional_gift_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListConditionalGiftsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListConditionalGifts {
    async fn read(&self, cx: &Cx, input: ListConditionalGiftsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let wanted = input.status.unwrap_or_default();
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let rows = gifts(&mut tx).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matching: Vec<(GiftStatus, &super::GiftRow)> = rows
            .iter()
            .map(|g| (GiftStatus::at(g.valid_from, g.valid_through, now), g))
            .filter(|(s, _)| wanted.is_empty() || wanted.contains(s))
            .collect();
        let total = matching.len() as i64;
        let items: Vec<Value> = matching
            .iter()
            .skip(usize::try_from(page.offset).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit).unwrap_or(20))
            .map(|(s, g)| {
                json!({"id": g.id, "name": g.name, "status": s.word(), "tiers": g.gift_tiers,
                       "valid_for_member_tier": audience(g.valid_for_member_tier.as_deref()),
                       "channels": g.sales_channels,
                       "valid_from": cx.time.format(g.valid_from),
                       "valid_through": cx.time.format(g.valid_through)})
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
    fn filters_by_the_status_words() {
        let s = closed_schema_for::<ListConditionalGiftsInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            s["properties"]["status"]["items"]["enum"],
            json!(["scheduled", "running", "ended"])
        );
        assert!(
            serde_json::from_value::<ListConditionalGiftsInput>(json!({"status": ["paused"]}))
                .is_err()
        );
    }
}
