//! `convert_exchange_to_return`: an exchange that cannot or should not go ahead (the
//! replacement ran out or is discontinued, or the shopper declines it) becomes a return. The
//! exchange closes as `converted`; a new return claim carries over its items, reason, pickup,
//! evidence and inspection and waits in `requested` for `refund_return`. The held replacement
//! is released and any exchange shipping the shopper paid is refunded.

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::Money;
use acom_core::ids::{ClaimId, RefundId};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{claim_lines, claim_not_found, refund_paid_charge, visible_claim};
use crate::errors::{CLAIM_NOT_FOUND, NOT_AN_EXCHANGE, NOT_REVOCABLE};
use crate::model::money_text;
use crate::model::returns::refund_amount;
use crate::repo::{self, ClaimEvent, ClaimRow, NewClaim, NewInspection};
use crate::{broken, rules};

/// Why an exchange becomes a return (docs/api/enums.md §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversionReason {
    /// The replacement ran out before it shipped.
    ReplacementOutOfStock,
    /// The replacement is no longer sold.
    ReplacementDiscontinued,
    /// The shopper no longer wants the replacement.
    CustomerDeclines,
}

impl ConversionReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReplacementOutOfStock => "replacement_out_of_stock",
            Self::ReplacementDiscontinued => "replacement_discontinued",
            Self::CustomerDeclines => "customer_declines",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConvertExchangeToReturnInput {
    /// The exchange claim, from `search_claims` or `get_claim`.
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_3301")]
    pub claim_id: String,
    /// replacement_out_of_stock or replacement_discontinued (the store is then at fault: no
    /// return shipping), or customer_declines (fault unchanged).
    #[schemars(example = &"replacement_out_of_stock")]
    pub reason: ConversionReason,
}

/// The conversion, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConvertPlan {
    pub exchange_id: String,
    pub from_status: String,
    pub return_id: String,
    pub reason: String,
    pub liability: String,
    pub return_fees: String,
    pub refund_estimate: Option<Money>,
    /// `(variant_id, units)` the exchange was holding, released now.
    pub release: Vec<(String, i32)>,
    /// A reshipment parcel that had not shipped yet, cancelled: `(delivery_id, state)`.
    pub parcel_cancelled: Option<(String, String)>,
    /// The exchange's inspection, copied to the return under a new id.
    pub inspection_copy: Option<(String, String, Option<String>, Option<String>)>,
    /// The refund of exchange shipping the shopper paid, when they paid any.
    pub charge_refund: Option<(String, Money)>,
}

pub struct ConvertExchangeToReturn;

impl Operation for ConvertExchangeToReturn {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.convert_exchange_to_return",
        tool: "convert_exchange_to_return",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/fulfillment/claims/{claim_id}/convert-to-return",
        summary: "Turn an exchange into a return when the replacement ran out, is discontinued, or the shopper declines it: the exchange closes as converted and a new return claim (same items, pickup and inspection) waits for refund_return. Tell the shopper with send_customer_message in the same change set.",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::one("claim_id")],
        checks: &["exchange.fallback_to_return_when", "exchange.on_fallback"],
        errors: &[CLAIM_NOT_FOUND, NOT_AN_EXCHANGE, ErrorCode::INVALID_STATE],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = ConvertExchangeToReturnInput;
    type Output = Value;
}

fn state_error(c: &ClaimRow, allowed: &[String], why: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INVALID_STATE,
        format!(
            "Exchange {} is {}: {why}.",
            c.id,
            c.claim_status.replace('_', " ")
        ),
    )
    .detail(
        json!({"entity": "claim", "from": c.claim_status, "allowed_to": ["converted"],
                   "allowed_from": allowed}),
    )
    .next("get_claim")
}

impl WriteOp<Cx> for ConvertExchangeToReturn {
    type Plan = ConvertPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ConvertExchangeToReturnInput,
    ) -> Result<Planned<ConvertPlan>, ToolError> {
        let rules = rules(&cx.policy)?;
        let reason = input.reason.as_str();
        if !rules
            .list("exchange.fallback_to_return_when")
            .iter()
            .any(|r| r == reason)
        {
            return Err(ToolError::invalid_input(format!(
                "The policy does not turn an exchange into a return for {reason}."
            ))
            .detail(json!({"field": "reason", "reason": "enum",
                           "allowed": rules.list("exchange.fallback_to_return_when")})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = visible_claim(&mut tx, &input.claim_id, "search_claims").await?;
        if c.claim_type != "exchange" {
            return Err(ToolError::new(
                NOT_AN_EXCHANGE,
                format!("Claim {} is a {}, not an exchange.", c.id, c.claim_type),
            )
            .detail(json!({"entity": "claim", "claim_type": c.claim_type}))
            .next("get_claim"));
        }
        let allowed = rules.list("exchange.allowed_conversion_states");
        if !allowed.contains(&c.claim_status) {
            return Err(state_error(
                &c,
                &allowed,
                "it can no longer become a return",
            ));
        }
        let deliveries =
            repo::deliveries_of_orders(&mut tx, std::slice::from_ref(&c.order_id)).await?;
        let reship = deliveries.iter().find(|d| {
            d.claim_id.as_deref() == Some(c.id.as_str()) && d.delivery_state != "cancelled"
        });
        let parcel_cancelled = match reship {
            Some(d)
                if matches!(
                    d.delivery_state.as_str(),
                    "awaiting_stock" | "ship_instructed" | "packed"
                ) =>
            {
                Some((d.id.clone(), d.delivery_state.clone()))
            }
            Some(_) => {
                return Err(state_error(
                    &c,
                    &allowed,
                    "the replacement has already shipped (exchange.conversion_window)",
                ));
            }
            None => None,
        };
        let items = repo::claim_items(&mut tx, std::slice::from_ref(&c.id)).await?;
        let mut release = Vec::new();
        let mut replacements = Vec::new();
        for i in &items {
            if let Some(v) = &i.replacement_product_id {
                release.push((v.clone(), i.order_quantity));
                if let Some(var) = catalog::variant(&mut tx, v).await? {
                    replacements.push(var);
                }
            }
        }
        let lines = claim_lines(&mut tx, std::slice::from_ref(&c.id)).await?;
        let inspection = repo::inspection(&mut tx, &c.id).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let liability = match rules
            .lookup(&format!("exchange.on_fallback.liability_after.{reason}"))
            .and_then(Value::as_str)
        {
            Some("unchanged") | None => c.liability.clone(),
            Some(other) => other.to_owned(),
        };
        let fallback_check = match input.reason {
            ConversionReason::CustomerDeclines => Check {
                rule: "exchange.fallback_to_return_when".into(),
                passed: true,
                detail: Some("The shopper declines the replacement".into()),
                fix_tool: None,
            },
            ConversionReason::ReplacementOutOfStock => {
                let left: i32 = replacements
                    .iter()
                    .filter_map(|v| v.stock.as_ref().map(|s| s.inventory_level))
                    .sum();
                let names: Vec<&str> = replacements.iter().map(|v| v.name.as_str()).collect();
                Check {
                    rule: "exchange.fallback_to_return_when".into(),
                    passed: left == 0,
                    detail: Some(if left == 0 {
                        format!("Replacement {} is out of stock", names.join(", "))
                    } else {
                        format!(
                            "Replacement {} still shows {left} in stock besides the unit held",
                            names.join(", ")
                        )
                    }),
                    fix_tool: (left > 0).then(|| "convert_exchange_to_return".into()),
                }
            }
            ConversionReason::ReplacementDiscontinued => {
                let gone = replacements.iter().any(|v| {
                    v.stock
                        .as_ref()
                        .is_none_or(|s| s.availability == "discontinued")
                });
                Check {
                    rule: "exchange.fallback_to_return_when".into(),
                    passed: gone,
                    detail: Some(if gone {
                        "The replacement is no longer sold".into()
                    } else {
                        "The replacement is still sold".into()
                    }),
                    fix_tool: (!gone).then(|| "convert_exchange_to_return".into()),
                }
            }
        };
        let return_fees = rules.return_fees_for(&liability);
        let on_fallback = Check {
            rule: "exchange.on_fallback".into(),
            passed: true,
            detail: Some(format!(
                "{}; tell the shopper with send_customer_message in the same change set",
                if liability == c.liability {
                    format!("Fault stays {}", liability.replace('_', " "))
                } else {
                    format!(
                        "Fault moves to {}: no return shipping charged",
                        liability.replace('_', " ")
                    )
                }
            )),
            fix_tool: Some("send_customer_message".into()),
        };
        let lines = lines.get(&c.id).cloned().unwrap_or_default();
        let refund_estimate = refund_amount(&lines, rules.coupon_deducted);
        let charge_refund = match (c.charge()?, c.charge_paid_at) {
            (Some(m), Some(_)) => Some((RefundId::generate().to_string(), m)),
            _ => None,
        };
        let return_id = ClaimId::generate().to_string();
        let mut changes = vec![
            Change {
                op: ChangeOp::Update,
                entity: "claim".into(),
                id: c.id.clone(),
                from: json!(c.claim_status),
                to: json!("converted"),
                note: None,
            },
            Change {
                op: ChangeOp::Create,
                entity: "claim".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"claim_type": "return", "status": "requested", "liability": liability,
                           "refund_estimate": refund_estimate}),
                note: None,
            },
        ];
        for (variant, units) in &release {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "inventory_level".into(),
                id: variant.clone(),
                from: json!({"reserved": units}),
                to: json!({"reserved": 0}),
                note: None,
            });
        }
        if let Some((d, state)) = &parcel_cancelled {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "parcel_delivery".into(),
                id: d.clone(),
                from: json!(state),
                to: json!("cancelled"),
                note: None,
            });
        }
        if let Some((_, m)) = &charge_refund {
            changes.push(Change {
                op: ChangeOp::Create,
                entity: "refund".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"claim_id": c.id, "amount": m, "why": "exchange shipping paid"}),
                note: Some(format!(
                    "{} exchange shipping back to the shopper",
                    money_text(*m)
                )),
            });
        }
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: ConvertPlan {
                exchange_id: c.id.clone(),
                from_status: c.claim_status.clone(),
                return_id,
                reason: reason.into(),
                liability,
                return_fees,
                refund_estimate,
                release,
                parcel_cancelled,
                inspection_copy: inspection.map(|i| {
                    (
                        format!("ins_{}", acom_core::id::new_suffix()),
                        i.inspection_condition,
                        i.liability,
                        i.description,
                    )
                }),
                charge_refund,
            },
            changes,
            checks: vec![fallback_check, on_fallback],
            basis: vec![Basis {
                entity: "fulfillment.claim".into(),
                id: c.id.clone(),
                version: i64::from(c.version),
            }],
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Turns an exchange into a return".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ConvertPlan) -> Result<Committed, ToolError> {
        let done = Committed {
            id: plan.return_id.clone(),
            produced: Map::from_iter([
                ("claim_id".to_owned(), json!(plan.return_id)),
                ("exchange_claim_id".to_owned(), json!(plan.exchange_id)),
            ]),
        };
        let c = repo::claim_for_update(tx, &plan.exchange_id)
            .await?
            .ok_or_else(|| claim_not_found(&plan.exchange_id, "search_claims"))?;
        if repo::claim(tx, &plan.return_id).await?.is_some() {
            return Ok(done);
        }
        if c.claim_status != plan.from_status {
            return Err(state_error(
                &c,
                &[plan.from_status.clone()],
                "it changed since the preview",
            ));
        }
        let items = repo::claim_items(tx, std::slice::from_ref(&c.id)).await?;
        let tool = Self::META.tool;
        repo::insert_claim(
            tx,
            &NewClaim {
                id: &plan.return_id,
                order_id: &c.order_id,
                customer_id: &c.customer_id,
                claim_type: "return",
                claim_status: "requested",
                claim_path: "store_review",
                claim_reason: c.claim_reason.as_deref(),
                cancel_reason: None,
                reason_text: c.reason_text.as_deref(),
                liability: &plan.liability,
                return_fees: Some(&plan.return_fees),
                send_back_method: c.send_back_method.as_deref(),
                pickup_window: c.pickup_window.clone(),
                evidence: c.evidence.clone(),
                amount: c.amount()?,
                refund_estimate: plan.refund_estimate,
                charge: None,
                items: items
                    .iter()
                    .map(|i| (i.order_item_id.clone(), i.order_quantity, None))
                    .collect(),
            },
        )
        .await?;
        repo::append_event(
            tx,
            &plan.return_id,
            &ClaimEvent {
                action_type: "ReturnAction",
                action_status: "potential",
                instrument: tool,
                from_state: None,
                to_state: Some("requested"),
                result: Some(json!({"converted_from": plan.exchange_id, "reason": plan.reason})),
            },
        )
        .await?;
        if let Some((id, condition, liability, description)) = &plan.inspection_copy {
            let agent = tx.principal.subject.clone();
            let now = tx.clock.now();
            repo::insert_inspection(
                tx,
                &NewInspection {
                    id,
                    claim_id: &plan.return_id,
                    condition,
                    liability: liability.as_deref(),
                    description: description.as_deref(),
                    agent_id: &agent,
                    at: now,
                },
            )
            .await?;
        }
        repo::move_claim(tx, &c.id, &plan.from_status, "converted").await?;
        for (variant, units) in &plan.release {
            catalog::move_stock(tx, variant, *units).await?;
        }
        if let Some((d, state)) = &plan.parcel_cancelled {
            repo::set_delivery_state(tx, d, "cancelled").await?;
            repo::append_delivery_event(
                tx,
                d,
                &ClaimEvent {
                    action_type: "CancelAction",
                    action_status: "completed",
                    instrument: tool,
                    from_state: Some(state),
                    to_state: Some("cancelled"),
                    result: Some(json!({"claim_id": c.id})),
                },
            )
            .await?;
        }
        let charge_refunded = match &plan.charge_refund {
            Some((id, _)) => refund_paid_charge(tx, &c, id, tool).await?,
            None => None,
        };
        repo::append_event(
            tx,
            &c.id,
            &ClaimEvent {
                action_type: "UpdateAction",
                action_status: "completed",
                instrument: tool,
                from_state: Some(&plan.from_status),
                to_state: Some("converted"),
                result: Some(
                    json!({"return_claim_id": plan.return_id, "reason": plan.reason,
                                    "released": plan.release, "charge_refunded": charge_refunded}),
                ),
            },
        )
        .await?;
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ConvertPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let exchange = repo::claim(&mut tx, &plan.exchange_id).await?;
        let ret = repo::claim(&mut tx, &plan.return_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match exchange {
            Some(c) if c.claim_status == "converted" => {}
            Some(c) => mismatches.push(format!("exchange {} is {}", c.id, c.claim_status)),
            None => mismatches.push(format!("exchange {} is missing", plan.exchange_id)),
        }
        if ret.is_none() {
            mismatches.push(format!("return {} is missing", plan.return_id));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ConvertPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        if plan.charge_refund.is_some() {
            return Err(ToolError::new(
                NOT_REVOCABLE,
                format!(
                    "The exchange shipping paid on {} was refunded; a refund cannot be taken back.",
                    plan.exchange_id
                ),
            ));
        }
        if repo::refund_of_claim(tx, &plan.return_id).await?.is_some()
            || !repo::move_claim(tx, &plan.return_id, "requested", "withdrawn").await?
        {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "Return {} has moved on and the exchange cannot come back.",
                    plan.return_id
                ),
            )
            .next("get_claim"));
        }
        if !repo::move_claim(tx, &plan.exchange_id, "converted", &plan.from_status).await? {
            return Err(broken(format!(
                "exchange {} is not converted",
                plan.exchange_id
            )));
        }
        for (variant, units) in &plan.release {
            catalog::move_stock(tx, variant, -units).await?;
        }
        if let Some((d, state)) = &plan.parcel_cancelled {
            repo::set_delivery_state(tx, d, state).await?;
        }
        for (id, from, to) in [
            (&plan.return_id, "requested", "withdrawn"),
            (&plan.exchange_id, "converted", plan.from_status.as_str()),
        ] {
            repo::append_event(
                tx,
                id,
                &ClaimEvent {
                    action_type: "UpdateAction",
                    action_status: "completed",
                    instrument: "undo_change_set",
                    from_state: Some(from),
                    to_state: Some(to),
                    result: Some(json!({"conversion_undone": plan.exchange_id})),
                },
            )
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_are_the_conversion_words() {
        assert!(
            serde_json::from_value::<ConvertExchangeToReturnInput>(
                json!({"claim_id": "clm_3301", "reason": "too_expensive"})
            )
            .is_err()
        );
        let i: ConvertExchangeToReturnInput =
            serde_json::from_value(json!({"claim_id": "clm_3301", "reason": "customer_declines"}))
                .unwrap();
        assert_eq!(i.reason.as_str(), "customer_declines");
    }
}
