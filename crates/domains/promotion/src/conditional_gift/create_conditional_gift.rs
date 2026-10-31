//! `create_conditional_gift` (docs/api/promotion.md#conditional_gift).

use acom_catalog::service::for_promotion::products_with_offers;
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::SalesChannel;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{GiftTier, gift};
use crate::model::money_text;
use crate::repo::{self, db};
use crate::{INVALID_PERIOD, TIER_NOT_FOUND, audience, known_tier, not_found, parse_time, rules};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateConditionalGiftInput {
    /// Name people read.
    #[schemars(length(min = 1, max = 100), example = &"Holiday gift over $120")]
    pub name: String,
    /// The thresholds in ascending order, each with its gift.
    #[schemars(length(min = 1, max = 10), example = json!([{"trigger_threshold": {"value": 120.00, "currency": "USD"}, "gift_product_id": "prd_merino_tee"}]))]
    pub tiers: Vec<GiftTier>,
    /// A tier id (`list_tiers`) or `everyone` (default).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"everyone")]
    pub valid_for_member_tier: Option<String>,
    /// Where it applies: web, mobile_web, app, agent. Default: every channel.
    #[schemars(example = ["web", "app"])]
    pub channels: Option<Vec<SalesChannel>>,
    /// Start (inclusive). Without an offset it is read in store time.
    #[schemars(example = &"2026-11-20T00:00")]
    pub valid_from: String,
    /// End (inclusive, minute precision), after `valid_from`.
    #[schemars(example = &"2026-11-30T23:59")]
    pub valid_through: String,
}

/// What a run of `create_conditional_gift` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GiftRun {
    pub id: String,
    pub name: String,
    pub tiers: Vec<GiftTier>,
    pub valid_for_member_tier: Option<String>,
    pub channels: Option<Vec<String>>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
}

pub struct CreateConditionalGift;

impl Operation for CreateConditionalGift {
    const META: OpMeta = OpMeta {
        address: "promotion.conditional_gift.create",
        tool: "create_conditional_gift",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/conditional-gifts",
        summary: "Add a gift product to orders whose subtotal before member benefits reaches a threshold, with a different gift per threshold if wanted, for a period.",
        consumes: &[Slot::many("product_id"), Slot::one("tier_id")],
        produces: &[Slot::one("conditional_gift_id")],
        checks: &[
            "conditional_gift.trigger",
            "conditional_gift.tiers_by_amount",
        ],
        errors: &[ErrorCode::NOT_FOUND, INVALID_PERIOD, TIER_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateConditionalGiftInput;
    type Output = Value;
}

impl WriteOp<Cx> for CreateConditionalGift {
    type Plan = GiftRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateConditionalGiftInput,
    ) -> Result<Planned<GiftRun>, ToolError> {
        let name = input.name.trim().to_owned();
        if name.is_empty() {
            return Err(ToolError::invalid_input("`name` must not be empty.")
                .detail(json!({"field": "name", "reason": "missing"})));
        }
        let from = parse_time(&cx.time, "valid_from", &input.valid_from)?;
        let through = parse_time(&cx.time, "valid_through", &input.valid_through)?;
        if from >= through {
            return Err(ToolError::new(
                INVALID_PERIOD,
                "`valid_from` must come before `valid_through`.",
            )
            .detail(json!({"field": "valid_from", "reason": "range"}))
            .next("create_conditional_gift"));
        }
        let by_amount = rules::flag(&cx.policy, "promotion.conditional_gift.tiers_by_amount")?;
        let measured = rules::word(&cx.policy, "promotion.conditional_gift.trigger.measured")?;
        let op = rules::word(&cx.policy, "promotion.conditional_gift.trigger.op")?;
        if input.tiers.is_empty() || (!by_amount && input.tiers.len() > 1) {
            return Err(ToolError::invalid_input(if by_amount {
                "Give at least one threshold."
            } else {
                "This store gives one gift per promotion (conditional_gift.tiers_by_amount is false)."
            })
            .detail(json!({"field": "tiers", "reason": "range"})));
        }
        for (i, t) in input.tiers.iter().enumerate() {
            crate::check_id("tiers.gift_product_id", "prd", &t.gift_product_id, false)?;
            if t.trigger_threshold.currency != cx.currency
                || t.trigger_threshold.value <= rust_decimal::Decimal::ZERO
            {
                return Err(ToolError::invalid_input(format!(
                    "Each threshold must be above zero in {}.",
                    cx.currency
                ))
                .detail(json!({"field": "tiers.trigger_threshold", "reason": "range"})));
            }
            if i > 0 && t.trigger_threshold.value <= input.tiers[i - 1].trigger_threshold.value {
                return Err(ToolError::invalid_input(
                    "Thresholds must go up from one tier to the next.",
                )
                .detail(json!({"field": "tiers.trigger_threshold", "reason": "range"})));
            }
        }
        let channels: Option<Vec<String>> = input
            .channels
            .map(|l| {
                let mut w: Vec<String> = l.iter().map(|c| c.as_str().to_owned()).collect();
                w.sort();
                w.dedup();
                w
            })
            .filter(|l| !l.is_empty());
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let tier = known_tier(
            &mut tx,
            input.valid_for_member_tier.as_deref().unwrap_or("everyone"),
        )
        .await?;
        let ids: Vec<String> = input
            .tiers
            .iter()
            .map(|t| t.gift_product_id.clone())
            .collect();
        let products = products_with_offers(&mut tx, &ids).await?;
        if let Some(missing) = ids
            .iter()
            .find(|id| !products.iter().any(|p| &p.product_id == *id))
        {
            return Err(not_found("product", missing, "search_products"));
        }
        let readable = format!("cgf_{}", crate::segment::slug(&name));
        let id = if readable.len() > 4 && gift(&mut tx, &readable).await?.is_none() {
            readable
        } else {
            repo::new_id("cgf")
        };
        tx.commit().await.map_err(ToolError::from)?;
        let tiers_json: Vec<Value> = input
            .tiers
            .iter()
            .map(|t| json!({"trigger_threshold": crate::money(t.trigger_threshold.value, t.trigger_threshold.currency.as_str()),
                            "gift_product_id": t.gift_product_id}))
            .collect();
        let gifts: Vec<String> = input
            .tiers
            .iter()
            .map(|t| {
                let name = products
                    .iter()
                    .find(|p| p.product_id == t.gift_product_id)
                    .map_or("", |p| p.name.as_str());
                format!(
                    "{name} from {}",
                    money_text(
                        t.trigger_threshold.value,
                        t.trigger_threshold.currency.as_str()
                    )
                )
            })
            .collect();
        let checks = vec![
            Check {
                rule: "conditional_gift.trigger".into(),
                passed: true,
                detail: Some(format!(
                    "Added when the {} is {} a threshold",
                    measured.replace('_', " "),
                    if op == "gte" { "at or above" } else { "above" }
                )),
                fix_tool: None,
            },
            Check {
                rule: "conditional_gift.tiers_by_amount".into(),
                passed: true,
                detail: Some(gifts.join("; ")),
                fix_tool: None,
            },
        ];
        let tier_id = tier.map(|t| t.id);
        let approval = cx
            .policy
            .approval_for(Self::META.tool, &json!({"reaches_customers": true}));
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "conditional_gift".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"name": name, "tiers": tiers_json,
                           "valid_for_member_tier": audience(tier_id.as_deref()),
                           "valid_from": cx.time.format(from), "valid_through": cx.time.format(through)}),
                note: None,
            }],
            plan: GiftRun {
                id,
                name,
                tiers: input.tiers,
                valid_for_member_tier: tier_id,
                channels,
                valid_from: from,
                valid_through: through,
            },
            checks,
            basis: Vec::new(),
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval.reason.unwrap_or_default(),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &GiftRun) -> Result<Committed, ToolError> {
        let change_set_id = tx.change_set_id.clone();
        sqlx::query(
            "INSERT INTO promotion.conditional_gift (tenant_id, id, name, gift_tiers,
               valid_for_member_tier, sales_channels, valid_from, valid_through, change_set_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (tenant_id, id) DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&run.id)
        .bind(&run.name)
        .bind(serde_json::to_value(&run.tiers).unwrap_or_default())
        .bind(&run.valid_for_member_tier)
        .bind(&run.channels)
        .bind(run.valid_from)
        .bind(run.valid_through)
        .bind(change_set_id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        Ok(Committed {
            id: run.id.clone(),
            produced: serde_json::Map::from_iter([(
                "conditional_gift_id".to_owned(),
                json!(run.id),
            )]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &GiftRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let g = gift(&mut tx, &run.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = g.is_some_and(|g| {
            g.valid_from == run.valid_from
                && g.valid_through == run.valid_through
                && serde_json::from_value::<Vec<GiftTier>>(g.gift_tiers)
                    .ok()
                    .as_ref()
                    == Some(&run.tiers)
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} differs from the plan", run.id)]
            },
        })
    }

    async fn undo(&self, tx: &mut TxCx, run: &GiftRun, _: &Committed) -> Result<(), ToolError> {
        sqlx::query("DELETE FROM promotion.conditional_gift WHERE tenant_id = $1 AND id = $2")
            .bind(tx.tenant.as_str())
            .bind(&run.id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn takes_the_spec_inputs() {
        let s = closed_schema_for::<CreateConditionalGiftInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        let i: CreateConditionalGiftInput = serde_json::from_value(json!({"name": "Holiday gift over $120",
            "tiers": [{"trigger_threshold": {"value": 120.00, "currency": "USD"}, "gift_product_id": "prd_merino_tee"}],
            "valid_from": "2026-11-20T00:00", "valid_through": "2026-11-30T23:59"})).unwrap();
        assert_eq!(i.tiers.len(), 1);
        assert_eq!(CreateConditionalGift::META.kind, Kind::Write);
    }
}
