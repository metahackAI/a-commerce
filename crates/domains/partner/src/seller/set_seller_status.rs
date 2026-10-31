//! `set_seller_status`: approves, activates, suspends or terminates a seller. It reaches what
//! shoppers see, what sells and what is paid out, so a person always approves it (W$). Only the
//! transitions of `lifecycle.allowed_transitions` run. Suspending takes the seller's products off
//! sale (`suspend.hide_products`) while open orders still ship; activating puts them back.
//! Terminating also takes them off sale, waits until no order is open and nothing is left to pay
//! out, and is final. A change with a later `valid_from` is booked and run by a job then.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::Money;
use acom_core::enums::SellerStatus;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::rules::Rules;
use crate::{
    NOT_REVOCABLE, OPEN_ORDERS_EXIST, PENDING_SETTLEMENT, SELLER_NOT_FOUND, activity, contract,
    jobs, parse_time, repo, seller_not_found, sentence, store, viewer,
};

/// The status a seller moves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetStatus {
    /// Review passed: contract and listings can be set up; nothing sells yet.
    Approved,
    /// Selling: its products are on sale.
    Active,
    /// Stopped for now: its products are hidden; open orders still ship.
    Suspended,
    /// Ended for good; cannot be undone.
    Terminated,
}

impl TargetStatus {
    pub fn status(self) -> SellerStatus {
        match self {
            TargetStatus::Approved => SellerStatus::Approved,
            TargetStatus::Active => SellerStatus::Active,
            TargetStatus::Suspended => SellerStatus::Suspended,
            TargetStatus::Terminated => SellerStatus::Terminated,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetSellerStatusInput {
    /// The seller, from `search_sellers`.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_harbor_goods")]
    pub seller_id: String,
    /// approved: review passed; active: selling; suspended: products hidden, open orders still
    /// ship; terminated: ended for good.
    #[serde(rename = "acom:target_status", alias = "target_status")]
    #[schemars(example = &"suspended")]
    pub target_status: TargetStatus,
    /// Why, in a sentence; it goes to the seller's status history.
    #[schemars(length(min = 1, max = 500), example = &"Three late shipments in a week")]
    pub reason: String,
    /// When the change takes effect (store time); now when left out.
    #[schemars(example = &"2026-11-09T00:00")]
    pub valid_from: Option<String>,
}

/// The transition as previewed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusPlan {
    pub seller_id: String,
    pub from: String,
    pub to: String,
    pub reason: String,
    /// When the change takes effect, when that is after the run; the change is booked.
    pub scheduled_at: Option<DateTime<Utc>>,
    /// Already in the target status: the run changes nothing.
    pub unchanged: bool,
}

pub struct SetSellerStatus;

impl Operation for SetSellerStatus {
    const META: OpMeta = OpMeta {
        address: "partner.seller.set_seller_status",
        tool: "set_seller_status",
        group: Group::Operate,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/partner/sellers/{seller_id}/status",
        summary: "Approve, activate, suspend or terminate a seller, now or from a later time. Suspending hides its products while open orders still ship; the preview shows the products, open orders and unpaid amount affected. A person always approves.",
        consumes: &[Slot::one("seller_id")],
        produces: &[Slot::one("seller_id")],
        checks: &["lifecycle.allowed_transitions", "suspend.hide_products"],
        errors: &[
            SELLER_NOT_FOUND,
            ErrorCode::INVALID_STATE,
            OPEN_ORDERS_EXIST,
            PENDING_SETTLEMENT,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetSellerStatusInput;
    type Output = Value;
}

/// Whether moving to `to` takes the seller's products off sale.
fn hides(to: &str, rules: &Rules) -> bool {
    to == SellerStatus::Terminated.as_str()
        || (to == SellerStatus::Suspended.as_str() && rules.hide_products)
}

/// What a transition did to the seller's products.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct Applied {
    pub products_hidden: Vec<String>,
    pub products_restored: Vec<String>,
}

/// Moves a seller from `from` to `to` and takes its products off sale or puts them back as the
/// new status wants. Used by the run, its undo, and the job that runs booked changes.
pub(crate) async fn apply_transition(
    tx: &mut TxCx,
    seller_id: &str,
    from: &str,
    to: &str,
) -> Result<Applied, ToolError> {
    if !repo::set_status(tx, seller_id, from, to).await? {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("{seller_id} is no longer {}.", from.replace('_', " ")),
        )
        .detail(json!({"entity": "seller", "from": from}))
        .next("get_seller"));
    }
    let rules = Rules::load(&tx.policy)?;
    let products = repo::products(tx, seller_id).await?;
    let now = tx.clock.now();
    let mut applied = Applied::default();
    if hides(to, &rules) {
        let listed: Vec<String> = products
            .iter()
            .filter(|(_, hidden)| hidden.is_none())
            .map(|(id, _)| id.clone())
            .collect();
        applied.products_hidden =
            acom_catalog::service::for_partner::take_off_sale(tx, &listed).await?;
        repo::mark_hidden(tx, &applied.products_hidden, Some(now)).await?;
    } else if to == SellerStatus::Active.as_str() {
        let hidden: Vec<String> = products
            .iter()
            .filter(|(_, hidden)| hidden.is_some())
            .map(|(id, _)| id.clone())
            .collect();
        applied.products_restored =
            acom_catalog::service::for_partner::put_back_on_sale(tx, &hidden).await?;
        repo::mark_hidden(tx, &hidden, None).await?;
    }
    Ok(applied)
}

fn transition_error(from: &str, to: &str, rules: &Rules) -> ToolError {
    let allowed = rules
        .allowed_transitions
        .get(from)
        .cloned()
        .unwrap_or_default();
    ToolError::new(
        ErrorCode::INVALID_STATE,
        format!(
            "A seller that is {} cannot become {}{}.",
            from.replace('_', " "),
            to.replace('_', " "),
            if allowed.is_empty() {
                String::new()
            } else {
                format!(
                    " (it can become {})",
                    allowed.join(" or ").replace('_', " ")
                )
            }
        ),
    )
    .detail(
        json!({"entity": "seller", "from": from, "allowed_to": allowed,
                   "rule": "lifecycle.allowed_transitions"}),
    )
    .next("get_seller")
}

fn money_text(m: &Money) -> String {
    format!("{} {}", m.value, m.currency)
}

impl WriteOp<Cx> for SetSellerStatus {
    type Plan = StatusPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetSellerStatusInput,
    ) -> Result<Planned<StatusPlan>, ToolError> {
        viewer::staff(cx)?;
        let reason = sentence("reason", &input.reason, "why the status changes")?;
        let now = cx.clock.now();
        let at = input
            .valid_from
            .as_deref()
            .map(|s| parse_time(cx, "valid_from", s))
            .transpose()?;
        let scheduled_at = at.filter(|t| *t > now);
        let rules = Rules::load(&cx.policy)?;
        let to = input.target_status.status().as_str();

        let mut tx = cx.read().await.map_err(store)?;
        viewer::viewer(cx, &mut tx)
            .await?
            .store_only("change a seller's status")?;
        let s = repo::seller(&mut tx, &input.seller_id)
            .await?
            .ok_or_else(|| seller_not_found(&input.seller_id))?;
        let from = s.seller_status.clone();
        let unchanged = from == to;
        if !unchanged && !rules.allows(&from, to) {
            return Err(transition_error(&from, to, &rules));
        }
        let products = activity::products(&mut tx, &s.id).await?;
        let hidden_now: Vec<String> = repo::products(&mut tx, &s.id)
            .await?
            .into_iter()
            .filter(|(_, h)| h.is_some())
            .map(|(id, _)| id)
            .collect();
        let open_orders = activity::open_orders(&mut tx, &products).await?;
        let unpaid = match contract::terms_now(&mut tx, &s.id, now).await? {
            Some(terms) => Some(activity::settlement(&mut tx, &rules, &products, &terms).await?),
            None => None,
        };
        tx.commit().await.map_err(store)?;
        let unpaid_amount = unpaid
            .as_ref()
            .map_or_else(|| Money::zero(cx.currency), |u| u.pending);

        if !unchanged && to == SellerStatus::Terminated.as_str() {
            if open_orders > 0 {
                return Err(ToolError::new(
                    OPEN_ORDERS_EXIST,
                    format!(
                        "{} has {open_orders} open order(s); they must ship or be cancelled before the seller is terminated (suspend it to stop new sales now).",
                        s.legal_name
                    ),
                )
                .detail(json!({"open_orders": open_orders}))
                .next("search_orders"));
            }
            if unpaid_amount.value > rust_decimal::Decimal::ZERO {
                return Err(ToolError::new(
                    PENDING_SETTLEMENT,
                    format!(
                        "{} is still owed about {} (payouts are made outside A-Commerce); terminate after it is paid out.",
                        s.legal_name,
                        money_text(&unpaid_amount)
                    ),
                )
                .detail(json!({"pending_amount": unpaid_amount,
                               "next_payout_date": unpaid.map(|u| u.next_payout_date.to_string())}))
                .next("get_seller_overview"));
            }
        }

        let will_hide: Vec<&str> = if hides(to, &rules) {
            products
                .iter()
                .filter(|p| p.approval_status == "active" && !hidden_now.contains(&p.product_id))
                .map(|p| p.product_id.as_str())
                .collect()
        } else {
            Vec::new()
        };
        let will_restore: Vec<&str> = if to == SellerStatus::Active.as_str() {
            hidden_now.iter().map(String::as_str).collect()
        } else {
            Vec::new()
        };

        let mut checks = vec![Check {
            rule: "lifecycle.allowed_transitions".into(),
            passed: true,
            detail: Some(if unchanged {
                format!("Already {}; nothing changes", to.replace('_', " "))
            } else {
                format!(
                    "{} → {} is allowed",
                    from.replace('_', " "),
                    to.replace('_', " ")
                )
            }),
            fix_tool: None,
        }];
        if to == SellerStatus::Suspended.as_str() || to == SellerStatus::Terminated.as_str() {
            checks.push(Check {
                rule: "suspend.hide_products".into(),
                passed: true,
                detail: Some(if will_hide.is_empty() && hides(to, &rules) {
                    "No product of the seller is on sale".into()
                } else if hides(to, &rules) {
                    format!(
                        "{} product(s) leave the store; {open_orders} open order(s) still ship",
                        will_hide.len()
                    )
                } else {
                    format!(
                        "The store keeps a suspended seller's products on sale; {open_orders} open order(s) still ship"
                    )
                }),
                fix_tool: None,
            });
        }

        let mut changes = vec![Change {
            op: if scheduled_at.is_some() {
                ChangeOp::Schedule
            } else {
                ChangeOp::Update
            },
            entity: "seller".into(),
            id: s.id.clone(),
            from: json!({"acom:seller_status": from}),
            to: json!({
                "acom:seller_status": to,
                "valid_from": scheduled_at.map(|t| cx.time.format(t)),
                "impact": {
                    "products_hidden": will_hide.len(),
                    "products_restored": will_restore.len(),
                    "open_orders": open_orders,
                    "pending_settlement": unpaid_amount,
                },
            }),
            note: Some(if to == SellerStatus::Terminated.as_str() {
                "Final: a terminated seller cannot come back, and this change cannot be undone."
                    .into()
            } else {
                format!("Reason: {reason}")
            }),
        }];
        for p in &will_hide {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "product".into(),
                id: (*p).to_owned(),
                from: json!({"approval_status": "active"}),
                to: json!({"approval_status": "draft"}),
                note: Some("Taken off sale with the seller".into()),
            });
        }
        for p in &will_restore {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "product".into(),
                id: (*p).to_owned(),
                from: json!({"approval_status": "draft"}),
                to: json!({"approval_status": "active"}),
                note: Some("Back on sale with the seller".into()),
            });
        }

        let reason_text = match input.target_status {
            _ if unchanged => format!("Keeps {} {}", s.legal_name, to.replace('_', " ")),
            TargetStatus::Approved => format!("Approves {} as a seller", s.legal_name),
            TargetStatus::Active => format!("Activates {}", s.legal_name),
            TargetStatus::Suspended => format!(
                "Suspends a seller with {open_orders} open order{}",
                if open_orders == 1 { "" } else { "s" }
            ),
            TargetStatus::Terminated => format!("Terminates {} for good", s.legal_name),
        };
        Ok(Planned {
            changes,
            checks,
            basis: vec![Basis {
                entity: "partner.seller".into(),
                id: s.id.clone(),
                version: i64::from(s.version),
            }],
            approval: ApprovalNeed::Needed {
                reason: reason_text,
            },
            plan: StatusPlan {
                seller_id: s.id,
                from,
                to: to.to_owned(),
                reason,
                scheduled_at,
                unchanged,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StatusPlan) -> Result<Committed, ToolError> {
        let done = Committed {
            id: plan.seller_id.clone(),
            produced: Map::from_iter([("seller_id".to_owned(), json!(plan.seller_id))]),
        };
        if plan.unchanged {
            return Ok(done);
        }
        let now = tx.clock.now();
        match plan.scheduled_at {
            Some(at) => {
                let position = repo::insert_event(
                    tx,
                    &repo::NewEvent {
                        seller_id: &plan.seller_id,
                        action_type: ActionType::UPDATE.0,
                        action_status: "potential",
                        instrument: Self::META.tool,
                        from_state: Some(&plan.from),
                        to_state: &plan.to,
                        result: json!({"reason": plan.reason}),
                        start_time: at,
                        end_time: None,
                    },
                )
                .await?;
                jobs::book(tx, &plan.seller_id, position, at).await?;
            }
            None => {
                let applied = apply_transition(tx, &plan.seller_id, &plan.from, &plan.to).await?;
                repo::insert_event(
                    tx,
                    &repo::NewEvent {
                        seller_id: &plan.seller_id,
                        action_type: ActionType::UPDATE.0,
                        action_status: "completed",
                        instrument: Self::META.tool,
                        from_state: Some(&plan.from),
                        to_state: &plan.to,
                        result: json!({"reason": plan.reason,
                                       "products_hidden": applied.products_hidden,
                                       "products_restored": applied.products_restored}),
                        start_time: now,
                        end_time: Some(now),
                    },
                )
                .await?;
            }
        }
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &StatusPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(store)?;
        let s = repo::seller(&mut tx, &plan.seller_id).await?;
        let waiting = repo::waiting_events(&mut tx, &plan.seller_id).await?;
        tx.commit().await.map_err(store)?;
        let mismatches = match (s, plan.scheduled_at) {
            (None, _) => vec![format!("seller {} is missing", plan.seller_id)],
            (Some(_), Some(at)) if !plan.unchanged => {
                if waiting
                    .iter()
                    .any(|e| e.start_time == at && e.to_state.as_deref() == Some(plan.to.as_str()))
                {
                    Vec::new()
                } else {
                    vec![format!("no change to {} is booked", plan.to)]
                }
            }
            (Some(s), _) if s.seller_status != plan.to => {
                vec![format!("seller {} is {}", s.id, s.seller_status)]
            }
            _ => Vec::new(),
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, tx: &mut TxCx, plan: &StatusPlan, _: &Committed) -> Result<(), ToolError> {
        if plan.unchanged {
            return Ok(());
        }
        if plan.to == SellerStatus::Terminated.as_str() {
            return Err(ToolError::new(
                NOT_REVOCABLE,
                format!("{} was terminated; a termination is final.", plan.seller_id),
            ));
        }
        let now = tx.clock.now();
        if let Some(at) = plan.scheduled_at {
            let waiting = repo::waiting_events(tx, &plan.seller_id).await?;
            if let Some(e) = waiting
                .iter()
                .find(|e| e.start_time == at && e.to_state.as_deref() == Some(plan.to.as_str()))
            {
                repo::finish_event(
                    tx,
                    &plan.seller_id,
                    e.position,
                    "failed",
                    None,
                    Some(&json!({"code": "undone", "class": "invalid_state",
                                  "message": "The booked change was undone before it ran."})),
                    now,
                )
                .await?;
                return Ok(());
            }
        }
        let applied = apply_transition(tx, &plan.seller_id, &plan.to, &plan.from).await?;
        repo::insert_event(
            tx,
            &repo::NewEvent {
                seller_id: &plan.seller_id,
                action_type: ActionType::UPDATE.0,
                action_status: "completed",
                instrument: "undo",
                from_state: Some(&plan.to),
                to_state: &plan.from,
                result: json!({"reason": "Undone", "products_hidden": applied.products_hidden,
                               "products_restored": applied.products_restored}),
                start_time: now,
                end_time: Some(now),
            },
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::rules;
    use crate::tests::{input, offline};

    #[tokio::test]
    async fn a_status_change_needs_a_reason() {
        let e = SetSellerStatus
            .preview(
                &offline("adm_sj"),
                input(
                    json!({"seller_id": "slr_fieldstone", "acom:target_status": "suspended",
                             "reason": " "}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[test]
    fn the_target_status_takes_only_the_four_moves() {
        let r: Result<SetSellerStatusInput, _> = serde_json::from_value(
            json!({"seller_id": "slr_fieldstone", "acom:target_status": "under_review",
                   "reason": "x"}),
        );
        assert!(r.is_err());
    }

    #[test]
    fn refused_moves_say_where_the_seller_can_go() {
        let e = transition_error("terminated", "active", &rules());
        assert_eq!(e.code, "invalid_state");
        assert_eq!(e.next_tool.as_deref(), Some("get_seller"));
        let e = transition_error("approved", "suspended", &rules());
        assert!(e.message.contains("active or terminated"), "{}", e.message);
    }

    #[test]
    fn suspension_hides_products_only_when_the_policy_says_so() {
        let mut r = rules();
        assert!(hides("suspended", &r) && hides("terminated", &r));
        r.hide_products = false;
        assert!(!hides("suspended", &r) && hides("terminated", &r));
        assert!(!hides("active", &r));
    }
}
