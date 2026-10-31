//! `publish_curation` (docs/api/merchandising.md#collection_page).
//!
//! Fixes a draft page's period, audience and channels and schedules it (`draft` → `scheduled`,
//! or `active` when it starts now). It reaches shoppers, so it always needs a person's approval.

use std::collections::BTreeMap;

use acom_catalog::service::for_merchandising::{ProductCard, product_cards};
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::DisplayStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    ALREADY_PUBLISHED, COLLECTION_PAGE_NOT_FOUND, EMPTY_CURATION, PRODUCT_DISCONTINUED, PageRow,
    content_checks, not_found, page, still_on_sale, write_publication,
};
use crate::banner::{BannerChannel, VALID_FROM_IN_PAST, channel_input};
use crate::display_slot::{channel_words, hidden_reason};
use crate::model::{DisplayRules, count_text, period_words};
use crate::{INVALID_PERIOD, TIER_NOT_FOUND, audience, check_id, known_tier, parse_time};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishCurationInput {
    /// The draft page (`list_curations`).
    #[schemars(regex(pattern = r"^col_[a-z0-9_]+$"), example = &"col_winter_edit")]
    pub collection_page_id: String,
    /// When it starts showing (inclusive), not in the past. Without an offset it is read in
    /// store time.
    #[schemars(example = &"2026-11-20T00:00")]
    pub valid_from: String,
    /// When it stops (inclusive, minute precision), after `valid_from`.
    #[schemars(example = &"2026-11-30T23:59")]
    pub valid_through: String,
    /// Who sees it: a tier id (`list_tiers`) or `everyone`. Default: the page's audience.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"everyone")]
    pub audience: Option<String>,
    /// Sales channels it shows on: web, mobile_web, app. Default: all three.
    #[schemars(length(min = 1, max = 3), example = ["web", "app"])]
    pub channels: Option<Vec<BannerChannel>>,
}

/// What `publish_curation` writes, with the draft as it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishPlan {
    pub page: PageRow,
    pub before: PageRow,
}

/// Publish a curated page for a period.
#[derive(Debug)]
pub struct PublishCuration;

impl Operation for PublishCuration {
    const META: OpMeta = OpMeta {
        address: "merchandising.collection_page.publish",
        tool: "publish_curation",
        group: Group::Offer,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/merchandising/collection-pages/{collection_page_id}/publish",
        summary: "Publish a draft curated page: fix its period, audience and channels and schedule it (it shows from valid_from). Reaches shoppers, so it always needs a person's approval.",
        consumes: &[Slot::one("collection_page_id")],
        produces: &[Slot::one("collection_page_id")],
        checks: &[
            "display.start_not_in_past",
            "curation.no_empty_section",
            "display.exclude_discontinued",
        ],
        errors: &[
            COLLECTION_PAGE_NOT_FOUND,
            VALID_FROM_IN_PAST,
            INVALID_PERIOD,
            EMPTY_CURATION,
            ALREADY_PUBLISHED,
            PRODUCT_DISCONTINUED,
            TIER_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::SCHEDULE,
        person_scope: "",
    };
    type Input = PublishCurationInput;
    type Output = Value;
}

fn already(p: &PageRow, status: DisplayStatus) -> ToolError {
    ToolError::new(
        ALREADY_PUBLISHED,
        format!(
            "{} is {} already; only a draft can be published.",
            p.id,
            status.as_str()
        ),
    )
    .detail(
        json!({"entity": "collection_page", "id": p.id, "from": status.as_str(),
                   "allowed_to": []}),
    )
    .next("get_curation")
}

impl WriteOp<Cx> for PublishCuration {
    type Plan = PublishPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: PublishCurationInput,
    ) -> Result<Planned<PublishPlan>, ToolError> {
        check_id("collection_page_id", "col", &input.collection_page_id)?;
        let rules = DisplayRules::from_policy(&cx.policy)?;
        let from = parse_time(&cx.time, "valid_from", &input.valid_from)?;
        let through = parse_time(&cx.time, "valid_through", &input.valid_through)?;
        if through <= from {
            return Err(ToolError::new(
                INVALID_PERIOD,
                "`valid_through` must come after `valid_from`.",
            )
            .detail(json!({"field": "valid_through", "reason": "range"}))
            .next("publish_curation"));
        }
        let now = cx.clock.now();
        if rules.start_not_in_past && from < now {
            return Err(ToolError::new(
                VALID_FROM_IN_PAST,
                format!(
                    "The page would start in the past ({}).",
                    cx.time.format(from)
                ),
            )
            .detail(json!({"field": "valid_from", "reason": "past"}))
            .next("publish_curation"));
        }

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let draft = page(&mut tx, &input.collection_page_id)
            .await?
            .ok_or_else(|| not_found(&input.collection_page_id))?;
        let status = draft.status(now);
        if status != DisplayStatus::Draft {
            return Err(already(&draft, status));
        }
        let tier = match &input.audience {
            Some(a) => known_tier(&mut tx, a).await?,
            None => match &draft.valid_for_member_tier {
                Some(t) => known_tier(&mut tx, t).await?,
                None => None,
            },
        };
        let products = draft.products();
        let cards: Vec<ProductCard> = product_cards(&mut tx, &products).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if products.is_empty() {
            return Err(ToolError::new(
                EMPTY_CURATION,
                format!("{} has no products to show.", draft.name),
            )
            .detail(json!({"rule": "curation.no_empty_section", "id": draft.id}))
            .next("update_curation"));
        }

        let by_id: BTreeMap<&str, &ProductCard> =
            cards.iter().map(|c| (c.product_id.as_str(), c)).collect();
        let hidden = products
            .iter()
            .filter(|p| hidden_reason(&rules, by_id.get(p.as_str()).copied()).is_some())
            .count();
        let sections = draft.sections();
        let mut checks = vec![Check {
            rule: "display.start_not_in_past".into(),
            passed: true,
            detail: Some(format!("Starts {}, after now", cx.time.format(from))),
            fix_tool: None,
        }];
        let mut content = content_checks(&rules, &sections, &cards, "update_curation");
        // The sections check carries what the page shows: sections, products, hidden ones.
        if let Some(c) = content
            .iter_mut()
            .find(|c| c.rule == "curation.no_empty_section" && c.passed)
        {
            c.detail = Some(format!(
                "{} section{}, {} product{}; {hidden} sold out or not for sale stay hidden until they can be bought",
                sections.len(),
                if sections.len() == 1 { "" } else { "s" },
                products.len(),
                if products.len() == 1 { "" } else { "s" },
            ));
        }
        content.reverse();
        checks.extend(content);
        let channels = channel_input(input.channels.as_deref());
        let new_status = if from <= now {
            DisplayStatus::Active
        } else {
            DisplayStatus::Scheduled
        };
        let published = PageRow {
            valid_for_member_tier: tier.as_ref().map(|t| t.id.clone()),
            valid_from: Some(from),
            valid_through: Some(through),
            display_status: new_status.as_str().to_owned(),
            sales_channel: channels,
            version: 0,
            ..draft.clone()
        };
        let reach = match &tier {
            Some(t) => format!("{} {} members", count_text(t.member_count), t.name),
            None => "everyone".to_owned(),
        };
        let change = Change {
            op: ChangeOp::Update,
            entity: "collection_page".into(),
            id: draft.id.clone(),
            from: json!({"status": "draft",
                         "valid_from": draft.valid_from.map(|t| cx.time.format(t)),
                         "valid_through": draft.valid_through.map(|t| cx.time.format(t))}),
            to: json!({
                "status": new_status,
                "valid_from": cx.time.format(from),
                "valid_through": cx.time.format(through),
                "audience": audience(published.valid_for_member_tier.as_deref()),
                "channels": channel_words(published.sales_channel.as_deref()),
            }),
            note: Some(format!(
                "Shows {}",
                period_words(from, Some(through), cx.time.zone)
            )),
        };
        Ok(Planned {
            plan: PublishPlan {
                page: published,
                before: draft.unversioned(),
            },
            changes: vec![change],
            checks,
            basis: vec![Basis {
                entity: "merchandising.collection_page".into(),
                id: draft.id.clone(),
                version: i64::from(draft.version),
            }],
            approval: ApprovalNeed::Needed {
                reason: format!(
                    "Publishes a page with {} product{} to {reach}",
                    count_text(i64::try_from(products.len()).unwrap_or(i64::MAX)),
                    if products.len() == 1 { "" } else { "s" }
                ),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &PublishPlan) -> Result<Committed, ToolError> {
        let rules = DisplayRules::from_policy(&tx.policy)?;
        let now = tx.clock.now();
        let current = page(tx, &plan.page.id)
            .await?
            .ok_or_else(|| not_found(&plan.page.id))?;
        let status = current.status(now);
        if status != DisplayStatus::Draft {
            // Running the same plan again finds it published as planned.
            if current.unversioned() == plan.page {
                return Ok(done(plan));
            }
            return Err(already(&current, status));
        }
        still_on_sale(tx, &rules, &plan.page.products(), "update_curation").await?;
        write_publication(tx, &plan.page).await?;
        Ok(done(plan))
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &PublishPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let stored = page(&mut tx, &plan.page.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let p = &plan.page;
        let matched = stored.is_some_and(|s| {
            s.display_status == p.display_status
                && s.valid_from == p.valid_from
                && s.valid_through == p.valid_through
                && s.valid_for_member_tier == p.valid_for_member_tier
                && s.sales_channel == p.sales_channel
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("curated page {} is not published as planned", p.id)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &PublishPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        let now = tx.clock.now();
        let Some(current) = page(tx, &plan.page.id).await? else {
            return Ok(());
        };
        if current.valid_from.is_none_or(|f| now <= f) {
            // Never shown: back to the draft it was.
            write_publication(tx, &plan.before).await
        } else {
            // Shown already: take it down now, keeping the record of when it showed.
            write_publication(
                tx,
                &PageRow {
                    display_status: DisplayStatus::Ended.as_str().to_owned(),
                    valid_through: Some(current.valid_through.map_or(now, |t| t.min(now))),
                    ..current
                },
            )
            .await
        }
    }
}

fn done(plan: &PublishPlan) -> Committed {
    Committed {
        id: plan.page.id.clone(),
        produced: Map::from_iter([("collection_page_id".to_owned(), json!(plan.page.id))]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_spec_input_with_a_closed_schema() {
        let i: PublishCurationInput = serde_json::from_value(json!({
            "collection_page_id": "col_winter_edit",
            "valid_from": "2026-11-20T00:00:00-05:00", "valid_through": "2026-11-30T23:59:00-05:00",
            "audience": "everyone", "channels": ["web", "app"]}))
        .unwrap();
        assert_eq!(
            channel_input(i.channels.as_deref()),
            Some(vec!["web".to_owned(), "app".to_owned()])
        );
        assert!(
            serde_json::from_value::<PublishCurationInput>(json!({
                "collection_page_id": "col_winter_edit", "valid_from": "x"}))
            .is_err()
        );
        let schema = acom_contract::schema::closed_schema_for::<PublishCurationInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(PublishCuration::META.kind, Kind::WriteApproval);
    }
}
