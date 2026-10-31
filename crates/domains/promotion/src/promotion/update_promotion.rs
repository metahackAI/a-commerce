//! `update_promotion` (docs/api/promotion.md#promotion): a promotion's name, period and scope.
//!
//! Reading of the spec: an ended promotion cannot change. A running one keeps its start (moving
//! it would take a benefit back mid-way; pause or end it instead), and the inputs cannot change a
//! discount amount, so every other change is allowed while it runs. A promotion that holds sale
//! prices (`set_sale_prices`) priced its products one by one: its period can move (the prices
//! follow), its products cannot.

use std::collections::BTreeSet;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::{DiscountLayer, PromotionStatus};
use acom_store::{Cx, TxCx};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::layer_overlap;
use crate::model::{AppliesTo, SaleRules, and_list, percent_text, promotion_status};
use crate::offer::others;
use crate::repo::{self, PromotionFields};
use crate::service::for_merchandising::category_products;
use crate::{INVALID_PERIOD, audience, check_id, not_found, parse_time, time_or_null};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePromotionInput {
    /// The promotion.
    #[schemars(regex(pattern = r"^promo_[a-z0-9_]+$"), example = &"promo_autumn_knit")]
    pub promotion_id: String,
    /// New name (the cart's discount line label).
    #[schemars(length(min = 1, max = 100), example = &"Autumn knitwear 15% — extended")]
    pub name: Option<String>,
    /// New start (inclusive), store time without an offset. A running promotion keeps its start.
    #[schemars(example = &"2026-11-20T00:00")]
    pub valid_from: Option<String>,
    /// New end (inclusive, minute precision), store time without an offset.
    #[schemars(example = &"2026-12-02T23:59")]
    pub valid_through: Option<String>,
    /// New products (`search_products`); the promotion then covers exactly these.
    #[schemars(
        length(min = 1, max = 500),
        inner(regex(pattern = r"^prd_[a-z0-9_]+$")),
        example = ["prd_denim_jacket"]
    )]
    pub product_ids: Option<Vec<String>>,
    /// New categories (`list_categories`); the promotion then covers their products.
    #[schemars(
        length(min = 1, max = 50),
        inner(regex(pattern = r"^cat_[a-z0-9_]+$")),
        example = ["cat_knitwear"]
    )]
    pub category_ids: Option<Vec<String>>,
    /// Products to leave out, replacing the current list (`[]` leaves none out).
    #[schemars(
        length(max = 500),
        inner(regex(pattern = r"^prd_[a-z0-9_]+$")),
        example = ["prd_cashmere_sweater"]
    )]
    pub exclude_product_ids: Option<Vec<String>>,
}

/// What a run of `update_promotion` writes: the fields before and after.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateRun {
    pub promotion_id: String,
    pub before: PromotionFields,
    pub after: PromotionFields,
}

pub struct UpdatePromotion;

impl Operation for UpdatePromotion {
    const META: OpMeta = OpMeta {
        address: "promotion.promotion.update",
        tool: "update_promotion",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/promotion/promotions/{promotion_id}",
        summary: "Change a promotion's name, period or products (products, categories, exclusions). An ended promotion cannot change, and a running one keeps its start; to stop a benefit, pause or end it. Checks overlaps and the discount limit again.",
        consumes: &[Slot::one("promotion_id")],
        produces: &[Slot::one("promotion_id")],
        checks: &["promotion_overlap", "max_discount"],
        errors: &[
            ErrorCode::INVALID_STATE,
            ErrorCode::NOT_FOUND,
            INVALID_PERIOD,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdatePromotionInput;
    type Output = Value;
}

fn refuse(message: String, from: PromotionStatus, next: &'static str) -> ToolError {
    ToolError::new(ErrorCode::INVALID_STATE, message)
        .detail(json!({"entity": "promotion", "from": from}))
        .next(next)
}

impl WriteOp<Cx> for UpdatePromotion {
    type Plan = UpdateRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdatePromotionInput,
    ) -> Result<Planned<UpdateRun>, ToolError> {
        check_id("promotion_id", "promo", &input.promotion_id, false)?;
        let asked = input.name.is_some()
            || input.valid_from.is_some()
            || input.valid_through.is_some()
            || input.product_ids.is_some()
            || input.category_ids.is_some()
            || input.exclude_product_ids.is_some();
        if !asked {
            return Err(ToolError::invalid_input(
                "Say what to change: name, valid_from, valid_through, product_ids, category_ids or exclude_product_ids.",
            )
            .detail(json!({"field": "name", "reason": "missing"})));
        }
        if input.product_ids.is_some() && input.category_ids.is_some() {
            return Err(
                ToolError::invalid_input("Give products or categories, not both.")
                    .detail(json!({"field": "category_ids", "reason": "format"})),
            );
        }
        let rules = SaleRules::from_policy(&cx.policy)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::promotion(&mut tx, &input.promotion_id)
            .await?
            .ok_or_else(|| not_found("promotion", &input.promotion_id, "search_promotions"))?;
        let status = promotion_status(&r.promotion_status, r.valid_from, r.valid_through, now);
        if status == PromotionStatus::Ended {
            return Err(refuse(
                format!("{} has ended; it cannot change.", r.name),
                status,
                "get_promotion",
            ));
        }
        let before = PromotionFields {
            name: r.name.clone(),
            valid_from: r.valid_from,
            valid_through: r.valid_through,
            applies_to: r.applies_to.clone(),
        };
        let mut after = before.clone();
        if let Some(n) = &input.name {
            let n = n.trim();
            if n.is_empty() {
                return Err(ToolError::invalid_input("`name` must not be empty.")
                    .detail(json!({"field": "name", "reason": "missing"})));
            }
            after.name = n.to_owned();
        }
        if let Some(s) = &input.valid_from {
            after.valid_from = parse_time(&cx.time, "valid_from", s)?;
        }
        if let Some(s) = &input.valid_through {
            after.valid_through = Some(parse_time(&cx.time, "valid_through", s)?);
        }
        if after.valid_through.is_some_and(|t| t <= after.valid_from) {
            return Err(ToolError::new(
                INVALID_PERIOD,
                "`valid_from` must come before `valid_through`.",
            )
            .detail(json!({"field": "valid_from", "reason": "range"}))
            .next("update_promotion"));
        }
        if status == PromotionStatus::Running && after.valid_from != before.valid_from {
            return Err(refuse(
                format!(
                    "{} is running, so its start cannot move; pause or end it to stop the benefit.",
                    r.name
                ),
                status,
                "pause_promotion",
            ));
        }
        let mut scope = AppliesTo::read(&r.applies_to);
        let scope_change = input.product_ids.is_some()
            || input.category_ids.is_some()
            || input.exclude_product_ids.is_some();
        if let Some(ids) = &input.product_ids {
            for id in ids {
                check_id("product_ids", "prd", id, false)?;
            }
            scope.scope = "products".into();
            scope.product_ids = ids
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            scope.category_ids = Vec::new();
        }
        if let Some(ids) = &input.category_ids {
            for id in ids {
                check_id("category_ids", "cat", id, false)?;
            }
            let found = category_products(&mut tx, ids).await?;
            if let Some(missing) = ids
                .iter()
                .find(|c| !found.iter().any(|f| &f.category_id == *c))
            {
                return Err(not_found("category", missing, "search_products"));
            }
            scope.scope = "categories".into();
            scope.category_ids = ids.clone();
            scope.product_ids = Vec::new();
        }
        if let Some(ids) = &input.exclude_product_ids {
            for id in ids {
                check_id("exclude_product_ids", "prd", id, false)?;
            }
            scope.exclude_product_ids = ids.clone();
        }
        if scope_change {
            if repo::price_count(&mut tx, &r.id).await? > 0 {
                return Err(refuse(
                    format!(
                        "{} sets sale prices product by product; for other products set new sale prices and end this promotion.",
                        r.name
                    ),
                    status,
                    "set_sale_prices",
                ));
            }
            after.applies_to = serde_json::to_value(&scope).unwrap_or_default();
        }
        // The products the promotion covers after the change (None = the whole order).
        let covered: Option<BTreeSet<String>> = match scope.scope.as_str() {
            "products" => Some(scope.product_ids.iter().cloned().collect()),
            "categories" => Some(
                category_products(&mut tx, &scope.category_ids)
                    .await?
                    .into_iter()
                    .flat_map(|c| c.product_ids)
                    .collect(),
            ),
            _ => None,
        }
        .map(|s: BTreeSet<String>| {
            s.into_iter()
                .filter(|p| !scope.exclude_product_ids.contains(p))
                .collect()
        });
        let through = after
            .valid_through
            .unwrap_or(after.valid_from + chrono::Duration::days(36500));
        let (mut others, mut basis) =
            others(&mut tx, &rules, after.valid_from, through, now).await?;
        tx.commit().await.map_err(ToolError::from)?;
        others.retain(|o| o.id != r.id);
        basis.retain(|b| b.id != r.id);
        basis.push(Basis {
            entity: "promotion.promotion".into(),
            id: r.id.clone(),
            version: i64::from(r.version),
        });

        let layer: DiscountLayer = r
            .discount_layer
            .parse()
            .unwrap_or(DiscountLayer::OrderDiscount);
        let words = match &covered {
            None => "the whole order".to_owned(),
            Some(p) => format!("its {} products", p.len()),
        };
        let overlap = layer_overlap(
            &rules,
            layer,
            r.valid_for_member_tier.as_deref(),
            covered.as_ref(),
            &others,
            &words,
            after.valid_from,
            through,
            &cx.time,
        );
        let max = match (
            r.percent_off,
            rules.max_discount(r.valid_for_member_tier.as_deref()),
        ) {
            (Some(p), Some(cap)) if p > cap * Decimal::ONE_HUNDRED => Check {
                rule: "max_discount".into(),
                passed: false,
                detail: Some(rules.over_limit_words(p, r.valid_for_member_tier.as_deref(), cap)),
                fix_tool: Some("end_promotion".into()),
            },
            (Some(p), Some(cap)) => Check {
                rule: "max_discount".into(),
                passed: true,
                detail: Some(format!(
                    "{}% stays within the {}% limit for {}",
                    percent_text(p),
                    percent_text(cap * Decimal::ONE_HUNDRED),
                    audience(r.valid_for_member_tier.as_deref())
                )),
                fix_tool: None,
            },
            _ => Check {
                rule: "max_discount".into(),
                passed: true,
                detail: Some("The discount itself does not change".into()),
                fix_tool: None,
            },
        };

        let mut from = serde_json::Map::new();
        let mut to = serde_json::Map::new();
        let mut changed = Vec::new();
        if after.name != before.name {
            from.insert("name".into(), json!(before.name));
            to.insert("name".into(), json!(after.name));
            changed.push("name");
        }
        if after.valid_from != before.valid_from {
            from.insert(
                "valid_from".into(),
                json!(cx.time.format(before.valid_from)),
            );
            to.insert("valid_from".into(), json!(cx.time.format(after.valid_from)));
            changed.push("start");
        }
        if after.valid_through != before.valid_through {
            from.insert(
                "valid_through".into(),
                time_or_null(&cx.time, before.valid_through),
            );
            to.insert(
                "valid_through".into(),
                time_or_null(&cx.time, after.valid_through),
            );
            changed.push("end");
        }
        if after.applies_to != before.applies_to {
            from.insert("applies_to".into(), before.applies_to.clone());
            to.insert("applies_to".into(), after.applies_to.clone());
            changed.push("products");
        }
        let changes = if changed.is_empty() {
            Vec::new()
        } else {
            vec![Change {
                op: ChangeOp::Update,
                entity: "promotion".into(),
                id: r.id.clone(),
                from: Value::Object(from),
                to: Value::Object(to),
                note: Some(format!(
                    "Changes the {} of {}",
                    and_list(&changed.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>()),
                    r.name
                )),
            }]
        };
        let facts = json!({"reaches_customers": true, "promotion_id": r.id});
        let approval = cx.policy.approval_for(Self::META.tool, &facts);
        Ok(Planned {
            plan: UpdateRun {
                promotion_id: r.id,
                before,
                after,
            },
            changes,
            checks: vec![overlap, max],
            basis,
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval.reason.unwrap_or_default(),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &UpdateRun) -> Result<Committed, ToolError> {
        if run.after != run.before
            && !repo::set_promotion_fields(tx, &run.promotion_id, &run.after).await?
        {
            return Err(not_found(
                "promotion",
                &run.promotion_id,
                "search_promotions",
            ));
        }
        Ok(Committed {
            id: run.promotion_id.clone(),
            produced: serde_json::Map::from_iter([(
                "promotion_id".to_owned(),
                json!(run.promotion_id),
            )]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &UpdateRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::promotion(&mut tx, &run.promotion_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let now = r.map(|r| PromotionFields {
            name: r.name,
            valid_from: r.valid_from,
            valid_through: r.valid_through,
            applies_to: r.applies_to,
        });
        let matched = now.as_ref() == Some(&run.after);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "promotion {} differs from the plan",
                    run.promotion_id
                )]
            },
        })
    }

    async fn undo(&self, tx: &mut TxCx, run: &UpdateRun, _: &Committed) -> Result<(), ToolError> {
        if run.after != run.before {
            repo::set_promotion_fields(tx, &run.promotion_id, &run.before).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn takes_the_spec_inputs_with_a_closed_schema() {
        let s = closed_schema_for::<UpdatePromotionInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        let i: UpdatePromotionInput = serde_json::from_value(
            json!({"promotion_id": "promo_1051", "valid_through": "2026-12-02T23:59"}),
        )
        .unwrap();
        assert_eq!(i.valid_through.as_deref(), Some("2026-12-02T23:59"));
        assert!(
            serde_json::from_value::<UpdatePromotionInput>(
                json!({"promotion_id": "promo_1051", "percent_off": 30})
            )
            .is_err()
        );
        assert_eq!(UpdatePromotion::META.kind, Kind::Write);
        assert_eq!(UpdatePromotion::META.method, Method::Patch);
    }

    #[test]
    fn a_plan_round_trips() {
        let f = PromotionFields {
            name: "x".into(),
            valid_from: chrono::DateTime::UNIX_EPOCH,
            valid_through: None,
            applies_to: json!({"scope": "order"}),
        };
        let run = UpdateRun {
            promotion_id: "promo_x".into(),
            before: f.clone(),
            after: PromotionFields {
                name: "y".into(),
                ..f
            },
        };
        let back: UpdateRun = serde_json::from_value(serde_json::to_value(&run).unwrap()).unwrap();
        assert_eq!(back, run);
    }
}
