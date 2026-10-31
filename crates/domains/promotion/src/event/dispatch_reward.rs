//! `dispatch_reward` (docs/api/promotion.md#event): sends winners their rewards the way the
//! reward type says (`event.reward_types`): `grant` adds store points to the ledger, `issue`
//! gives the coupon, `ship` and `email` are requests to systems outside this one (stubs that
//! say so in `fulfillment_ref`), `manual` records it for a person. A winner a store compliance
//! rule waits on is held as `compliance_pending` (`event.compliance.on_unsatisfied`). Sent
//! rewards are not taken back.

use std::collections::BTreeMap;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::Money;
use acom_core::enums::LoyaltyTransactionType;
use acom_customer::service::for_promotion as customers;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Days, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    COMPLIANCE_PENDING, EntryRow, EventRow, MISSING_ADDRESS, RewardTier, applicable_rules, entries,
    event, known_event, requirements, rewards, set_event_status,
};
use crate::coupon::POOL_EXHAUSTED;
use crate::loyalty::{NewEntry, insert as ledger_insert, store_date};
use crate::model::{count_text, money_text};
use crate::repo::{self, db};
use crate::rules::{self, LoyaltyRules};
use crate::{check_id, not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DispatchRewardInput {
    /// The event.
    #[schemars(regex(pattern = r"^evt_[a-z0-9_]+$"), example = &"evt_0311")]
    pub event_id: String,
    /// Only these winning entries. Default: every winner whose reward was not sent.
    #[schemars(inner(regex(pattern = r"^ent_[a-z0-9_]+$")), example = ["ent_5521"])]
    pub entry_ids: Option<Vec<String>>,
    /// Only the winners of this rank.
    #[schemars(range(min = 1, max = 100), example = 1)]
    pub rank: Option<u32>,
    /// Tell the winners (a stubbed message; nothing leaves the store). Default true.
    #[schemars(example = true)]
    pub notify: Option<bool>,
}

/// One reward to make.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RewardPlan {
    pub id: String,
    pub entry_id: String,
    pub customer_id: String,
    pub rank: u32,
    pub reward_type: String,
    pub reward_ref: Option<String>,
    pub reward_value: Option<Money>,
    pub fulfillment: String,
    /// `dispatched`, or `compliance_pending` while a store rule waits on the winner.
    pub status: String,
    pub requirements: Vec<String>,
    /// What sending makes: the ledger entry, the member coupon, or the stub's reference.
    pub fulfillment_ref: Option<String>,
    /// For an issued coupon: when the copy expires.
    pub owned_through: Option<DateTime<Utc>>,
}

/// What a run of `dispatch_reward` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DispatchRun {
    pub event_id: String,
    pub batch_id: String,
    pub rewards: Vec<RewardPlan>,
    pub notify: bool,
}

pub struct DispatchReward;

impl Operation for DispatchReward {
    const META: OpMeta = OpMeta {
        address: "promotion.event.dispatch_reward",
        tool: "dispatch_reward",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/promotion/events/{event_id}/rewards",
        summary: "Send winners their rewards the way each reward type is sent: store points, a coupon, a shipment or an email request, or a manual record. Winners a store compliance rule waits on are held. Reaches customers, so a person always approves; sent rewards are not taken back.",
        consumes: &[Slot::one("event_id"), Slot::many("entry_id")],
        produces: &[Slot::many("reward_id")],
        checks: &[
            "event.compliance.on_unsatisfied",
            "event.reward_types.ship_to",
        ],
        errors: &[
            COMPLIANCE_PENDING,
            MISSING_ADDRESS,
            POOL_EXHAUSTED,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::SEND,
        person_scope: "",
    };
    type Input = DispatchRewardInput;
    type Output = Value;
}

/// Plans the rewards of `winners` (won entries without a reward yet). Shared with the instant
/// results of `enter_event`.
pub(crate) async fn plan_rewards(
    tx: &mut TxCx,
    e: &EventRow,
    winners: &[EntryRow],
) -> Result<(Vec<RewardPlan>, Vec<Check>), ToolError> {
    let p = tx.policy.clone();
    let kinds = rules::registry(&p, "reward_types")?;
    let applicable = applicable_rules(&p, &e.compliance_jurisdictions)?;
    let hold = rules::word(&p, "promotion.event.compliance.on_unsatisfied")? == "hold_dispatch";
    let tiers: BTreeMap<u32, RewardTier> = e.tiers().into_iter().map(|t| (t.rank, t)).collect();
    let customers_wanted: Vec<String> = winners.iter().map(|w| w.customer_id.clone()).collect();
    let addresses: BTreeMap<String, String> = customers::shipping_addresses(tx, &customers_wanted)
        .await?
        .into_iter()
        .collect();
    let mut plans = Vec::new();
    let mut missing = Vec::new();
    let mut coupons: BTreeMap<String, i64> = BTreeMap::new();
    for w in winners {
        let rank = u32::try_from(w.rank.unwrap_or(1)).unwrap_or(1);
        let Some(tier) = tiers.get(&rank) else {
            continue;
        };
        let fulfillment = kinds
            .iter()
            .find(|k| k.id == tier.reward_type)
            .and_then(|k| k.fulfillment.clone())
            .unwrap_or_else(|| "manual".into());
        let needs = requirements(&applicable, tier);
        let held = hold && !needs.is_empty();
        let mut owned_through = None;
        let fulfillment_ref = if held {
            None
        } else {
            match fulfillment.as_str() {
                "grant" => Some(repo::new_id("lyt")),
                "issue" => {
                    let id = tier.reward_ref.clone().unwrap_or_default();
                    let c = repo::coupon(tx, &id)
                        .await?
                        .ok_or_else(|| not_found("coupon", &id, "search_coupons"))?;
                    owned_through = c.valid_through;
                    *coupons.entry(c.id).or_insert(0) += 1;
                    Some(acom_core::ids::MemberCouponId::generate().to_string())
                }
                "ship" => match addresses.get(&w.customer_id) {
                    Some(a) => Some(format!("stub_shipment:{a}")),
                    None => {
                        missing.push(w.customer_id.clone());
                        None
                    }
                },
                "email" => Some(format!("stub_email:{}", w.customer_id)),
                _ => None,
            }
        };
        plans.push(RewardPlan {
            id: repo::new_id("rwd"),
            entry_id: w.id.clone(),
            customer_id: w.customer_id.clone(),
            rank,
            reward_type: tier.reward_type.clone(),
            reward_ref: tier.reward_ref.clone(),
            reward_value: tier.reward_value,
            fulfillment,
            status: if held {
                "compliance_pending"
            } else {
                "dispatched"
            }
            .into(),
            requirements: needs,
            fulfillment_ref,
            owned_through,
        });
    }
    if !missing.is_empty() {
        return Err(ToolError::new(
            MISSING_ADDRESS,
            format!(
                "{} product winner{} no shipping address on file; ask them for one first.",
                count_text(missing.len() as i64),
                if missing.len() == 1 { " has" } else { "s have" }
            ),
        )
        .detail(json!({"rule": "event.reward_types.ship_to", "items": missing})));
    }
    let pool = rules::integer(&p, "promotion.coupon.total_pool_limit")?;
    if pool > 0 {
        for (coupon, n) in &coupons {
            let issued = repo::issued_count(tx, coupon).await?;
            if issued + n > pool {
                return Err(ToolError::new(
                    POOL_EXHAUSTED,
                    format!("Only {} copies of {coupon} are left; the rewards need {n}.", (pool - issued).max(0)),
                )
                .detail(json!({"limit": pool, "actual": issued + n, "policy_key": "coupon.total_pool_limit"})));
            }
        }
    }
    let held: Vec<&RewardPlan> = plans
        .iter()
        .filter(|r| r.status == "compliance_pending")
        .collect();
    let shipped = plans
        .iter()
        .filter(|r| r.fulfillment == "ship" && r.status == "dispatched")
        .count();
    let checks = vec![
        Check {
            rule: "event.compliance.on_unsatisfied".into(),
            // Holding is what the rule asks for, so the check passes and says who waits.
            passed: true,
            detail: Some(if held.is_empty() {
                "No winner waits on a store rule".into()
            } else {
                format!(
                    "{} winner{} not sent the required info ({}); held",
                    count_text(held.len() as i64),
                    if held.len() == 1 { " has" } else { "s have" },
                    held[0].requirements.join(", ")
                )
            }),
            fix_tool: None,
        },
        Check {
            rule: "event.reward_types.ship_to".into(),
            passed: true,
            detail: Some(if shipped == 0 {
                "Nothing to ship".into()
            } else {
                format!(
                    "Shipping address on file for {} product winner{}",
                    count_text(shipped as i64),
                    if shipped == 1 { "" } else { "s" }
                )
            }),
            fix_tool: None,
        },
    ];
    Ok((plans, checks))
}

/// Makes the planned rewards: the ledger grants and coupons, then the reward rows. A reward
/// already made for its entry and rank is left alone.
pub(crate) async fn make_rewards(
    tx: &mut TxCx,
    event: &EventRow,
    batch_id: &str,
    plans: &[RewardPlan],
) -> Result<Vec<String>, ToolError> {
    let now = tx.clock.now();
    let window = LoyaltyRules::read(&tx.policy)?.window_days;
    let today = store_date(&tx.time, now);
    let mut made = Vec::new();
    for r in plans {
        let done: Option<String> = sqlx::query_scalar(
            "SELECT id FROM promotion.event_reward WHERE tenant_id = $1 AND entry_id = $2 AND rank = $3",
        )
        .bind(tx.tenant.as_str())
        .bind(&r.entry_id)
        .bind(i32::try_from(r.rank).unwrap_or(1))
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        if done.is_some() {
            continue;
        }
        if r.status == "dispatched" {
            match (r.fulfillment.as_str(), &r.fulfillment_ref) {
                ("grant", Some(lyt)) => {
                    let amount = r.reward_value.ok_or_else(|| {
                        ToolError::invalid_input("A points reward needs a value.")
                    })?;
                    ledger_insert(
                        tx,
                        &NewEntry {
                            id: lyt.clone(),
                            customer_id: r.customer_id.clone(),
                            transaction_type: LoyaltyTransactionType::Grant,
                            amount,
                            order_id: None,
                            source_transaction_id: None,
                            expires: Some(today + Days::new(u64::try_from(window).unwrap_or(0))),
                            description: Some(format!("{}, rank {} reward", event.name, r.rank)),
                            agent_id: Some(tx.principal.subject.clone()),
                        },
                    )
                    .await?;
                }
                ("issue", Some(mcp)) => {
                    let coupon = r.reward_ref.clone().unwrap_or_default();
                    repo::insert_member_coupons(
                        tx,
                        &coupon,
                        &[(mcp.clone(), r.customer_id.clone())],
                        now,
                        r.owned_through,
                    )
                    .await?;
                }
                _ => {}
            }
        }
        sqlx::query(
            "INSERT INTO promotion.event_reward (tenant_id, id, event_id, entry_id, customer_id,
               rank, reward_type, reward_ref, reward_value, reward_currency, fulfillment,
               reward_status, compliance_requirements, fulfillment_ref, reward_batch_id,
               dispatched_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
             ON CONFLICT (tenant_id, entry_id, rank) DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&r.id)
        .bind(&event.id)
        .bind(&r.entry_id)
        .bind(&r.customer_id)
        .bind(i32::try_from(r.rank).unwrap_or(1))
        .bind(&r.reward_type)
        .bind(&r.reward_ref)
        .bind(r.reward_value.map(|m| m.value))
        .bind(r.reward_value.map(|m| m.currency.as_str().to_owned()))
        .bind(&r.fulfillment)
        .bind(&r.status)
        .bind(&r.requirements)
        .bind(&r.fulfillment_ref)
        .bind(batch_id)
        .bind((r.status == "dispatched").then_some(now))
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        made.push(r.id.clone());
    }
    Ok(made)
}

impl WriteOp<Cx> for DispatchReward {
    type Plan = DispatchRun;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: DispatchRewardInput,
    ) -> Result<Planned<DispatchRun>, ToolError> {
        for id in input.entry_ids.iter().flatten() {
            check_id("entry_ids", "ent", id, false)?;
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let e = known_event(&mut tx, &input.event_id, "search_events").await?;
        let all = entries(&mut tx, &e.id).await?;
        let paid = rewards(&mut tx, Some(&e.id), None).await?;
        if let Some(ids) = &input.entry_ids {
            if let Some(missing) = ids.iter().find(|id| !all.iter().any(|x| &x.id == *id)) {
                return Err(not_found("event_entry", missing, "get_event"));
            }
            let waiting: Vec<&String> = ids
                .iter()
                .filter(|id| {
                    paid.iter()
                        .any(|r| &r.entry_id == *id && r.reward_status == "compliance_pending")
                })
                .collect();
            if !waiting.is_empty() {
                return Err(ToolError::new(
                    COMPLIANCE_PENDING,
                    format!(
                        "{} still wait{} on the winner's info for a store rule.",
                        crate::model::and_list(
                            &waiting.iter().map(|s| (*s).clone()).collect::<Vec<_>>()
                        ),
                        if waiting.len() == 1 { "s" } else { "" }
                    ),
                )
                .detail(json!({"rule": "event.compliance.on_unsatisfied", "items": waiting}))
                .next("list_my_entries"));
            }
        }
        let winners: Vec<EntryRow> = all
            .into_iter()
            .filter(|x| x.entry_status == "won")
            .filter(|x| {
                input
                    .entry_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&x.id))
            })
            .filter(|x| input.rank.is_none_or(|r| x.rank == i32::try_from(r).ok()))
            .filter(|x| {
                !paid
                    .iter()
                    .any(|r| r.entry_id == x.id && Some(r.rank) == x.rank)
            })
            .collect();
        let (plans, checks) = plan_rewards(&mut tx, &e, &winners).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let changes: Vec<Change> = plans
            .iter()
            .map(|r| Change {
                op: ChangeOp::Create,
                entity: "event_reward".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"entry_id": r.entry_id, "reward_type": r.reward_type,
                           "fulfillment": r.fulfillment, "reward_status": r.status,
                           "reward_value": r.reward_value.map(|m| crate::money(m.value, m.currency.as_str()))}),
                note: (!r.requirements.is_empty()).then(|| format!("Held until the winner sends {}", r.requirements.join(", "))),
            })
            .collect();
        let value: Option<Money> = plans
            .iter()
            .filter_map(|r| r.reward_value)
            .reduce(|a, b| a.checked_add(b).unwrap_or(a));
        let reason = format!(
            "Sends rewards to {} winner{}{}",
            count_text(plans.len() as i64),
            if plans.len() == 1 { "" } else { "s" },
            value.map_or(String::new(), |v| format!(
                " worth {}",
                money_text(v.value, v.currency.as_str())
            ))
        );
        Ok(Planned {
            plan: DispatchRun {
                event_id: e.id.clone(),
                batch_id: repo::new_id("rwb"),
                rewards: plans,
                notify: input.notify.unwrap_or(true),
            },
            changes,
            checks,
            basis: vec![Basis {
                entity: "promotion.event".into(),
                id: e.id,
                version: i64::from(e.version),
            }],
            approval: ApprovalNeed::Needed { reason },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &DispatchRun) -> Result<Committed, ToolError> {
        let e = event(tx, &run.event_id)
            .await?
            .ok_or_else(|| not_found("event", &run.event_id, "search_events"))?;
        let made = make_rewards(tx, &e, &run.batch_id, &run.rewards).await?;
        // Fulfilling while some winner waits; completed once every winner was sent a reward.
        let now = tx.clock.now();
        let status = e.status(now);
        if matches!(status.as_str(), "drawing" | "announced" | "fulfilling") {
            let all = entries(tx, &e.id).await?;
            let paid = rewards(tx, Some(&e.id), None).await?;
            let done = all.iter().filter(|x| x.entry_status == "won").all(|x| {
                paid.iter()
                    .any(|r| r.entry_id == x.id && r.reward_status == "dispatched")
            });
            let to = if done { "completed" } else { "fulfilling" };
            if e.event_status != to {
                set_event_status(tx, &e.id, &e.event_status, to).await?;
            }
        }
        Ok(Committed {
            id: run.batch_id.clone(),
            produced: serde_json::Map::from_iter([
                ("reward_id".to_owned(), json!(made)),
                (
                    "notified".to_owned(),
                    json!(if run.notify {
                        "stub: winners would be told in the app and by email"
                    } else {
                        "no"
                    }),
                ),
            ]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &DispatchRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let paid = rewards(&mut tx, Some(&run.event_id), None).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches: Vec<String> = run
            .rewards
            .iter()
            .filter(|r| {
                !paid.iter().any(|p| {
                    p.entry_id == r.entry_id && p.rank == i32::try_from(r.rank).unwrap_or(0)
                })
            })
            .map(|r| format!("no reward for {}", r.entry_id))
            .collect();
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    /// Sent rewards are not taken back (`UNDO` is not supported); a failed change set leaves
    /// them as they are.
    async fn undo(&self, _: &mut TxCx, _: &DispatchRun, _: &Committed) -> Result<(), ToolError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn always_needs_a_person() {
        assert_eq!(DispatchReward::META.kind, Kind::WriteApproval);
        assert_eq!(DispatchReward::UNDO, UndoSupport::NotSupported);
        let s = closed_schema_for::<DispatchRewardInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }
}
