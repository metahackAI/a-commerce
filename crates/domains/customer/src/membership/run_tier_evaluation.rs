//! `run_tier_evaluation`: evaluate a program now, outside its schedule: members move tiers, the
//! history records why, and members are told. An ad-hoc run always waits for a person
//! (`tier.evaluation.ad_hoc_runs`). Entry coupons belong to promotion and notices to messaging,
//! which customer cannot call: the run counts them and says they were not issued or sent.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::TierChangeCause;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    EvaluationScope, Placed, apply_moves, as_of, entry_benefits, evaluate_members, grouped,
    notices, program_not_found, revert_moves, running, scope_ids, tier_defs,
};
use crate::access::{always, db, policy, staff_only};
use crate::progress::{Direction, Move};
use crate::{AS_OF_IN_FUTURE, EVALUATION_IN_PROGRESS, PROGRAM_NOT_FOUND, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunTierEvaluationInput {
    /// The program (`list_tiers`).
    #[schemars(regex(pattern = r"^prg_[a-z0-9_]+$"), example = &"prg_rewards")]
    pub program_id: String,
    /// The moment to measure at, store time. Now when left out; not in the future.
    #[schemars(example = &"2026-11-06T11:00")]
    pub as_of: Option<String>,
    /// all (default), segment (`segment_id`) or customers (`customer_ids`).
    #[schemars(example = &"all")]
    pub scope: Option<EvaluationScope>,
    /// The segment, with `scope: segment`.
    #[schemars(regex(pattern = r"^seg_[a-z0-9_]+$"), example = &"seg_lapsed_gold")]
    pub segment_id: Option<String>,
    /// The members, with `scope: customers`.
    #[schemars(length(min = 1, max = 100), example = ["cus_alex"])]
    pub customer_ids: Option<Vec<String>>,
    /// Tell members who move. The store's `tier.notify.on` when left out.
    #[schemars(example = true)]
    pub notify: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunPlan {
    pub evaluation_id: String,
    pub program_id: String,
    pub as_of: DateTime<Utc>,
    pub scope: Value,
    /// Only the members who move or enter a grace period.
    pub moves: Vec<(Placed, Move)>,
    pub notices: usize,
    pub coupons: usize,
}

pub struct RunTierEvaluation;

impl Operation for RunTierEvaluation {
    const META: OpMeta = OpMeta {
        address: "customer.membership.run_tier_evaluation",
        tool: "run_tier_evaluation",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/customer/tier-programs/{program_id}/evaluations",
        summary: "Evaluate a tier program now, outside its schedule: members move up or down, enter grace, and are told. A person always approves; the tier moves can be undone.",
        consumes: &[Slot::one("program_id")],
        produces: &[Slot::one("evaluation_id")],
        checks: &[
            "tier.evaluation.settle_days",
            "tier.override.evaluation_respects_override",
            "tier.evaluation.one_run_at_a_time",
            "tier.notify.message_kind",
        ],
        errors: &[EVALUATION_IN_PROGRESS, PROGRAM_NOT_FOUND, AS_OF_IN_FUTURE],
        audience: Audience::Operator,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = RunTierEvaluationInput;
    type Output = Value;
}

fn in_progress(id: &str) -> ToolError {
    ToolError::new(
        EVALUATION_IN_PROGRESS,
        format!("Evaluation {id} of this program is still running."),
    )
    .detail(json!({"entity": "tier_evaluation", "id": id, "from": "active"}))
    .next("list_tier_history")
}

impl WriteOp<Cx> for RunTierEvaluation {
    type Plan = RunPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RunTierEvaluationInput,
    ) -> Result<Planned<RunPlan>, ToolError> {
        staff_only(cx)?;
        let at = as_of(cx, input.as_of.as_deref(), "run_tier_evaluation")?;
        let scope = input.scope.unwrap_or(EvaluationScope::All);
        let ids = scope_ids(
            scope,
            input.segment_id.as_deref(),
            input.customer_ids.as_deref(),
            "run_tier_evaluation",
        )?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let program = repo::program(&mut tx, &input.program_id)
            .await?
            .ok_or_else(|| program_not_found(&input.program_id))?;
        if let Some(r) = running(&mut tx, &program.id).await? {
            return Err(in_progress(&r));
        }
        let tiers = tier_defs(&repo::tiers(&mut tx, Some(&program.id)).await?);
        let outcome = evaluate_members(
            &mut tx,
            &program,
            &tiers,
            &program.evaluation,
            &program.movement,
            ids.as_deref(),
            at,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let overrides = outcome
            .moves
            .iter()
            .filter(|(_, m)| m.direction == Direction::Override)
            .count();
        let moves: Vec<(Placed, Move)> = outcome
            .moves
            .into_iter()
            .filter(|(_, m)| {
                matches!(
                    m.direction,
                    Direction::Up | Direction::Down | Direction::Grace
                )
            })
            .collect();
        let notify = input.notify.unwrap_or(true);
        let notice_count = if notify {
            notices(&cx.policy, &moves)
        } else {
            0
        };
        let coupons: usize = moves
            .iter()
            .filter(|(_, m)| m.direction == Direction::Up)
            .map(|(_, m)| entry_benefits(&tiers, &m.to_tier_id).len())
            .sum();
        let mut changes: Vec<Change> = grouped(&moves)
            .into_iter()
            .map(|(f, t, d, n)| Change {
                op: ChangeOp::Update,
                entity: "member_tier".into(),
                id: String::new(),
                from: json!(f),
                to: json!(t),
                note: Some(match d {
                    Direction::Grace => format!("{n} enter a grace period and keep {f} for now"),
                    _ => format!("{n} members"),
                }),
            })
            .collect();
        if coupons > 0 {
            changes.push(Change {
                op: ChangeOp::Grant,
                entity: "member_coupon".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"count": coupons}),
                note: Some("Entry coupons are promotion's; recorded, not issued from here".into()),
            });
        }
        if notice_count > 0 {
            changes.push(Change {
                op: ChangeOp::Send,
                entity: "message".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"count": notice_count}),
                note: Some(
                    "Tier notices; delivery is not connected, so they are recorded, not sent"
                        .into(),
                ),
            });
        }
        let kind = policy(cx, "customer.tier.notify.message_kind")
            .and_then(Value::as_str)
            .unwrap_or("transactional");
        let up = moves
            .iter()
            .filter(|(_, m)| m.direction == Direction::Up)
            .count();
        let down = moves
            .iter()
            .filter(|(_, m)| m.direction == Direction::Down)
            .count();
        let checks = vec![
            Check {
                rule: "tier.evaluation.settle_days".into(),
                passed: true,
                detail: Some(if outcome.measured {
                    format!("Orders count {} days after delivery", outcome.settle_days)
                } else {
                    "Orders are not connected here: nothing measured, nobody moves".to_owned()
                }),
                fix_tool: None,
            },
            Check {
                rule: "tier.override.evaluation_respects_override".into(),
                passed: true,
                detail: Some(format!("{overrides} manual tiers left as they are")),
                fix_tool: None,
            },
            Check {
                rule: "tier.evaluation.one_run_at_a_time".into(),
                passed: true,
                detail: Some("No other evaluation is running".into()),
                fix_tool: None,
            },
            Check {
                rule: "tier.notify.message_kind".into(),
                passed: true,
                detail: Some(format!(
                    "Tier notices are {kind}; promotional wording would need marketing consent"
                )),
                fix_tool: Some("create_message_template".into()),
            },
        ];
        let facts = json!({"program_id": program.id});
        Ok(Planned {
            changes,
            checks,
            basis: vec![Basis {
                entity: "customer.member_program".into(),
                id: program.id.clone(),
                version: i64::from(program.version),
            }],
            approval: always(
                cx,
                Self::META.tool,
                &facts,
                format!("Moves {up} members up and {down} down, sends {notice_count} notices"),
            ),
            plan: RunPlan {
                evaluation_id: format!("tev_{}", acom_core::id::new_suffix()),
                program_id: program.id,
                as_of: at,
                scope: json!({"scope": scope.word(), "segment_id": input.segment_id, "customer_ids": ids}),
                moves,
                notices: notice_count,
                coupons,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &RunPlan) -> Result<Committed, ToolError> {
        let mut produced = Map::new();
        produced.insert("evaluation_id".into(), json!(plan.evaluation_id));
        let done: Option<String> = sqlx::query_scalar(
            "SELECT action_status FROM customer.tier_evaluation WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.evaluation_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        if done.is_some() {
            // A repeated run returns the first result.
            return Ok(Committed {
                id: plan.evaluation_id.clone(),
                produced,
            });
        }
        if let Some(r) = running(tx, &plan.program_id).await? {
            return Err(in_progress(&r));
        }
        let now = tx.clock.now();
        sqlx::query(
            "INSERT INTO customer.tier_evaluation (tenant_id, id, member_program_id, as_of, scope,
               action_status, agent_id, start_time)
             VALUES ($1, $2, $3, $4, $5, 'active', $6, $7)",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.evaluation_id)
        .bind(&plan.program_id)
        .bind(plan.as_of)
        .bind(&plan.scope)
        .bind(&tx.principal.subject)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        apply_moves(
            tx,
            &plan.program_id,
            Some(&plan.evaluation_id),
            &plan.moves,
            TierChangeCause::Evaluation,
        )
        .await?;
        let count = |d: Direction| plan.moves.iter().filter(|(_, m)| m.direction == d).count();
        let summary = json!({"up": count(Direction::Up), "down": count(Direction::Down),
                             "grace": count(Direction::Grace), "coupons": plan.coupons,
                             "notices": plan.notices});
        sqlx::query(
            "UPDATE customer.tier_evaluation SET action_status = 'completed', moves = $3,
               summary = $4, end_time = $5, updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.evaluation_id)
        .bind(json!(
            plan.moves
                .iter()
                .map(|(p, m)| json!({"customer_id": p.customer_id,
            "from_tier_id": m.from_tier_id, "to_tier_id": m.to_tier_id,
            "direction": m.direction.word(), "reason": m.reason,
            "from_since": p.tier_since}))
                .collect::<Vec<_>>()
        ))
        .bind(&summary)
        .bind(tx.clock.now())
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        produced.insert("summary".into(), summary);
        produced.insert(
            "notices".into(),
            json!({"count": plan.notices, "delivered": false,
                   "note": "Messaging is not connected to customer; the notices were counted, not sent."}),
        );
        produced.insert(
            "coupons".into(),
            json!({"count": plan.coupons, "issued": false, "next_tool": "issue_coupon",
                   "note": "Entry coupons are issued by promotion, which customer cannot call."}),
        );
        Ok(Committed {
            id: plan.evaluation_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RunPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        for (p, m) in plan
            .moves
            .iter()
            .filter(|(_, m)| matches!(m.direction, Direction::Up | Direction::Down))
        {
            let now = repo::memberships(&mut tx, &p.customer_id).await?;
            if !now
                .iter()
                .any(|x| x.member_program_id == plan.program_id && x.tier_id == m.to_tier_id)
            {
                mismatches.push(format!("{} is not in {}", p.customer_id, m.to_tier_id));
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
        plan: &RunPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        revert_moves(
            tx,
            &plan.program_id,
            Some(&plan.evaluation_id),
            &plan.moves,
            TierChangeCause::Evaluation,
        )
        .await?;
        sqlx::query(
            "UPDATE customer.tier_evaluation SET action_status = 'failed', updated_at = now(),
               version = version + 1 WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.evaluation_id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_always_waits_for_a_person() {
        let s = closed_schema_for::<RunTierEvaluationInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(RunTierEvaluation::META.kind, Kind::WriteApproval);
        assert_eq!(
            s["properties"]["scope"]["enum"],
            json!(["all", "segment", "customers"])
        );
    }
}
