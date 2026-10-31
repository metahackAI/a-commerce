//! `reject_claim`: the operator declines a claim with a reason the policy backs.

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{claim_not_found, invalid_state, visible_claim};
use crate::errors::{CLAIM_NOT_FOUND, REASON_REQUIRED};
use crate::model::date_text;
use crate::model::delivery::{days_since, local_date};
use crate::repo::{self, ClaimEvent};
use crate::{delivered_at, rules};

/// Why a claim is declined (docs/api/enums.md §2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeclineReason {
    /// Delivered longer ago than the return window.
    OutsideReturnWindow,
    /// A final-sale item or a gift card.
    NotReturnable,
    /// Inspection found damage or missing parts the shopper caused.
    InspectionFailed,
    /// A cancel came after the parcel reached the carrier: offer a return instead.
    AlreadyHandedOver,
    /// Delivered longer ago than the exchange window.
    OutsideExchangeWindow,
}

impl DeclineReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::OutsideReturnWindow => "outside_return_window",
            Self::NotReturnable => "not_returnable",
            Self::InspectionFailed => "inspection_failed",
            Self::AlreadyHandedOver => "already_handed_over",
            Self::OutsideExchangeWindow => "outside_exchange_window",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RejectClaimInput {
    /// The claim to decline.
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_20290")]
    pub claim_id: String,
    /// Why; required, and checked against the policy.
    #[schemars(example = &"outside_return_window")]
    pub reason: Option<DeclineReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectPlan {
    pub claim_id: String,
    pub from_status: String,
    pub reason: String,
    /// `(variant_id, units)` an exchange was holding, released on decline.
    pub release: Vec<(String, i32)>,
}

pub struct RejectClaim;

impl Operation for RejectClaim {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.reject",
        tool: "reject_claim",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/fulfillment/claims/{claim_id}/reject",
        summary: "Decline a claim with a reason the policy backs (outside the return window, not returnable, failed inspection, already with the carrier). Tell the shopper separately with send_customer_message(kind=decline).",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[
            "return_window_days",
            "return.non_returnable_when",
            "exchange.window",
            "return.inventory_restore_when",
            "cancel.operator_only_states",
        ],
        errors: &[CLAIM_NOT_FOUND, ErrorCode::INVALID_STATE, REASON_REQUIRED],
        audience: Audience::Operator,
        action_type: ActionType::REJECT,
        person_scope: "",
    };
    type Input = RejectClaimInput;
    type Output = Value;
}

impl WriteOp<Cx> for RejectClaim {
    type Plan = RejectPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RejectClaimInput,
    ) -> Result<Planned<RejectPlan>, ToolError> {
        let reason = input.reason.ok_or_else(|| {
            ToolError::new(REASON_REQUIRED, "Say why the claim is declined.")
                .detail(json!({"field": "reason", "reason": "missing"}))
                .next("reject_claim")
        })?;
        let rules = rules(&cx.policy)?;
        let now = cx.clock.now();
        let zone = cx.time.zone;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = visible_claim(&mut tx, &input.claim_id, "search_claims").await?;
        if c.claim_status == "declined" {
            tx.commit().await.map_err(ToolError::from)?;
            return Ok(Planned {
                plan: RejectPlan {
                    claim_id: c.id,
                    from_status: "declined".into(),
                    reason: reason.as_str().into(),
                    release: Vec::new(),
                },
                changes: Vec::new(),
                checks: Vec::new(),
                basis: Vec::new(),
                approval: ApprovalNeed::NotNeeded,
            });
        }
        if !matches!(
            c.claim_status.as_str(),
            "requested" | "photos_requested" | "inspected"
        ) {
            return Err(invalid_state(&c, &["declined"]));
        }
        let items = repo::claim_items(&mut tx, std::slice::from_ref(&c.id)).await?;
        let deliveries =
            repo::deliveries_of_orders(&mut tx, std::slice::from_ref(&c.order_id)).await?;
        let delivered = delivered_at(&deliveries, &c.order_id);
        let check = match reason {
            DeclineReason::OutsideReturnWindow | DeclineReason::OutsideExchangeWindow => {
                let (rule, limit) = if reason == DeclineReason::OutsideReturnWindow {
                    ("return_window_days", rules.return_window_days)
                } else {
                    (
                        "exchange.window",
                        rules
                            .lookup("exchange.window.self_service_days")
                            .and_then(Value::as_i64)
                            .unwrap_or(rules.return_window_days),
                    )
                };
                match delivered {
                    Some(d) => {
                        let days = days_since(d, now, zone);
                        let when = date_text(local_date(d, zone));
                        Check {
                            rule: rule.into(),
                            passed: days > limit,
                            detail: Some(if days > limit {
                                format!("Delivered {when}, {days} days ago (limit {limit})")
                            } else {
                                format!(
                                    "Delivered {when}, {days} days ago, inside the {limit}-day window"
                                )
                            }),
                            fix_tool: (days <= limit).then(|| "evaluate_return_request".into()),
                        }
                    }
                    None => Check {
                        rule: rule.into(),
                        passed: false,
                        detail: Some("Not delivered yet, so no window has passed".into()),
                        fix_tool: Some("evaluate_return_request".into()),
                    },
                }
            }
            DeclineReason::NotReturnable => {
                let lines = sales::items(
                    &mut tx,
                    &items
                        .iter()
                        .map(|i| i.order_item_id.clone())
                        .collect::<Vec<_>>(),
                )
                .await?;
                let mut why = None;
                for l in &lines {
                    if let Some(v) = catalog::variant(&mut tx, &l.variant_id).await? {
                        if v.sale_type == "gift_card" {
                            why = Some(format!("{} is a gift card", l.name));
                        } else if v.keywords.iter().any(|k| k == "final_sale") {
                            why = Some(format!("{} was a final-sale item", l.name));
                        }
                    }
                }
                Check {
                    rule: "return.non_returnable_when".into(),
                    passed: why.is_some(),
                    detail: Some(
                        why.unwrap_or_else(|| "Every item on the claim can be returned".into()),
                    ),
                    fix_tool: None,
                }
            }
            DeclineReason::InspectionFailed => {
                let i = repo::inspection(&mut tx, &c.id).await?;
                let failed = i
                    .as_ref()
                    .is_some_and(|i| i.inspection_condition != "as_new");
                Check {
                    rule: "return.inventory_restore_when".into(),
                    passed: failed,
                    detail: Some(match &i {
                        Some(i) => {
                            format!("Inspected as {}", i.inspection_condition.replace('_', " "))
                        }
                        None => "Not inspected yet".to_owned(),
                    }),
                    fix_tool: (!failed).then(|| "assess_return_condition".into()),
                }
            }
            DeclineReason::AlreadyHandedOver => {
                let lines = sales::items(
                    &mut tx,
                    &items
                        .iter()
                        .map(|i| i.order_item_id.clone())
                        .collect::<Vec<_>>(),
                )
                .await?;
                let gone = c.claim_type == "cancel"
                    && lines.iter().any(|l| {
                        l.fulfillment_stage.as_deref() == Some("shipped")
                            || matches!(l.order_item_status.as_str(), "in_transit" | "delivered")
                    });
                Check {
                    rule: "cancel.operator_only_states".into(),
                    passed: gone,
                    detail: Some(if gone {
                        "The parcel is already with the carrier; the shopper can return it".into()
                    } else {
                        "The items have not reached the carrier yet".into()
                    }),
                    fix_tool: (!gone).then(|| "approve_claim".into()),
                }
            }
        };
        tx.commit().await.map_err(ToolError::from)?;
        let release = if c.claim_type == "exchange" {
            items
                .iter()
                .filter_map(|i| {
                    i.replacement_product_id
                        .clone()
                        .map(|v| (v, i.order_quantity))
                })
                .collect()
        } else {
            Vec::new()
        };
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: RejectPlan {
                claim_id: c.id.clone(),
                from_status: c.claim_status.clone(),
                reason: reason.as_str().into(),
                release,
            },
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "claim".into(),
                id: c.id.clone(),
                from: json!(c.claim_status),
                to: json!("declined"),
                note: None,
            }],
            checks: vec![check],
            basis: vec![Basis {
                entity: "fulfillment.claim".into(),
                id: c.id,
                version: i64::from(c.version),
            }],
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval.reason.unwrap_or_else(|| "Declines a claim".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &RejectPlan) -> Result<Committed, ToolError> {
        let c = repo::claim_for_update(tx, &plan.claim_id)
            .await?
            .ok_or_else(|| claim_not_found(&plan.claim_id, "search_claims"))?;
        let done = Committed {
            id: plan.claim_id.clone(),
            produced: Map::from_iter([("claim_id".to_owned(), json!(plan.claim_id))]),
        };
        if c.claim_status == "declined" {
            return Ok(done);
        }
        if c.claim_status != plan.from_status {
            return Err(invalid_state(&c, &["declined"]));
        }
        repo::move_claim(tx, &plan.claim_id, &plan.from_status, "declined").await?;
        for (variant, units) in &plan.release {
            catalog::move_stock(tx, variant, *units).await?;
        }
        repo::append_event(
            tx,
            &plan.claim_id,
            &ClaimEvent {
                action_type: "RejectAction",
                action_status: "completed",
                instrument: Self::META.tool,
                from_state: Some(&plan.from_status),
                to_state: Some("declined"),
                result: Some(json!({"reason": plan.reason})),
            },
        )
        .await?;
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RejectPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::claim(&mut tx, &plan.claim_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match c {
            Some(c) if c.claim_status == "declined" => Vec::new(),
            Some(c) => vec![format!("claim {} is {}", c.id, c.claim_status)],
            None => vec![format!("claim {} is missing", plan.claim_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, tx: &mut TxCx, plan: &RejectPlan, _: &Committed) -> Result<(), ToolError> {
        if plan.from_status == "declined" {
            return Ok(());
        }
        if !repo::move_claim(tx, &plan.claim_id, "declined", &plan.from_status).await? {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("Claim {} is no longer declined.", plan.claim_id),
            )
            .next("get_claim"));
        }
        for (variant, units) in &plan.release {
            catalog::move_stock(tx, variant, -units).await?;
        }
        repo::append_event(
            tx,
            &plan.claim_id,
            &ClaimEvent {
                action_type: "UpdateAction",
                action_status: "completed",
                instrument: "undo_change_set",
                from_state: Some("declined"),
                to_state: Some(&plan.from_status),
                result: None,
            },
        )
        .await?;
        Ok(())
    }
}
