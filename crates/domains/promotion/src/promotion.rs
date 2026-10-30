//! `list_active_promotions`, `search_promotions`, `get_promotion`, `pause_promotion`,
//! `end_promotion` (docs/api/promotion.md#promotion).

use std::collections::{BTreeMap, BTreeSet};

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, ReadOp, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::{DiscountLayer, PromotionStatus};
use acom_store::{Cx, Page, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{AppliesTo, promotion_status};
use crate::repo::{self, PromotionFilter, PromotionRow};
use crate::service::for_merchandising::category_products;
use crate::{
    INVALID_FILTER, INVALID_PERIOD, TIER_NOT_FOUND, audience, check_id, known_tier, not_found,
    parse_time, percent, pick, time_or_null,
};

/// How many products each promotion covers: the listed products, or the products of its
/// categories; `None` for an order-wide promotion.
async fn product_counts(
    tx: &mut TxCx,
    rows: &[PromotionRow],
) -> Result<BTreeMap<String, Option<i64>>, ToolError> {
    let scopes: Vec<(String, AppliesTo)> = rows
        .iter()
        .map(|r| (r.id.clone(), AppliesTo::read(&r.applies_to)))
        .collect();
    let mut wanted: Vec<String> = scopes
        .iter()
        .filter(|(_, a)| a.scope == "categories")
        .flat_map(|(_, a)| a.category_ids.clone())
        .collect();
    wanted.sort();
    wanted.dedup();
    let by_category: BTreeMap<String, Vec<String>> = category_products(tx, &wanted)
        .await?
        .into_iter()
        .map(|c| (c.category_id, c.product_ids))
        .collect();
    Ok(scopes
        .into_iter()
        .map(|(id, a)| {
            let excluded: BTreeSet<&String> = a.exclude_product_ids.iter().collect();
            let count = match a.scope.as_str() {
                "products" => Some(
                    a.product_ids
                        .iter()
                        .filter(|p| !excluded.contains(p))
                        .count() as i64,
                ),
                "categories" => {
                    let all: BTreeSet<&String> = a
                        .category_ids
                        .iter()
                        .filter_map(|c| by_category.get(c))
                        .flatten()
                        .filter(|p| !excluded.contains(p))
                        .collect();
                    Some(all.len() as i64)
                }
                _ => None,
            };
            (id, count)
        })
        .collect())
}

/// The list item of a promotion.
fn summary(cx: &Cx, r: &PromotionRow, now: DateTime<Utc>) -> Value {
    json!({
        "id": r.id,
        "name": r.name,
        "status": promotion_status(&r.promotion_status, r.valid_from, r.valid_through, now),
        "discount_layer": r.discount_layer,
        "valid_for_member_tier": audience(r.valid_for_member_tier.as_deref()),
        "valid_from": cx.time.format(r.valid_from),
        "valid_through": time_or_null(&cx.time, r.valid_through),
    })
}

fn applies_to(r: &PromotionRow, product_count: Option<i64>) -> Value {
    let a = AppliesTo::read(&r.applies_to);
    json!({"scope": if a.scope.is_empty() { "order".to_owned() } else { a.scope },
           "category_ids": a.category_ids, "product_count": product_count})
}

fn period(
    cx: &Cx,
    from: Option<&str>,
    through: Option<&str>,
    code: ErrorCode,
    next: &'static str,
) -> Result<(Option<DateTime<Utc>>, Option<DateTime<Utc>>), ToolError> {
    let bad = |field: &str, e: ToolError| {
        ToolError::new(code, e.message)
            .detail(json!({"field": field, "reason": "format"}))
            .next(next)
    };
    let from = from
        .map(|s| parse_time(&cx.time, "valid_from", s).map_err(|e| bad("valid_from", e)))
        .transpose()?;
    let through = through
        .map(|s| parse_time(&cx.time, "valid_through", s).map_err(|e| bad("valid_through", e)))
        .transpose()?;
    if let (Some(f), Some(t)) = (from, through)
        && f > t
    {
        return Err(
            ToolError::new(code, "`valid_from` is after `valid_through`.")
                .detail(json!({"field": "valid_from", "reason": "range"}))
                .next(next),
        );
    }
    Ok((from, through))
}

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
        summary: "Running and scheduled promotions whose period overlaps the given one, with what each covers. The same data the promotion_overlap check of set_sale_prices reads, so overlaps can be explained before previewing.",
        consumes: &[Slot::one("tier_id")],
        produces: &[Slot::many("promotion_id")],
        checks: &[],
        errors: &[INVALID_PERIOD, TIER_NOT_FOUND],
        audience: Audience::Operator,
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
        if let Some(t) = &input.valid_for_member_tier {
            known_tier(&mut tx, t).await?;
        }
        let filter = PromotionFilter {
            statuses,
            tier: input.valid_for_member_tier.clone(),
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchPromotionsInput {
    /// Only these statuses (the status now). All when left out.
    #[schemars(example = ["ended"])]
    pub status: Option<Vec<PromotionStatus>>,
    /// Only promotions for this tier id, or `everyone` for those without a tier.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"everyone")]
    pub valid_for_member_tier: Option<String>,
    /// Only this discount layer. item_price: member and sale prices; order_discount: discounts
    /// on the order; add_on: store points and shipping.
    #[schemars(example = &"item_price")]
    pub discount_layer: Option<DiscountLayer>,
    /// Only promotions whose period reaches this moment or later (store time without an offset).
    #[schemars(example = &"2026-10-01T00:00")]
    pub valid_from: Option<String>,
    /// Only promotions that start by this moment (store time without an offset).
    #[schemars(example = &"2026-10-31T23:59")]
    pub valid_through: Option<String>,
    /// Words in the promotion's name.
    #[schemars(length(min = 1, max = 100), example = &"linen")]
    pub query: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchPromotions;

impl Operation for SearchPromotions {
    const META: OpMeta = OpMeta {
        address: "promotion.promotion.search",
        tool: "search_promotions",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/promotions",
        summary: "Find promotions by status, period, tier, discount layer or name, whether or not they overlap anything.",
        consumes: &[],
        produces: &[Slot::many("promotion_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchPromotionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchPromotions {
    async fn read(&self, cx: &Cx, input: SearchPromotionsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let (from, through) = period(
            cx,
            input.valid_from.as_deref(),
            input.valid_through.as_deref(),
            INVALID_FILTER,
            "list_enums",
        )?;
        let filter = PromotionFilter {
            statuses: input
                .status
                .unwrap_or_default()
                .iter()
                .map(|s| s.as_str().to_owned())
                .collect(),
            tier: input.valid_for_member_tier,
            tier_reach: false,
            layer: input.discount_layer.map(|l| l.as_str().to_owned()),
            from,
            through,
            query: input.query.filter(|q| !q.trim().is_empty()),
        };
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (total, rows) = repo::promotions(&mut tx, &filter, now, Some(page)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|r| summary(cx, r, now)).collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPromotionInput {
    /// The promotion.
    #[schemars(regex(pattern = r"^promo_[a-z0-9_]+$"), example = &"promo_autumn_knit")]
    pub promotion_id: String,
    /// Only these fields of the promotion (`id` always comes back).
    #[schemars(example = ["status", "applies_to", "coupon_ids"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetPromotion;

impl Operation for GetPromotion {
    const META: OpMeta = OpMeta {
        address: "promotion.promotion.get",
        tool: "get_promotion",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/promotions/{promotion_id}",
        summary: "One promotion with what it covers, how many member or sale prices it holds, its coupons, and the change set that created it.",
        consumes: &[Slot::one("promotion_id")],
        produces: &[Slot::many("coupon_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPromotionInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetPromotion {
    async fn read(&self, cx: &Cx, input: GetPromotionInput) -> Result<Value, ToolError> {
        check_id("promotion_id", "promo", &input.promotion_id, false)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::promotion(&mut tx, &input.promotion_id)
            .await?
            .ok_or_else(|| not_found("promotion", &input.promotion_id, "search_promotions"))?;
        let counts = product_counts(&mut tx, std::slice::from_ref(&r)).await?;
        let prices = repo::price_count(&mut tx, &r.id).await?;
        let coupons = repo::coupon_ids_of(&mut tx, &r.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut out = summary(cx, &r, now);
        out["description"] = json!(r.description);
        out["percent_off"] = r.percent_off.map_or(Value::Null, percent);
        out["amount_off"] = match (r.amount_off_value, &r.amount_off_currency) {
            (Some(v), Some(c)) => crate::money(v, c),
            _ => Value::Null,
        };
        out["applies_to"] = applies_to(&r, counts.get(&r.id).copied().flatten());
        out["price_specification_count"] = json!(prices);
        out["coupon_ids"] = json!(coupons);
        out["change_set_id"] = json!(r.change_set_id);
        out["version"] = json!(r.version);
        Ok(pick(out, input.fields.as_deref()))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PromotionStateInput {
    /// The promotion.
    #[schemars(regex(pattern = r"^promo_[a-z0-9_]+$"), example = &"promo_autumn_knit")]
    pub promotion_id: String,
    /// Why, in a sentence, for the change set and the action log.
    #[schemars(length(max = 500), example = &"Stock ran out faster than planned.")]
    pub reason: Option<String>,
}

/// What `pause_promotion` and `end_promotion` run: one status change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusPlan {
    pub promotion_id: String,
    /// The stored status the change starts from (the status shown may be computed from it).
    pub stored_from: String,
    pub to: String,
    pub reason: Option<String>,
}

/// Preview of a status change: refuses a change the current status does not allow.
async fn preview_status(
    cx: &Cx,
    tool: &'static str,
    input: PromotionStateInput,
    to: PromotionStatus,
    allowed_from: &[PromotionStatus],
) -> Result<Planned<StatusPlan>, ToolError> {
    check_id("promotion_id", "promo", &input.promotion_id, false)?;
    let now = cx.clock.now();
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let r = repo::promotion(&mut tx, &input.promotion_id)
        .await?
        .ok_or_else(|| not_found("promotion", &input.promotion_id, "search_promotions"))?;
    tx.commit().await.map_err(ToolError::from)?;
    let current = promotion_status(&r.promotion_status, r.valid_from, r.valid_through, now);
    let already = current == to;
    if !already && !allowed_from.contains(&current) {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("{} is {current}, so it cannot become {to}.", r.name),
        )
        .detail(json!({"entity": "promotion", "from": current,
                       "allowed_from": allowed_from.iter().map(|s| s.as_str()).collect::<Vec<_>>()}))
        .next("get_promotion"));
    }
    let changes = if already {
        Vec::new()
    } else {
        vec![Change {
            op: ChangeOp::Update,
            entity: "promotion".into(),
            id: r.id.clone(),
            from: json!(current),
            to: json!(to),
            note: input.reason.clone(),
        }]
    };
    let facts = json!({"reaches_customers": true, "promotion_id": r.id});
    let approval = cx.policy.approval_for(tool, &facts);
    Ok(Planned {
        plan: StatusPlan {
            promotion_id: r.id.clone(),
            stored_from: r.promotion_status.clone(),
            to: to.as_str().to_owned(),
            reason: input.reason,
        },
        changes,
        checks: Vec::new(),
        basis: vec![Basis {
            entity: "promotion.promotion".into(),
            id: r.id,
            version: i64::from(r.version),
        }],
        approval: match approval {
            a if a.needed => ApprovalNeed::Needed {
                reason: a.reason.unwrap_or_default(),
            },
            _ => ApprovalNeed::NotNeeded,
        },
    })
}

async fn commit_status(tx: &mut TxCx, plan: &StatusPlan) -> Result<Committed, ToolError> {
    let moved = repo::set_status(tx, &plan.promotion_id, &plan.stored_from, &plan.to).await?;
    if moved == 0 {
        // Already there (the same plan ran before, or nothing had to change) is fine.
        let now = repo::promotion(tx, &plan.promotion_id).await?;
        if now.as_ref().map(|p| p.promotion_status.as_str()) != Some(plan.to.as_str()) {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                "The promotion changed since the preview; preview again.",
            )
            .detail(json!({"entity": "promotion", "id": plan.promotion_id}))
            .next("get_promotion"));
        }
    }
    Ok(Committed {
        id: plan.promotion_id.clone(),
        produced: serde_json::Map::from_iter([(
            "promotion_id".to_owned(),
            json!(plan.promotion_id),
        )]),
    })
}

async fn verify_status(cx: &Cx, plan: &StatusPlan) -> Result<Verification, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let now = repo::promotion(&mut tx, &plan.promotion_id).await?;
    tx.commit().await.map_err(ToolError::from)?;
    let stored = now.map(|p| p.promotion_status);
    let mut mismatches = Vec::new();
    if stored.as_deref() != Some(plan.to.as_str()) {
        mismatches.push(format!(
            "{} is {} instead of {}",
            plan.promotion_id,
            stored.as_deref().unwrap_or("missing"),
            plan.to
        ));
    }
    Ok(Verification {
        matched: mismatches.is_empty(),
        mismatches,
    })
}

async fn undo_status(tx: &mut TxCx, plan: &StatusPlan) -> Result<(), ToolError> {
    if plan.stored_from != plan.to {
        repo::set_status(tx, &plan.promotion_id, &plan.to, &plan.stored_from).await?;
    }
    Ok(())
}

pub struct PausePromotion;

impl Operation for PausePromotion {
    const META: OpMeta = OpMeta {
        address: "promotion.promotion.pause",
        tool: "pause_promotion",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/promotions/{promotion_id}/pause",
        summary: "Pause a running promotion (running → paused). While paused its member prices and coupons do not apply.",
        consumes: &[Slot::one("promotion_id")],
        produces: &[Slot::one("promotion_id")],
        checks: &[],
        errors: &[ErrorCode::INVALID_STATE, ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = PromotionStateInput;
    type Output = Value;
}

impl WriteOp<Cx> for PausePromotion {
    type Plan = StatusPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: PromotionStateInput,
    ) -> Result<Planned<StatusPlan>, ToolError> {
        preview_status(
            cx,
            Self::META.tool,
            input,
            PromotionStatus::Paused,
            &[PromotionStatus::Running],
        )
        .await
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StatusPlan) -> Result<Committed, ToolError> {
        commit_status(tx, plan).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &StatusPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        verify_status(cx, plan).await
    }

    async fn undo(&self, tx: &mut TxCx, plan: &StatusPlan, _: &Committed) -> Result<(), ToolError> {
        undo_status(tx, plan).await
    }
}

pub struct EndPromotion;

impl Operation for EndPromotion {
    const META: OpMeta = OpMeta {
        address: "promotion.promotion.end",
        tool: "end_promotion",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/promotions/{promotion_id}/end",
        summary: "End a promotion before its period is over (scheduled, running or paused → ended, final). Its member prices stop applying from then.",
        consumes: &[Slot::one("promotion_id")],
        produces: &[Slot::one("promotion_id")],
        checks: &[],
        errors: &[ErrorCode::INVALID_STATE, ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = PromotionStateInput;
    type Output = Value;
}

impl WriteOp<Cx> for EndPromotion {
    type Plan = StatusPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: PromotionStateInput,
    ) -> Result<Planned<StatusPlan>, ToolError> {
        preview_status(
            cx,
            Self::META.tool,
            input,
            PromotionStatus::Ended,
            &[
                PromotionStatus::Scheduled,
                PromotionStatus::Running,
                PromotionStatus::Paused,
            ],
        )
        .await
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StatusPlan) -> Result<Committed, ToolError> {
        commit_status(tx, plan).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &StatusPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        verify_status(cx, plan).await
    }

    async fn undo(&self, tx: &mut TxCx, plan: &StatusPlan, _: &Committed) -> Result<(), ToolError> {
        undo_status(tx, plan).await
    }
}
