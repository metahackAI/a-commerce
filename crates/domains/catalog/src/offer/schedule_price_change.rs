//! `schedule_price_change` (docs/api/catalog.md#offer): a list price for a future period. It is
//! not switched on by a job: price reads compute it from `valid_from`/`valid_through` at the
//! moment they run (`pricing.scheduled_change.compute: on_demand`), and after `valid_through`
//! the offer's own price applies again.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorClass,
    ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_core::{Currency, Money};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::model::{money_of, money_text, num};
use crate::rules::PriceFloorRule;
use crate::write::{self, Schedule};
use crate::{BELOW_PRICE_FLOOR, PRODUCT_NOT_FOUND, parse_time, product_not_found, repo};

/// A scheduled change that would start in the past.
pub const VALID_FROM_IN_PAST: ErrorCode =
    ErrorCode::new("valid_from_in_past", ErrorClass::InvalidInput);

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchedulePriceChangeInput {
    /// The variant.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_windbreaker_m_slate")]
    pub variant_id: String,
    /// The list price for the period, in major units.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", example = 95.00)]
    pub price: Decimal,
    /// ISO 4217 code of `price`; the offer's currency. Default the store's.
    pub price_currency: Option<Currency>,
    /// When the price starts, in store time; must be later than now.
    #[schemars(example = &"2026-12-01T00:00")]
    pub valid_from: String,
    /// When it ends (inclusive); the current list price applies again after. Open-ended when
    /// left out.
    #[schemars(example = &"2026-12-31T23:59")]
    pub valid_through: Option<String>,
}

/// What `schedule_price_change` runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchedulePlan {
    pub schedule_id: String,
    pub variant_id: String,
    pub offer_id: String,
    pub price: Money,
    pub valid_from: DateTime<Utc>,
    pub valid_through: Option<DateTime<Utc>>,
    pub floor_ok: bool,
}

/// Schedule a list price for a later period.
#[derive(Debug)]
pub struct SchedulePriceChange;

impl Operation for SchedulePriceChange {
    const META: OpMeta = OpMeta {
        address: "catalog.offer.schedule_price_change",
        tool: "schedule_price_change",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/variants/{variant_id}/price/schedule",
        summary: "Schedule a variant's list price for a future period. It applies from valid_from by itself, and the current price returns after valid_through. Prices under the floor are refused.",
        consumes: &[Slot::one("variant_id")],
        produces: &[Slot::one("offer_id")],
        checks: &["price_floor", "pricing.scheduled_change"],
        errors: &[BELOW_PRICE_FLOOR, VALID_FROM_IN_PAST, PRODUCT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SCHEDULE,
        person_scope: "",
    };
    type Input = SchedulePriceChangeInput;
    type Output = Value;
}

impl WriteOp<Cx> for SchedulePriceChange {
    type Plan = SchedulePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SchedulePriceChangeInput,
    ) -> Result<Planned<SchedulePlan>, ToolError> {
        let rule = PriceFloorRule::from_policy(&cx.policy)?;
        if input.price.is_sign_negative() {
            return Err(ToolError::invalid_input("`price` cannot be negative.")
                .detail(json!({"field": "price", "reason": "range"})));
        }
        let now = cx.clock.now();
        let from = parse_time(cx, "valid_from", Some(&input.valid_from))?
            .ok_or_else(|| ToolError::invalid_input("`valid_from` is required."))?;
        let through = parse_time(cx, "valid_through", input.valid_through.as_deref())?;
        let start_not_in_past = cx
            .policy
            .lookup("catalog.pricing.scheduled_change.start_not_in_past")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if start_not_in_past && from <= now {
            return Err(ToolError::new(
                VALID_FROM_IN_PAST,
                format!("{} is not later than now ({}).", cx.time.format(from), cx.time.format(now)),
            )
            .detail(json!({"field": "valid_from", "reason": "past", "rule": "pricing.scheduled_change"}))
            .next("schedule_price_change"));
        }
        if through.is_some_and(|t| t <= from) {
            return Err(
                ToolError::invalid_input("`valid_through` must be after `valid_from`.")
                    .detail(json!({"field": "valid_through", "reason": "range"})),
            );
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let state = repo::offer_state(&mut tx, &input.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let s = state
            .ok_or_else(|| product_not_found("variant", &input.variant_id, "search_products"))?;
        let current = money_of(s.price, &s.price_currency)
            .ok_or_else(|| ToolError::new(ErrorCode::INTERNAL, "offer currency is not ISO 4217"))?;
        let currency = input.price_currency.unwrap_or(current.currency);
        if currency != current.currency {
            return Err(ToolError::invalid_input(format!(
                "This offer is priced in {}, not {currency}.",
                current.currency
            ))
            .detail(json!({"field": "price_currency", "reason": "enum", "allowed": [current.currency.to_string()]})));
        }
        let price = Money::new(input.price, currency).rounded();
        let cost = s
            .cost_value
            .zip(s.cost_currency.as_deref())
            .and_then(|(v, c)| money_of(v, c));
        let floor_check = match cost {
            Some(c) => {
                let ok = rule.allows(price, c);
                Check {
                    rule: "price_floor".into(),
                    passed: ok,
                    detail: Some(format!(
                        "{} is {} the floor {} ({})",
                        money_text(&price),
                        if ok { "at or above" } else { "below" },
                        money_text(&rule.floor(c)),
                        rule.markup_text().replacen("cost", &money_text(&c), 1)
                    )),
                    fix_tool: (!ok).then(|| "schedule_price_change".into()),
                }
            }
            None => Check {
                rule: "price_floor".into(),
                passed: true,
                detail: Some("No cost on file, so there is no floor to check".into()),
                fix_tool: None,
            },
        };
        let period = match through {
            Some(t) => format!("{} to {}", cx.time.format(from), cx.time.format(t)),
            None => format!("from {}", cx.time.format(from)),
        };
        let schedule_id = format!(
            "ups_{}_{}",
            s.offer_id.trim_start_matches("off_"),
            from.with_timezone(&cx.time.zone).format("%Y%m%d%H%M")
        );
        let approval = cx.policy.approval_for(
            Self::META.tool,
            &json!({"changed_offers": 1, "change_type": "structural"}),
        );
        let plan = SchedulePlan {
            schedule_id,
            variant_id: s.variant_id.clone(),
            offer_id: s.offer_id.clone(),
            price,
            valid_from: from,
            valid_through: through,
            floor_ok: floor_check.passed,
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Schedule,
                entity: "unit_price_specification".into(),
                id: String::new(),
                from: json!({"price": num(current.value)}),
                to: json!({"offer_id": s.offer_id, "price": num(price.value),
                           "price_currency": price.currency.to_string(), "price_type": "list_price",
                           "valid_from": cx.time.format(from),
                           "valid_through": through.map(|t| cx.time.format(t))}),
                note: Some(format!(
                    "List price {} {period}; {} again after",
                    money_text(&price),
                    money_text(&current)
                )),
            }],
            checks: vec![
                floor_check,
                Check {
                    rule: "pricing.scheduled_change".into(),
                    passed: true,
                    detail: Some(format!("Starts later than now; applied on demand {period}")),
                    fix_tool: None,
                },
            ],
            basis: vec![Basis {
                entity: "catalog.offer".into(),
                id: s.offer_id,
                version: i64::from(s.version),
            }],
            approval: match approval.reason {
                Some(reason) if approval.needed => ApprovalNeed::Needed { reason },
                _ => ApprovalNeed::NotNeeded,
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SchedulePlan) -> Result<Committed, ToolError> {
        if !plan.floor_ok {
            return Err(ToolError::new(
                BELOW_PRICE_FLOOR,
                format!(
                    "At {} {} would sell below its price floor.",
                    money_text(&plan.price),
                    plan.variant_id
                ),
            )
            .detail(json!({"rule": "price_floor", "items": [plan.variant_id]}))
            .next("schedule_price_change"));
        }
        write::insert_schedule(
            tx,
            &Schedule {
                id: plan.schedule_id.clone(),
                offer_id: plan.offer_id.clone(),
                price: plan.price.value,
                price_currency: plan.price.currency.to_string(),
                valid_from: plan.valid_from,
                valid_through: plan.valid_through,
            },
        )
        .await?;
        let mut produced = Map::new();
        produced.insert("offer_id".into(), json!(plan.offer_id));
        produced.insert(
            "unit_price_specification_id".into(),
            json!(plan.schedule_id),
        );
        Ok(Committed {
            id: plan.offer_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SchedulePlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let all = write::schedules(&mut tx, std::slice::from_ref(&plan.offer_id)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = all.iter().any(|s| {
            s.id == plan.schedule_id
                && s.price == plan.price.value
                && s.valid_from == plan.valid_from
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not scheduled", plan.schedule_id)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SchedulePlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        write::delete_schedule(tx, &plan.schedule_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    async fn a_start_in_the_past_is_refused() {
        let e = SchedulePriceChange
            .preview(
                &testing::cx(acom_core::enums::ActorKind::Operator, "adm_sj"),
                serde_json::from_value(
                    json!({"variant_id": "var_windbreaker_m_slate", "price": 95.00,
                    "valid_from": "2026-11-01T00:00"}),
                )
                .unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("valid_from_in_past", Some("schedule_price_change"))
        );
        let schema = acom_contract::schema::closed_schema_for::<SchedulePriceChangeInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }
}
