//! `list_curations` (docs/api/merchandising.md#collection_page).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::DisplayStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{page_summary, pages};
use crate::display_slot::{View, operators_only, view};
use crate::{TIER_NOT_FOUND, audience, known_tier, parse_time};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListCurationsInput {
    /// Only pages in this state: draft, scheduled, active or ended.
    #[schemars(example = &"active")]
    pub status: Option<DisplayStatus>,
    /// Only pages for this audience: a tier id (`list_tiers`) or `everyone`. Operators only.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"everyone")]
    pub audience: Option<String>,
    /// Judge the state at this moment (store time without an offset). Default: now. Operators
    /// only.
    #[schemars(example = &"2026-11-07T10:00")]
    pub as_of: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListCurations;

impl Operation for ListCurations {
    const META: OpMeta = OpMeta {
        address: "merchandising.collection_page.list",
        tool: "list_curations",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/merchandising/collection-pages",
        summary: "Curated pages (collections of products in sections) with their state, period and audience. A shopper gets only the pages showing to them now.",
        consumes: &[],
        produces: &[Slot::many("collection_page_id")],
        checks: &[],
        errors: &[TIER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListCurationsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListCurations {
    async fn read(&self, cx: &Cx, input: ListCurationsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let view = view(cx, &mut tx).await?;
        let at = match (&view, &input.as_of) {
            (View::Shopper(_), Some(_)) => return Err(operators_only("as_of")),
            (View::Operator, Some(s)) => parse_time(&cx.time, "as_of", s)?,
            (_, None) => cx.clock.now(),
        };
        let wanted_audience = match (&view, &input.audience) {
            (View::Shopper(_), Some(_)) => return Err(operators_only("audience")),
            (View::Operator, Some(a)) => {
                known_tier(&mut tx, a).await?;
                Some(a.clone())
            }
            (_, None) => None,
        };
        let all = pages(&mut tx).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let matching: Vec<_> = all
            .iter()
            .filter(|p| {
                let status = p.status(at);
                let to = audience(p.valid_for_member_tier.as_deref());
                let visible = match &view {
                    View::Operator => true,
                    // A shopper sees the pages showing now, for everyone or for their tier.
                    View::Shopper(tier) => {
                        status == DisplayStatus::Active && (to == "everyone" || to == tier)
                    }
                };
                visible
                    && input.status.is_none_or(|s| s == status)
                    && wanted_audience.as_deref().is_none_or(|a| a == to)
            })
            .collect();
        let total = i64::try_from(matching.len()).unwrap_or(i64::MAX);
        let start = usize::try_from(page.offset).unwrap_or(0);
        let size = usize::try_from(page.limit).unwrap_or(0);
        let items: Vec<Value> = matching
            .iter()
            .skip(start)
            .take(size)
            .map(|p| page_summary(cx, p, at))
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(items.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
