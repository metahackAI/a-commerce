//! `set_shipping_rate`: what shipping costs in one zone (paid, free, or free over a minimum).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::ShippingFeeType;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::zone_not_found;
use crate::errors::{INVALID_THRESHOLD, ZONE_NOT_FOUND};
use crate::model::money_text;
use crate::repo::{self, ZoneValues};
use crate::rules;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetShippingRateInput {
    /// The zone, from `list_shipping_zones`.
    #[schemars(regex(pattern = r"^zone_[a-z0-9_]+$"), example = &"zone_us_contiguous")]
    pub zone_id: String,
    /// Shipping charged per parcel when it is not free, in the store's currency.
    #[schemars(example = json!({"value": 6.00, "currency": "USD"}))]
    pub shipping_rate: Money,
    /// paid: always charged; free: never charged; free_over_min_subtotal: free from
    /// `free_over` (subtotal before member benefits).
    #[schemars(example = &"free_over_min_subtotal")]
    pub fee_type: ShippingFeeType,
    /// The subtotal from which shipping is free; only with free_over_min_subtotal.
    #[schemars(example = json!({"value": 80.00, "currency": "USD"}))]
    pub free_over: Option<Money>,
}

/// The zone before and after, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RatePlan {
    pub zone_id: String,
    pub before: ZoneValues,
    pub after: ZoneValues,
}

pub struct SetShippingRate;

impl Operation for SetShippingRate {
    const META: OpMeta = OpMeta {
        address: "fulfillment.shipping_rate.set",
        tool: "set_shipping_rate",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/fulfillment/shipping-zones/{zone_id}/rate",
        summary: "Set what shipping costs in one delivery zone: a paid rate, free, or free over a minimum subtotal.",
        consumes: &[Slot::one("zone_id")],
        produces: &[Slot::one("zone_id")],
        checks: &["delivery.shipping_fee_types", "free_shipping_min_subtotal"],
        errors: &[INVALID_THRESHOLD, ZONE_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetShippingRateInput;
    type Output = Value;
}

fn rate_wire(z: &ZoneValues) -> Value {
    json!({"fee_type": z.fee_type, "shipping_rate": z.shipping_rate, "free_over": z.free_over})
}

fn threshold(message: impl Into<String>) -> ToolError {
    ToolError::new(INVALID_THRESHOLD, message)
        .detail(json!({"field": "free_over", "reason": "range"}))
        .next("set_shipping_rate")
}

impl WriteOp<Cx> for SetShippingRate {
    type Plan = RatePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetShippingRateInput,
    ) -> Result<Planned<RatePlan>, ToolError> {
        let rules = rules(&cx.policy)?;
        let rate = input.shipping_rate.rounded();
        if rate.currency != cx.currency || rate.value.is_sign_negative() {
            return Err(ToolError::invalid_input(format!(
                "The rate must be 0 or more, in {}.",
                cx.currency
            ))
            .detail(json!({"field": "shipping_rate", "reason": "range"})));
        }
        let free_over = match (input.fee_type, input.free_over) {
            (ShippingFeeType::FreeOverMinSubtotal, None) => {
                return Err(threshold(
                    "Say from which subtotal shipping is free (free_over).",
                ));
            }
            (ShippingFeeType::FreeOverMinSubtotal, Some(m))
                if m.currency != cx.currency || m.value <= rust_decimal::Decimal::ZERO =>
            {
                return Err(threshold(format!(
                    "The free-shipping minimum must be more than 0, in {}.",
                    cx.currency
                )));
            }
            (ShippingFeeType::FreeOverMinSubtotal, Some(m)) => Some(m.rounded()),
            (_, Some(_)) => {
                return Err(threshold(
                    "free_over only applies with fee_type free_over_min_subtotal.",
                ));
            }
            (_, None) => None,
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::zone(&mut tx, &input.zone_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let row = row.ok_or_else(|| zone_not_found(&input.zone_id, "list_shipping_zones"))?;
        let before = row.values()?;
        let after = ZoneValues {
            fee_type: input.fee_type.as_str().to_owned(),
            shipping_rate: rate,
            free_over,
            ..before.clone()
        };

        let offered = rules.list("delivery.shipping_fee_types");
        let fee_ok = offered.iter().any(|f| f == input.fee_type.as_str());
        let store_min = rules.free_shipping_min;
        let (min_ok, min_detail) = match free_over {
            Some(m) if m == store_min => (true, format!("Free over {} kept", money_text(m))),
            Some(m) => (
                false,
                format!(
                    "Checkout makes shipping free from {} (order.yml free_shipping_min_subtotal); this zone would say {}",
                    money_text(store_min),
                    money_text(m)
                ),
            ),
            None if input.fee_type == ShippingFeeType::Free => {
                (true, format!("Shipping is free in {}", before.name))
            }
            None => (
                true,
                format!(
                    "{} per parcel in {}, never free",
                    money_text(rate),
                    before.name
                ),
            ),
        };
        let checks = vec![
            Check {
                rule: "delivery.shipping_fee_types".into(),
                passed: fee_ok,
                detail: Some(if fee_ok {
                    format!(
                        "{} is a fee type the store uses",
                        input.fee_type.as_str().replace('_', " ")
                    )
                } else {
                    format!("The store's policy offers only {}", offered.join(", "))
                }),
                fix_tool: (!fee_ok).then(|| "set_shipping_rate".into()),
            },
            Check {
                rule: "free_shipping_min_subtotal".into(),
                passed: min_ok,
                detail: Some(min_detail),
                fix_tool: (!min_ok).then(|| "set_shipping_rate".into()),
            },
        ];
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: RatePlan {
                zone_id: row.id.clone(),
                before: before.clone(),
                after: after.clone(),
            },
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "shipping_zone".into(),
                id: row.id.clone(),
                from: rate_wire(&before),
                to: rate_wire(&after),
                note: None,
            }],
            checks,
            basis: vec![Basis {
                entity: "fulfillment.shipping_zone".into(),
                id: row.id,
                version: i64::from(row.version),
            }],
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Changes a shipping rate".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &RatePlan) -> Result<Committed, ToolError> {
        if !repo::update_zone(tx, &plan.zone_id, &plan.after).await? {
            return Err(zone_not_found(&plan.zone_id, "list_shipping_zones"));
        }
        Ok(Committed {
            id: plan.zone_id.clone(),
            produced: Map::from_iter([("zone_id".to_owned(), json!(plan.zone_id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RatePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::zone(&mut tx, &plan.zone_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            None => vec![format!("zone {} is missing", plan.zone_id)],
            Some(r) => {
                let v = r.values()?;
                if (v.fee_type.as_str(), v.shipping_rate, v.free_over)
                    == (
                        plan.after.fee_type.as_str(),
                        plan.after.shipping_rate,
                        plan.after.free_over,
                    )
                {
                    Vec::new()
                } else {
                    vec![format!("zone {} has another rate", r.id)]
                }
            }
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, tx: &mut TxCx, plan: &RatePlan, _: &Committed) -> Result<(), ToolError> {
        let restored = ZoneValues {
            fee_type: plan.before.fee_type.clone(),
            shipping_rate: plan.before.shipping_rate,
            free_over: plan.before.free_over,
            ..match repo::zone(tx, &plan.zone_id).await? {
                Some(r) => r.values()?,
                None => return Err(zone_not_found(&plan.zone_id, "list_shipping_zones")),
            }
        };
        repo::update_zone(tx, &plan.zone_id, &restored)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    #[tokio::test]
    async fn a_free_over_rate_needs_its_minimum() {
        let e = SetShippingRate
            .preview(
                &offline("adm_sj"),
                input(
                    json!({"zone_id": "zone_us_contiguous", "fee_type": "free_over_min_subtotal",
                             "shipping_rate": {"value": 6.00, "currency": "USD"}}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_threshold");
        assert_eq!(e.next_tool.as_deref(), Some("set_shipping_rate"));
        let e = SetShippingRate
            .preview(
                &offline("adm_sj"),
                input(json!({"zone_id": "zone_us_contiguous", "fee_type": "paid",
                             "shipping_rate": {"value": 6.00, "currency": "USD"},
                             "free_over": {"value": 80.00, "currency": "USD"}})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_threshold");
    }
}
