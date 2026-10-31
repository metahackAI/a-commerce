//! `approve_claim`: the operator approves claims waiting in the store's queue (always a
//! person's approval in a change set).

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::Money;
use acom_core::enums::Liability;
use acom_core::ids::RefundId;
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    ApproveStep, PlannedRefund, apply_effects, cancel_block_error, cancel_effects, cancel_view,
    claim_not_found, invalid_state, pays_sentence, visible_claim,
};
use crate::errors::{
    ALREADY_HANDED_OVER, CHARGE_UNPAID, CLAIM_NOT_FOUND, INSPECTION_PENDING,
    LIABILITY_COST_CONFLICT, NOT_REVOCABLE, REPLACEMENT_UNAVAILABLE,
};
use crate::model::cancel::{CancelAsk, CancelBlock, cancel_outcome};
use crate::model::money_text;
use crate::repo::{self, ClaimEvent, ClaimRow};
use crate::{broken, rules};

/// A shipping cost decided by the operator.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostOverride {
    /// Shipping charged to the shopper instead of the computed charge (0.00 waives it).
    #[schemars(example = json!({"value": 0.00, "currency": "USD"}))]
    pub amount: Money,
    /// Why, in a sentence. Required.
    #[schemars(length(min = 1, max = 500), example = &"Courtesy: first cancel for this customer.")]
    pub note: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApproveClaimInput {
    /// Claims to approve together: cancels waiting for the store, exchanges and returns
    /// waiting for their pickup. Return refunds go through refund_return instead.
    #[schemars(length(min = 1, max = 50), inner(regex(pattern = r"^clm_[a-z0-9_]+$")), example = ["clm_4410"])]
    pub claim_ids: Vec<String>,
    /// The claim named in the REST path (`/claims/{claim_id}/approve`); it joins claim_ids.
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_4410")]
    pub claim_id: Option<String>,
    /// Who is at fault, when the operator decides differently from the request; shipping and
    /// restores are computed again.
    #[schemars(example = &"store_fault")]
    pub liability: Option<Liability>,
    /// A shipping charge decided by the operator, with the reason.
    pub cost_override: Option<CostOverride>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovePlan {
    pub steps: Vec<ApproveStep>,
}

pub struct ApproveClaim;

impl Operation for ApproveClaim {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.approve",
        tool: "approve_claim",
        group: Group::Transact,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/fulfillment/claims/{claim_id}/approve",
        summary: "Approve claims waiting in the store's queue: a cancel stops the shipment, puts stock and coupons back and refunds; an exchange or return gets its pickup. Always approved by a person in a change set.",
        consumes: &[Slot::many("claim_id")],
        produces: &[Slot::many("claim_id")],
        checks: &[
            "liability.pays",
            "cancel.operator_only_states",
            "cancel.shipping_recalculation",
            "cancel.restore",
            "refund.to",
            "exchange.replacement",
            "return.inventory_restore_when",
            "change_set.irreversible",
        ],
        errors: &[
            CLAIM_NOT_FOUND,
            LIABILITY_COST_CONFLICT,
            ALREADY_HANDED_OVER,
            REPLACEMENT_UNAVAILABLE,
            INSPECTION_PENDING,
            CHARGE_UNPAID,
            ErrorCode::INVALID_STATE,
        ],
        audience: Audience::Operator,
        action_type: ActionType::AUTHORIZE,
        person_scope: "",
    };
    type Input = ApproveClaimInput;
    type Output = Value;
}

impl WriteOp<Cx> for ApproveClaim {
    type Plan = ApprovePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ApproveClaimInput,
    ) -> Result<Planned<ApprovePlan>, ToolError> {
        let rules = rules(&cx.policy)?;
        let now = cx.clock.now();
        let liability = input.liability.map(Liability::as_str);
        if let Some(o) = &input.cost_override {
            if o.note.trim().is_empty() {
                return Err(
                    ToolError::invalid_input("`cost_override.note` must say why.")
                        .detail(json!({"field": "cost_override.note", "reason": "missing"})),
                );
            }
            if liability.is_some_and(|l| l != "customer_fault") && !o.amount.value.is_zero() {
                return Err(ToolError::new(
                    LIABILITY_COST_CONFLICT,
                    "When the store is at fault the shopper pays no shipping; set the charge to 0.00 or change who is at fault.",
                )
                .detail(json!({"rule": "liability.pays"}))
                .next("approve_claim"));
            }
        }
        let mut steps = Vec::new();
        let mut changes = Vec::new();
        let mut checks = Vec::new();
        let mut basis = Vec::new();
        let mut reasons = Vec::new();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mut claim_ids = input.claim_ids.clone();
        if let Some(id) = &input.claim_id
            && !claim_ids.contains(id)
        {
            claim_ids.insert(0, id.clone());
        }
        for claim_id in &claim_ids {
            let c = visible_claim(&mut tx, claim_id, "search_claims").await?;
            basis.push(Basis {
                entity: "fulfillment.claim".into(),
                id: c.id.clone(),
                version: i64::from(c.version),
            });
            match c.claim_status.as_str() {
                "requested" => {}
                "charge_due" => {
                    return Err(ToolError::new(
                        CHARGE_UNPAID,
                        format!("The shopper has not paid the extra charge on {} yet.", c.id),
                    )
                    .detail(json!({"entity": "claim", "from": c.claim_status}))
                    .next("get_claim"));
                }
                "picked_up" if c.claim_type == "exchange" => {
                    return Err(ToolError::new(
                        INSPECTION_PENDING,
                        format!(
                            "Exchange {} is back at the warehouse; inspect it first.",
                            c.id
                        ),
                    )
                    .detail(json!({"entity": "claim", "from": c.claim_status}))
                    .next("assess_return_condition"));
                }
                _ => return Err(invalid_state(&c, &["approved", "pickup_instructed"])),
            }
            let liability_change = liability
                .filter(|l| *l != c.liability)
                .map(|l| (c.liability.clone(), l.to_owned()));
            let now_liability = liability.unwrap_or(&c.liability).to_owned();
            if let Some((from, to)) = &liability_change {
                checks.push(Check {
                    rule: "liability.pays".into(),
                    passed: true,
                    detail: Some(pays_sentence(&rules, to)),
                    fix_tool: None,
                });
                changes.push(Change {
                    op: ChangeOp::Update,
                    entity: "claim".into(),
                    id: c.id.clone(),
                    from: json!({"liability": from}),
                    to: json!({"liability": to}),
                    note: None,
                });
            }
            let step = match c.claim_type.as_str() {
                "cancel" => {
                    let (s, units) = plan_cancel(
                        &mut tx,
                        &rules,
                        &c,
                        &now_liability,
                        input.cost_override.as_ref(),
                        now,
                        (&mut changes, &mut checks, &mut basis),
                    )
                    .await?;
                    reasons.push(match &s.refund {
                        Some(r) => format!(
                            "Cancels {} and refunds {}",
                            items_text(units),
                            money_text(r.amount)
                        ),
                        None => format!("Cancels the items on claim {}", c.id),
                    });
                    ApproveStep {
                        liability_change,
                        ..s
                    }
                }
                other => {
                    let items = repo::claim_items(&mut tx, std::slice::from_ref(&c.id)).await?;
                    if other == "exchange" {
                        for i in &items {
                            let Some(var) = &i.replacement_product_id else {
                                continue;
                            };
                            let v = catalog::variant(&mut tx, var).await?;
                            let ok = v.as_ref().is_some_and(|v| {
                                v.stock
                                    .as_ref()
                                    .is_some_and(|s| s.availability != "discontinued")
                            });
                            if !ok {
                                return Err(ToolError::new(
                                    REPLACEMENT_UNAVAILABLE,
                                    format!("The replacement {var} is no longer sold."),
                                )
                                .detail(json!({"variant_id": var}))
                                .next("convert_exchange_to_return"));
                            }
                            checks.push(Check {
                                rule: "exchange.replacement".into(),
                                passed: true,
                                detail: Some(format!(
                                    "{} is held for this exchange",
                                    v.map(|v| v.name).unwrap_or_else(|| var.clone())
                                )),
                                fix_tool: Some("convert_exchange_to_return".into()),
                            });
                        }
                    }
                    if liability_change.is_none() {
                        checks.push(Check {
                            rule: "liability.pays".into(),
                            passed: true,
                            detail: Some(pays_sentence(&rules, &now_liability)),
                            fix_tool: None,
                        });
                    }
                    checks.push(Check {
                        rule: "return.inventory_restore_when".into(),
                        passed: true,
                        detail: Some(
                            "The item goes back to stock only if it is inspected as new".into(),
                        ),
                        fix_tool: None,
                    });
                    changes.push(Change {
                        op: ChangeOp::Update,
                        entity: "claim".into(),
                        id: c.id.clone(),
                        from: json!(c.claim_status),
                        to: json!("pickup_instructed"),
                        note: None,
                    });
                    reasons.push(format!("Starts the {other} pickup for claim {}", c.id));
                    ApproveStep {
                        claim_id: c.id.clone(),
                        claim_type: c.claim_type.clone(),
                        order_id: c.order_id.clone(),
                        from_status: c.claim_status.clone(),
                        to_status: "pickup_instructed".into(),
                        liability_change,
                        lines_cancelled: Vec::new(),
                        restock: Vec::new(),
                        deliveries_cancelled: Vec::new(),
                        coupons_restored: Vec::new(),
                        order_cancelled_from: None,
                        refund: None,
                    }
                }
            };
            steps.push(step);
        }
        tx.commit().await.map_err(ToolError::from)?;
        if steps.iter().any(|s| s.refund.is_some()) {
            checks.push(Check {
                rule: "change_set.irreversible".into(),
                passed: true,
                detail: Some("The refund can't be undone once it is sent".into()),
                fix_tool: None,
            });
        }
        Ok(Planned {
            plan: ApprovePlan { steps },
            changes,
            checks,
            basis,
            approval: ApprovalNeed::Needed {
                reason: reasons.join("; "),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ApprovePlan) -> Result<Committed, ToolError> {
        let mut refunds = Vec::new();
        for s in &plan.steps {
            let c = repo::claim_for_update(tx, &s.claim_id)
                .await?
                .ok_or_else(|| claim_not_found(&s.claim_id, "search_claims"))?;
            if let Some(r) = &s.refund {
                refunds.push(r.refund_id.clone());
            }
            if c.claim_status == s.to_status {
                continue;
            }
            if c.claim_status != s.from_status {
                return Err(invalid_state(&c, &[s.to_status.as_str()]));
            }
            repo::move_claim(tx, &s.claim_id, &s.from_status, &s.to_status).await?;
            apply_effects(tx, s, Self::META.tool).await?;
            repo::append_event(
                tx,
                &s.claim_id,
                &ClaimEvent {
                    action_type: "AuthorizeAction",
                    action_status: "completed",
                    instrument: Self::META.tool,
                    from_state: Some(&s.from_status),
                    to_state: Some(&s.to_status),
                    result: Some(json!({"refund_id": s.refund.as_ref().map(|r| &r.refund_id),
                                        "liability": s.liability_change.as_ref().map(|l| &l.1)})),
                },
            )
            .await?;
        }
        let ids: Vec<&String> = plan.steps.iter().map(|s| &s.claim_id).collect();
        let first = plan
            .steps
            .first()
            .map(|s| s.claim_id.clone())
            .ok_or_else(|| broken("approve_claim plan has no claims"))?;
        Ok(Committed {
            id: first,
            produced: Map::from_iter([
                ("claim_id".to_owned(), json!(ids)),
                ("refund_id".to_owned(), json!(refunds)),
            ]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ApprovePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut mismatches = Vec::new();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        for s in &plan.steps {
            match repo::claim(&mut tx, &s.claim_id).await? {
                Some(c) if c.claim_status == s.to_status || !c.is_open() => {}
                Some(c) => mismatches.push(format!("claim {} is {}", c.id, c.claim_status)),
                None => mismatches.push(format!("claim {} is missing", s.claim_id)),
            }
            if let Some(r) = &s.refund
                && repo::refund(&mut tx, &r.refund_id).await?.is_none()
            {
                mismatches.push(format!("refund {} is missing", r.refund_id));
            }
        }
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ApprovePlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        if let Some(s) = plan.steps.iter().find(|s| s.refund.is_some()) {
            return Err(ToolError::new(
                NOT_REVOCABLE,
                format!(
                    "Claim {} was refunded; a refund cannot be taken back.",
                    s.claim_id
                ),
            ));
        }
        for s in plan.steps.iter().rev() {
            if !repo::move_claim(tx, &s.claim_id, &s.to_status, &s.from_status).await? {
                return Err(ToolError::new(
                    ErrorCode::INVALID_STATE,
                    format!(
                        "Claim {} has moved on and cannot be taken back.",
                        s.claim_id
                    ),
                )
                .next("get_claim"));
            }
            if let Some((from, _)) = &s.liability_change {
                repo::set_liability(tx, &s.claim_id, from).await?;
            }
            for (line, before) in &s.lines_cancelled {
                sales::set_item_status(tx, line, before).await?;
            }
            for (variant, units) in &s.restock {
                catalog::move_stock(tx, variant, -units).await?;
            }
            for (delivery, before) in &s.deliveries_cancelled {
                repo::set_delivery_state(tx, delivery, before).await?;
            }
            if let Some(before) = &s.order_cancelled_from {
                sales::set_order_status(tx, &s.order_id, before, "UpdateAction", "undo_change_set")
                    .await?;
            }
            repo::append_event(
                tx,
                &s.claim_id,
                &ClaimEvent {
                    action_type: "UpdateAction",
                    action_status: "completed",
                    instrument: "undo_change_set",
                    from_state: Some(&s.to_status),
                    to_state: Some(&s.from_status),
                    result: None,
                },
            )
            .await?;
        }
        Ok(())
    }
}

fn items_text(n: i32) -> String {
    if n == 1 {
        "1 item".to_owned()
    } else {
        format!("{n} items")
    }
}

type Sinks<'a> = (&'a mut Vec<Change>, &'a mut Vec<Check>, &'a mut Vec<Basis>);

/// The approval of one cancel claim: what stops, what comes back, the refund.
async fn plan_cancel(
    tx: &mut TxCx,
    rules: &crate::model::Rules,
    c: &ClaimRow,
    liability: &str,
    cost: Option<&CostOverride>,
    now: chrono::DateTime<chrono::Utc>,
    (changes, checks, basis): Sinks<'_>,
) -> Result<(ApproveStep, i32), ToolError> {
    let order = sales::order(tx, &c.order_id)
        .await?
        .ok_or_else(|| broken(format!("claim {} has no order", c.id)))?;
    let view = cancel_view(tx, &order, Some(&c.id)).await?;
    let items = repo::claim_items(tx, std::slice::from_ref(&c.id)).await?;
    let asked: Vec<(String, i32)> = items
        .iter()
        .map(|i| (i.order_item_id.clone(), i.order_quantity))
        .collect();
    let outcome = cancel_outcome(
        rules,
        &view,
        CancelAsk {
            items: Some(&asked),
            reason: c.cancel_reason.as_deref(),
            liability: Some(liability),
            operator: true,
        },
        now,
    )
    .map_err(|b| match b {
        CancelBlock::AlreadyShipped(id) => ToolError::new(
            ALREADY_HANDED_OVER,
            "The item is already with the carrier; decline the cancel and offer a return.",
        )
        .detail(json!({"order_item_id": id, "rule": "cancel.operator_only_states"}))
        .next("reject_claim"),
        other => cancel_block_error(other),
    })?;
    let e = &outcome.estimate;
    let currency = e.refund_total.currency;
    let charges: rust_decimal::Decimal = e.extra_charge.iter().map(|x| x.amount.value).sum();
    let (refund_total, charge_due) = match cost {
        Some(o) => {
            let base = e.refund_total.value + charges;
            (
                Money::new(
                    (base - o.amount.value).max(rust_decimal::Decimal::ZERO),
                    currency,
                ),
                Money::new(
                    (o.amount.value - base).max(rust_decimal::Decimal::ZERO),
                    currency,
                ),
            )
        }
        None => (e.refund_total, e.charge_due),
    };
    if !charge_due.value.is_zero() {
        return Err(ToolError::new(
            CHARGE_UNPAID,
            format!(
                "The shipping charge is {} more than the refund; the shopper must pay it first.",
                money_text(charge_due)
            ),
        )
        .detail(json!({"amount": charge_due}))
        .next("get_claim"));
    }

    let lines = sales::order_items(tx, &c.order_id).await?;
    let stages: Vec<String> = asked
        .iter()
        .filter_map(|(id, _)| lines.iter().find(|l| &l.id == id))
        .map(|l| {
            l.fulfillment_stage
                .clone()
                .unwrap_or_else(|| l.order_item_status.clone())
        })
        .collect();
    checks.push(Check {
        rule: "cancel.operator_only_states".into(),
        passed: true,
        detail: Some(match stages.first().map(String::as_str) {
            Some("packed") => "Item is being packed and not yet with the carrier".to_owned(),
            Some("ship_instructed") => {
                "Item is being picked and not yet with the carrier".to_owned()
            }
            Some(other) => format!(
                "Item is {} and not yet with the carrier",
                other.replace('_', " ")
            ),
            None => "Items are not yet with the carrier".to_owned(),
        }),
        fix_tool: Some("reject_claim".into()),
    });
    checks.push(Check {
        rule: "cancel.shipping_recalculation".into(),
        passed: true,
        detail: Some(match (cost, e.extra_charge.first()) {
            (Some(o), _) => format!(
                "Shipping set to {} by the operator: {}",
                money_text(o.amount),
                o.note
            ),
            (None, Some(x)) => format!(
                "{}: {} shipping taken from the refund ({})",
                if outcome.liability == "customer_fault" {
                    "Customer fault"
                } else {
                    "Charge"
                },
                money_text(x.amount),
                x.why.to_lowercase()
            ),
            (None, None) => "No shipping charge".to_owned(),
        }),
        fix_tool: Some("approve_claim".into()),
    });
    let coupons = if e.coupons_restored.is_empty() {
        "No coupon on these items".to_owned()
    } else {
        format!(
            "{} coupon{} restored",
            e.coupons_restored.len(),
            if e.coupons_restored.len() == 1 {
                ""
            } else {
                "s"
            }
        )
    };
    checks.push(Check {
        rule: "cancel.restore".into(),
        passed: true,
        detail: Some(format!(
            "{coupons}; {} pending points cancelled",
            money_text(e.points_pending_cancelled)
        )),
        fix_tool: None,
    });

    let fx = cancel_effects(tx, &c.order_id, &view, &asked).await?;
    basis.extend(fx.basis);
    let full = outcome.estimate.scope == "full";
    changes.push(Change {
        op: ChangeOp::Update,
        entity: "claim".into(),
        id: c.id.clone(),
        from: json!(c.claim_status),
        to: json!("approved"),
        note: None,
    });
    changes.extend(fx.parcel_changes);
    changes.extend(fx.stock_changes);
    let payment = e
        .refund_breakdown
        .iter()
        .find(|p| !p.amount.value.is_zero())
        .or(e.refund_breakdown.first());
    let refund = (!refund_total.value.is_zero()).then(|| PlannedRefund {
        refund_id: RefundId::generate().to_string(),
        payment_id: payment.map(|p| p.payment_id.clone()),
        amount: refund_total,
        refund_method: rules.refund_to.clone(),
        refund_type: rules.defect_refund_type.clone(),
        to: payment.map(|p| p.to.clone()).unwrap_or_default(),
    });
    if let Some(r) = &refund {
        checks.push(Check {
            rule: "refund.to".into(),
            passed: !e.needs_refund_destination,
            detail: Some(if e.needs_refund_destination {
                "The original payment is too old to reverse; a person must pick where the money goes"
                    .to_owned()
            } else {
                format!("{} back to {}", money_text(r.amount), r.to)
            }),
            fix_tool: None,
        });
        changes.push(Change {
            op: ChangeOp::Create,
            entity: "refund".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"claim_id": c.id, "amount": r.amount, "to": r.to}),
            note: None,
        });
    }
    let units = asked.iter().map(|(_, q)| *q).sum();
    let step = ApproveStep {
        claim_id: c.id.clone(),
        claim_type: c.claim_type.clone(),
        order_id: c.order_id.clone(),
        from_status: c.claim_status.clone(),
        to_status: "approved".into(),
        liability_change: None,
        lines_cancelled: fx.lines_cancelled,
        restock: fx.restock,
        deliveries_cancelled: fx.deliveries_cancelled,
        coupons_restored: e
            .coupons_restored
            .iter()
            .map(|x| x.member_coupon_id.clone())
            .collect(),
        order_cancelled_from: full.then(|| order.order_status.clone()),
        refund,
    };
    Ok((step, units))
}
