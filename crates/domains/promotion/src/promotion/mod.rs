//! The `promotion` resource (docs/api/promotion.md#promotion): what the reads of a promotion
//! share (its list item, what it covers, the period inputs) and the status change behind
//! `pause_promotion` and `end_promotion`. One file per operation beside this one.

pub(crate) mod end_promotion;
pub(crate) mod get_promotion;
pub(crate) mod list_active_promotions;
pub(crate) mod pause_promotion;
pub(crate) mod search_promotions;

use std::collections::{BTreeMap, BTreeSet};

use acom_contract::{
    ApprovalNeed, Basis, Change, ChangeOp, Committed, ErrorCode, Planned, ToolError, Verification,
};
use acom_core::enums::PromotionStatus;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{AppliesTo, promotion_status};
use crate::repo::{self, PromotionRow};
use crate::service::for_merchandising::category_products;
use crate::{audience, check_id, not_found, parse_time, time_or_null};

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

/// A period from an input; either end may be open.
type Period = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);

fn period(
    cx: &Cx,
    from: Option<&str>,
    through: Option<&str>,
    code: ErrorCode,
    next: &'static str,
) -> Result<Period, ToolError> {
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
    // Nothing to change when the preview found it there already.
    let moved = if plan.stored_from == plan.to {
        0
    } else {
        repo::set_status(tx, &plan.promotion_id, &plan.stored_from, &plan.to).await?
    };
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
