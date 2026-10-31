//! `list_active_promotions` (docs/api/promotion.md#promotion).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::enums::PromotionStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{applies_to, period, product_counts, summary};
use crate::repo::{self, PromotionFilter};
use crate::{INVALID_PERIOD, TIER_NOT_FOUND, known_tier};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListActivePromotionsInput {
    /// Start of the period to check (inclusive). Without an offset it is read in store time.
    #[schemars(example = &"2026-11-07T00:00")]
    pub valid_from: String,
    /// End of the period to check (inclusive, minute precision), after `valid_from`.
    #[schemars(example = &"2026-11-08T23:59")]
    pub valid_through: String,
    /// Only these statuses. Default: the statuses the overlap check compares (running and
    /// scheduled).
    #[schemars(example = ["running", "scheduled"])]
    pub status: Option<Vec<PromotionStatus>>,
    /// A tier id (`list_tiers`) or `everyone`: only promotions that reach it (its own and those
    /// for everyone). All audiences when left out.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub valid_for_member_tier: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListActivePromotions;

impl Operation for ListActivePromotions {
    const META: OpMeta = OpMeta {
        address: "promotion.promotion.list_active",
        tool: "list_active_promotions",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/promotions/active",
        summary: "Running and scheduled promotions whose period overlaps the given one, with what each covers. The same data the promotion_overlap check of set_sale_prices reads, so overlaps can be explained before previewing. A shopper gets only the promotions running now (whatever the period) for everyone or for their own tier.",
        consumes: &[Slot::one("tier_id")],
        produces: &[Slot::many("promotion_id")],
        checks: &[],
        errors: &[INVALID_PERIOD, TIER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListActivePromotionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListActivePromotions {
    async fn read(&self, cx: &Cx, input: ListActivePromotionsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let (from, through) = period(
            cx,
            Some(&input.valid_from),
            Some(&input.valid_through),
            INVALID_PERIOD,
            "list_active_promotions",
        )?;
        let chose_status = input.status.is_some();
        let statuses: Vec<String> = match input.status {
            Some(s) => s.iter().map(|s| s.as_str().to_owned()).collect(),
            None => crate::model::SaleRules::from_policy(&cx.policy)?
                .overlap_statuses
                .iter()
                .map(|s| s.as_str().to_owned())
                .collect(),
        };
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        // A shopper sees the sales on for them now: running, for everyone or their own tier.
        let shopper = !acom_catalog::is_operator(&cx.principal);
        let (statuses, tier) = if shopper {
            if input.valid_for_member_tier.is_some() || chose_status {
                return Err(ToolError::new(
                    ErrorCode::FORBIDDEN,
                    "Only store operators can choose the tier or statuses to list.",
                )
                .detail(json!({"reason": "scope"})));
            }
            let mine = match cx.principal.customer_id() {
                Some(c) => acom_customer::service::for_promotion::tier_of(&mut tx, c).await?,
                None => None,
            };
            (
                vec![PromotionStatus::Running.as_str().to_owned()],
                Some(mine.unwrap_or_else(|| "everyone".to_owned())),
            )
        } else {
            if let Some(t) = &input.valid_for_member_tier {
                known_tier(&mut tx, t).await?;
            }
            (statuses, input.valid_for_member_tier.clone())
        };
        // For a shopper the period is now: they see what is on, not what is coming.
        let (from, through) = if shopper {
            (Some(now), Some(now))
        } else {
            (from, through)
        };
        let filter = PromotionFilter {
            statuses,
            tier,
            tier_reach: true,
            from,
            through,
            ..PromotionFilter::default()
        };
        let (total, rows) = repo::promotions(&mut tx, &filter, now, Some(page)).await?;
        let counts = product_counts(&mut tx, &rows).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|r| {
                let mut item = summary(cx, r, now);
                item["applies_to"] = applies_to(r, counts.get(&r.id).copied().flatten());
                item
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
