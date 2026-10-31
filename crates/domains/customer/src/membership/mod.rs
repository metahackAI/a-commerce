//! `membership`: tier programs and tiers, which are data the store defines
//! (`customer.member_program`, `customer.member_program_tier`), never code. Reads
//! (`list_tiers`, `get_tier`, `get_member_tier`, `preview_tier_evaluation`,
//! `list_tier_history`) and the writes that always wait for a person (`define_tier_program`,
//! `run_tier_evaluation`, `set_member_tier`). The evaluation rules are in `progress`.

use acom_contract::ToolError;
use acom_core::enums::TierChangeCause;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::access::db;
use crate::progress::{
    Direction, Measure, Member, Move, Movement, TierDef, cents, evaluate, threshold_money,
    window_start,
};
use crate::service::providers;
use crate::{AS_OF_IN_FUTURE, PROGRAM_NOT_FOUND, TIER_NOT_FOUND, repo};

mod define_tier_program;
mod get_member_tier;
mod get_tier;
mod list_tier_history;
mod list_tiers;
mod preview_tier_evaluation;
mod run_tier_evaluation;
mod set_member_tier;

pub use define_tier_program::*;
pub use get_member_tier::*;
pub use get_tier::*;
pub use list_tier_history::*;
pub use list_tiers::*;
pub use preview_tier_evaluation::*;
pub use run_tier_evaluation::*;
pub use set_member_tier::*;

fn tier_not_found(id: &str) -> ToolError {
    ToolError::new(TIER_NOT_FOUND, format!("There is no tier {id}."))
        .detail(json!({"entity": "tier", "id": id}))
        .next("list_tiers")
}

/// `{value, unit_text}` of a tier's points multiplier, or null.
fn points_json(t: &repo::TierRow) -> Value {
    match (
        &t.membership_points_earned_value,
        &t.membership_points_earned_unit_text,
    ) {
        (Some(v), unit) => json!({"value": v, "unit_text": unit}),
        _ => Value::Null,
    }
}

async fn member_count(tx: &mut TxCx, program: &str, tier: &str) -> Result<i64, ToolError> {
    Ok(repo::member_counts(tx, Some(program))
        .await?
        .into_iter()
        .find(|(t, _)| t == tier)
        .map_or(0, |(_, n)| n))
}

/// `Spend at least 300.00 USD in 6 months`, one line per requirement.
pub(crate) fn requirement_text(req: &Value) -> String {
    let metric = req
        .get("metric")
        .and_then(Value::as_str)
        .unwrap_or("custom");
    let window = req.get("window").map(|w| {
        let length = w.get("length").and_then(Value::as_u64).unwrap_or(0);
        let unit = w.get("unit").and_then(Value::as_str).unwrap_or("month");
        match w.get("type").and_then(Value::as_str) {
            Some("lifetime") => " in total".to_owned(),
            _ => format!(" in {length} {unit}{}", if length == 1 { "" } else { "s" }),
        }
    });
    let window = window.unwrap_or_default();
    match metric {
        "spend" => match threshold_money(req, "threshold") {
            Some(m) => {
                let m = cents(m.value, m.currency);
                format!("Spend at least {} {}{window}", m.value, m.currency)
            }
            None => format!("Spend{window}"),
        },
        "order_count" => format!(
            "At least {} delivered orders{window}",
            req.get("threshold").and_then(Value::as_i64).unwrap_or(0)
        ),
        "invite" => "Invited".to_owned(),
        "manual" => "Set by an operator".to_owned(),
        other => other.replace('_', " "),
    }
}

// ── evaluation (preview_tier_evaluation, run_tier_evaluation, define_tier_program) ───────

pub(crate) fn program_not_found(id: &str) -> ToolError {
    ToolError::new(PROGRAM_NOT_FOUND, format!("There is no program {id}."))
        .detail(json!({"entity": "program", "id": id}))
        .next("list_tiers")
}

/// Who an evaluation looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationScope {
    /// Every active member of the program.
    All,
    /// The members of a segment (`segment_id`). Segments belong to promotion, which customer
    /// cannot read yet; refused for now.
    Segment,
    /// The members in `customer_ids`.
    Customers,
}

impl EvaluationScope {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Segment => "segment",
            Self::Customers => "customers",
        }
    }
}

/// Checks the scope's fields; `Some(ids)` for `customers`, `None` for everyone.
pub(crate) fn scope_ids(
    scope: EvaluationScope,
    segment_id: Option<&str>,
    customer_ids: Option<&[String]>,
    tool: &'static str,
) -> Result<Option<Vec<String>>, ToolError> {
    match scope {
        EvaluationScope::All => Ok(None),
        EvaluationScope::Customers => match customer_ids {
            Some(ids) if !ids.is_empty() => Ok(Some(ids.to_vec())),
            _ => Err(
                ToolError::invalid_input("`scope: customers` needs `customer_ids`.")
                    .detail(json!({"field": "customer_ids", "reason": "missing"}))
                    .next(tool),
            ),
        },
        EvaluationScope::Segment => Err(ToolError::invalid_input(match segment_id {
            Some(_) => {
                "Segments are not available to tier evaluations yet; use scope all or customers."
            }
            None => "`scope: segment` needs `segment_id`; segments are not available yet either.",
        })
        .detail(json!({"field": "segment_id", "reason": "unsupported"}))
        .next(tool)),
    }
}

/// `as_of` from the input: now when left out, never in the future.
pub(crate) fn as_of(
    cx: &Cx,
    s: Option<&str>,
    tool: &'static str,
) -> Result<DateTime<Utc>, ToolError> {
    let now = cx.clock.now();
    let t = crate::access::parse_time(cx, "as_of", s)?.unwrap_or(now);
    if t > now {
        return Err(ToolError::new(
            AS_OF_IN_FUTURE,
            "`as_of` is in the future; results are not known yet.",
        )
        .detail(json!({"field": "as_of", "reason": "future"}))
        .next(tool));
    }
    Ok(t)
}

/// The tiers of a program as evaluations see them.
pub(crate) fn tier_defs(tiers: &[repo::TierRow]) -> Vec<TierDef> {
    tiers
        .iter()
        .map(|t| TierDef::from_stored(&t.id, t.rank, &t.has_tier_requirement, &t.has_tier_benefit))
        .collect()
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct MemberRow {
    customer_id: String,
    name: String,
    tier_id: String,
    tier_since: DateTime<Utc>,
    override_active: bool,
    in_grace: bool,
}

/// One member's place in the program before an evaluation (kept so a run can be undone).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    pub customer_id: String,
    pub name: String,
    pub tier_id: String,
    pub tier_since: DateTime<Utc>,
}

/// The active and dormant members of a program (or the listed ones), with whether a manual tier
/// holds at `at` and whether they are in a grace period.
async fn members(
    tx: &mut TxCx,
    program_id: &str,
    ids: Option<&[String]>,
    at: DateTime<Utc>,
) -> Result<Vec<MemberRow>, ToolError> {
    sqlx::query_as::<_, MemberRow>(
        "SELECT m.customer_id, p.name, m.tier_id, m.tier_since,
           (m.tier_override IS NOT NULL
            AND COALESCE((m.tier_override->>'expires_at')::timestamptz > $3, true)) AS override_active,
           COALESCE((SELECT e.cause = 'grace_started' FROM customer.program_membership_event e
                     WHERE e.tenant_id = m.tenant_id AND e.customer_id = m.customer_id
                       AND e.member_program_id = m.member_program_id
                     ORDER BY e.position DESC LIMIT 1), false) AS in_grace
         FROM customer.program_membership m
         JOIN customer.person p ON p.tenant_id = m.tenant_id AND p.id = m.customer_id
         WHERE m.tenant_id = $1 AND m.member_program_id = $2
           AND p.member_state IN ('active', 'dormant')
           AND ($4::text[] IS NULL OR m.customer_id = ANY($4))
         ORDER BY m.customer_id",
    )
    .bind(tx.tenant.as_str())
    .bind(program_id)
    .bind(at)
    .bind(ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// An evaluation: each member's move under `tiers` at `at`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Outcome {
    pub moves: Vec<(Placed, Move)>,
    /// Orders could be read (sales connected); without them nobody moves.
    pub measured: bool,
    pub settle_days: i64,
}

/// Evaluates the members of a program under the given tiers (the stored ones, or a new
/// definition `define_tier_program` previews). Reads each member's orders through sales.
pub(crate) async fn evaluate_members(
    tx: &mut TxCx,
    program: &repo::ProgramRow,
    tiers: &[TierDef],
    evaluation: &Value,
    movement: &Value,
    ids: Option<&[String]>,
    at: DateTime<Utc>,
) -> Result<Outcome, ToolError> {
    let policy_tier = tx
        .policy
        .lookup("customer.tier")
        .cloned()
        .unwrap_or(Value::Null);
    let settle_days = evaluation
        .get("settle_days")
        .and_then(Value::as_i64)
        .or_else(|| {
            policy_tier
                .pointer("/evaluation/settle_days")
                .and_then(Value::as_i64)
        })
        .unwrap_or(0);
    let measure = Measure {
        default_window: policy_tier
            .pointer("/requirement/window")
            .cloned()
            .unwrap_or(Value::Null),
        default_channels: policy_tier
            .pointer("/requirement/counts/channels")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        settle_days,
        as_of: at,
        time: tx.time,
    };
    let rules = Movement::from_values(
        movement,
        policy_tier.get("movement").unwrap_or(&Value::Null),
    );
    // The widest window of the program bounds the orders read.
    let since = tiers
        .iter()
        .flat_map(|t| t.requirements.iter())
        .map(|r| {
            window_start(
                r.get("window").unwrap_or(&measure.default_window),
                at,
                tx.time,
            )
        })
        .min()
        .unwrap_or(at);
    let rows = members(tx, &program.id, ids, at).await?;
    let reader = providers::member_orders();
    let mut moves = Vec::with_capacity(rows.len());
    for r in rows {
        let orders = match reader {
            Some(read) => Some(read(tx, &r.customer_id, since).await?),
            None => None,
        };
        let member = Member {
            customer_id: r.customer_id.clone(),
            name: r.name.clone(),
            tier_id: r.tier_id.clone(),
            override_active: r.override_active,
            in_grace: r.in_grace,
        };
        let m = evaluate(tiers, &member, orders.as_deref(), &measure, &rules);
        moves.push((
            Placed {
                customer_id: r.customer_id,
                name: r.name,
                tier_id: r.tier_id,
                tier_since: r.tier_since,
            },
            m,
        ));
    }
    Ok(Outcome {
        moves,
        measured: reader.is_some(),
        settle_days,
    })
}

/// Moves grouped by path: `(from, to, direction, members)`, in a stable order.
pub(crate) fn grouped(moves: &[(Placed, Move)]) -> Vec<(String, String, Direction, i64)> {
    let mut out: Vec<(String, String, Direction, i64)> = Vec::new();
    for (_, m) in moves {
        match out
            .iter_mut()
            .find(|(f, t, d, _)| *f == m.from_tier_id && *t == m.to_tier_id && *d == m.direction)
        {
            Some(e) => e.3 += 1,
            None => out.push((m.from_tier_id.clone(), m.to_tier_id.clone(), m.direction, 1)),
        }
    }
    out.sort_by(|a, b| (a.2.word(), &a.0, &a.1).cmp(&(b.2.word(), &b.0, &b.1)));
    out
}

/// The benefits a member gets on entering a tier (`coupon_on_entry`).
pub(crate) fn entry_benefits(tiers: &[TierDef], tier_id: &str) -> Vec<Value> {
    tiers
        .iter()
        .find(|t| t.id == tier_id)
        .and_then(|t| t.benefits.as_array())
        .map(|l| {
            l.iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("coupon_on_entry"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Whether an evaluation of the program is running now (`tier.evaluation.one_run_at_a_time`).
pub(crate) async fn running(tx: &mut TxCx, program_id: &str) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM customer.tier_evaluation
         WHERE tenant_id = $1 AND member_program_id = $2 AND action_status = 'active'
         ORDER BY start_time DESC LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(program_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Applies the moves of an evaluation run: tiers change and history rows are added (a grace
/// period only adds history). Returns the moves that changed a tier.
pub(crate) async fn apply_moves(
    tx: &mut TxCx,
    program_id: &str,
    evaluation_id: Option<&str>,
    moves: &[(Placed, Move)],
    cause: TierChangeCause,
) -> Result<usize, ToolError> {
    let now = tx.clock.now();
    let agent = tx.principal.subject.clone();
    let mut changed = 0;
    for (p, m) in moves {
        match m.direction {
            Direction::Up | Direction::Down => {
                repo::set_membership_tier(tx, &p.customer_id, program_id, &m.to_tier_id, now, None)
                    .await?;
                changed += 1;
            }
            Direction::Grace => {}
            Direction::Same | Direction::Override => continue,
        }
        let cause = match m.direction {
            Direction::Grace => TierChangeCause::GraceStarted,
            _ => cause,
        };
        repo::add_history(
            tx,
            &repo::TierChange {
                customer_id: p.customer_id.clone(),
                program_id: program_id.to_owned(),
                from_tier_id: Some(m.from_tier_id.clone()),
                to_tier_id: Some(m.to_tier_id.clone()),
                cause: cause.as_str().into(),
                evaluation_id: evaluation_id.map(str::to_owned),
                snapshot: m.snapshot.clone(),
                agent_id: agent.clone(),
                reason: Some(m.reason.clone()),
                expires: None,
            },
        )
        .await?;
    }
    Ok(changed)
}

/// Puts members back where they were before a run (undo); history notes it.
pub(crate) async fn revert_moves(
    tx: &mut TxCx,
    program_id: &str,
    evaluation_id: Option<&str>,
    moves: &[(Placed, Move)],
    cause: TierChangeCause,
) -> Result<(), ToolError> {
    let agent = tx.principal.subject.clone();
    for (p, m) in moves {
        if !matches!(m.direction, Direction::Up | Direction::Down) {
            continue;
        }
        repo::set_membership_tier(
            tx,
            &p.customer_id,
            program_id,
            &p.tier_id,
            p.tier_since,
            None,
        )
        .await?;
        repo::add_history(
            tx,
            &repo::TierChange {
                customer_id: p.customer_id.clone(),
                program_id: program_id.to_owned(),
                from_tier_id: Some(m.to_tier_id.clone()),
                to_tier_id: Some(p.tier_id.clone()),
                cause: cause.as_str().into(),
                evaluation_id: evaluation_id.map(str::to_owned),
                snapshot: None,
                agent_id: agent.clone(),
                reason: Some("Undone".into()),
                expires: None,
            },
        )
        .await?;
    }
    Ok(())
}

/// Notices about tier moves (`tier.notify.on`): `upgrade`, `downgrade`, `downgrade_warning`.
pub(crate) fn notices(policy: &acom_policy::PolicySet, moves: &[(Placed, Move)]) -> usize {
    let on: Vec<String> = policy
        .lookup("customer.tier.notify.on")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    moves
        .iter()
        .filter(|(_, m)| {
            let kind = match m.direction {
                Direction::Up => "upgrade",
                Direction::Down => "downgrade",
                Direction::Grace => "downgrade_warning",
                _ => return false,
            };
            on.iter().any(|o| o == kind)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use acom_contract::ReadOp;

    use super::*;

    #[test]
    fn describes_requirements_in_a_line() {
        let gold = json!({"metric": "spend", "threshold": {"value": 300.00, "currency": "USD"},
                          "window": {"type": "rolling", "length": 6, "unit": "month"}});
        assert_eq!(
            requirement_text(&gold),
            "Spend at least 300.00 USD in 6 months"
        );
        assert_eq!(requirement_text(&json!({"metric": "invite"})), "Invited");
    }

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reads_tiers_and_memberships() {
        let mut tx = crate::testdb::tx("adm_sj").await;
        let tiers = repo::tiers(&mut tx, Some("prg_rewards")).await.unwrap();
        assert_eq!(
            tiers.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["vip", "gold", "member"]
        );
        let alex = repo::memberships(&mut tx, "cus_alex").await.unwrap();
        assert_eq!(alex[0].tier_id, "member");
        let cx = crate::testdb::cx("cus_alex").await;
        let out = GetMemberTier
            .read(
                &cx,
                GetMemberTierInput {
                    customer_id: "me".into(),
                    program_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(out["tier_id"], "member");
        assert_eq!(out["source"], "entry");
    }

    /// A fixed order reader for the tests: Alex spent 312.00 in September; nobody else bought.
    fn fake_orders<'a>(
        _tx: &'a mut TxCx,
        customer_id: &'a str,
        _since: DateTime<Utc>,
    ) -> crate::service::providers::Reading<'a, Vec<crate::service::providers::MemberOrder>> {
        Box::pin(async move {
            let time = acom_core::StoreTime::new(chrono_tz::America::New_York);
            Ok(if customer_id == "cus_alex" {
                vec![crate::service::providers::MemberOrder {
                    order_id: "ord_test_alex".into(),
                    order_number: "29999".into(),
                    amount: acom_core::Money::new(
                        rust_decimal::Decimal::new(31200, 2),
                        acom_core::Currency::USD,
                    ),
                    sales_channel: "web".into(),
                    delivered_at: Some(time.parse("2026-09-15T12:00").unwrap()),
                }]
            } else {
                vec![]
            })
        })
    }

    /// Integration: evaluation, a manual tier and the history, rolled back. Installs a fixed
    /// order reader for this test process. Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn evaluates_sets_tiers_by_hand_and_keeps_history() {
        use acom_contract::{ReadOp, WriteOp};

        crate::service::providers::provide_member_orders(fake_orders);
        let cx = crate::testdb::cx("adm_sj").await;
        let preview = PreviewTierEvaluation
            .read(
                &cx,
                PreviewTierEvaluationInput {
                    program_id: "prg_rewards".into(),
                    as_of: None,
                    scope: Some(EvaluationScope::Customers),
                    segment_id: None,
                    customer_ids: Some(vec!["cus_alex".into(), "cus_jamie".into()]),
                    direction: None,
                    group_by: None,
                    fields: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap();
        let items = preview["items"].as_array().unwrap();
        let alex = items
            .iter()
            .find(|i| i["customer_id"] == "cus_alex")
            .unwrap();
        assert_eq!(
            (alex["to_tier_id"].as_str(), alex["direction"].as_str()),
            (Some("gold"), Some("up"))
        );
        assert_eq!(alex["name"], "A***");
        // Jamie is VIP and VIP can also come by invite: kept, so not listed.
        assert!(items.iter().all(|i| i["customer_id"] != "cus_jamie"));

        let run = RunTierEvaluation
            .preview(
                &cx,
                RunTierEvaluationInput {
                    program_id: "prg_rewards".into(),
                    as_of: None,
                    scope: Some(EvaluationScope::Customers),
                    segment_id: None,
                    customer_ids: Some(vec!["cus_alex".into()]),
                    notify: None,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            run.approval,
            acom_contract::ApprovalNeed::Needed { .. }
        ));
        let mut tx = cx.begin().await.unwrap();
        let done = RunTierEvaluation.commit(&mut tx, &run.plan).await.unwrap();
        assert_eq!(done.produced["summary"]["up"], 1);
        assert_eq!(done.produced["notices"]["delivered"], false);
        let m = repo::memberships(&mut tx, "cus_alex").await.unwrap();
        assert_eq!(m[0].tier_id, "gold");
        RunTierEvaluation
            .undo(&mut tx, &run.plan, &done)
            .await
            .unwrap();
        let m = repo::memberships(&mut tx, "cus_alex").await.unwrap();
        assert_eq!(m[0].tier_id, "member");
        drop(tx);

        // A manual tier until a date, then undone.
        let set = SetMemberTier
            .preview(
                &cx,
                SetMemberTierInput {
                    customer_id: "cus_alex".into(),
                    tier_id: Some("vip".into()),
                    reason: "Goodwill after a defective pair of boots, inq_771".into(),
                    expires_at: Some("2027-03-31T23:59".into()),
                    after_expiry: None,
                    issue_entry_benefits: None,
                    clear_override: None,
                },
            )
            .await
            .unwrap();
        let too_long = SetMemberTier
            .preview(
                &cx,
                SetMemberTierInput {
                    customer_id: "cus_alex".into(),
                    tier_id: Some("vip".into()),
                    reason: "Goodwill".into(),
                    expires_at: Some("2028-03-31T23:59".into()),
                    after_expiry: None,
                    issue_entry_benefits: None,
                    clear_override: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(too_long.code, "override_too_long");
        let mut tx = cx.begin().await.unwrap();
        let done = SetMemberTier.commit(&mut tx, &set.plan).await.unwrap();
        let m = repo::memberships(&mut tx, "cus_alex").await.unwrap();
        assert_eq!(m[0].tier_id, "vip");
        assert!(m[0].tier_override.is_some());
        let causes: Vec<String> = sqlx::query_scalar(
            "SELECT cause FROM customer.program_membership_event
             WHERE customer_id = 'cus_alex' ORDER BY position",
        )
        .fetch_all(&mut *tx.tx)
        .await
        .unwrap();
        assert_eq!(causes, ["entry", "manual_override"]);
        SetMemberTier.undo(&mut tx, &set.plan, &done).await.unwrap();
        let m = repo::memberships(&mut tx, "cus_alex").await.unwrap();
        assert_eq!(
            (m[0].tier_id.as_str(), m[0].tier_override.is_none()),
            ("member", true)
        );
        drop(tx);

        let history = ListTierHistory
            .read(
                &cx,
                ListTierHistoryInput {
                    customer_id: "cus_jamie".into(),
                    program_id: None,
                    at_from: None,
                    at_through: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(history["total"], 3);
        assert_eq!(history["items"][0]["to_tier_id"], "vip");
        assert_eq!(history["items"][0]["evaluation_id"], "tev_2609");
    }

    /// Integration: a new version of the program (a Platinum tier, VIP needs more), undone.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn defines_a_new_version_and_undoes_it() {
        use acom_contract::WriteOp;
        use acom_core::{Currency, Money};
        use rust_decimal::Decimal;

        let usd = |v: i64| Money::new(Decimal::new(v * 100, 2), Currency::USD);
        let spend = |v: i64| RequirementInput {
            metric: Metric::Spend,
            threshold: Some(usd(v)),
            threshold_count: None,
            retain_threshold: None,
            retain_threshold_count: None,
            window: None,
            counts: None,
            custom_metric: None,
        };
        let tier = |id: &str, rank: u32, reqs: Vec<RequirementInput>| TierInput {
            id: id.into(),
            name: id.to_uppercase(),
            rank,
            requirement_combine: None,
            has_tier_requirement: Some(reqs),
            has_tier_benefit: None,
        };
        let input = || DefineTierProgramInput {
            program_id: "prg_rewards".into(),
            name: "Northwind Rewards".into(),
            has_tiers: vec![
                tier("member", 1, vec![]),
                tier("gold", 2, vec![spend(300)]),
                tier("vip", 3, vec![spend(1000)]),
                tier("platinum", 4, vec![spend(2000)]),
            ],
            evaluation: None,
            movement: None,
            tier_migrations: None,
            takes_effect: None,
        };
        let cx = crate::testdb::cx("adm_sj").await;
        let planned = DefineTierProgram.preview(&cx, input()).await.unwrap();
        assert!(planned.changes.iter().any(|c| c.id == "platinum"));
        assert!(
            planned.checks.iter().all(|c| c.passed),
            "{:?}",
            planned.checks
        );
        let mut bad = input();
        bad.has_tiers[0].has_tier_requirement = Some(vec![spend(10)]);
        let e = DefineTierProgram.preview(&cx, bad).await.unwrap_err();
        assert_eq!(e.code, "entry_tier_missing");
        let mut tx = cx.begin().await.unwrap();
        let done = DefineTierProgram
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap();
        let tiers = repo::tiers(&mut tx, Some("prg_rewards")).await.unwrap();
        assert_eq!(tiers[0].id, "platinum");
        DefineTierProgram
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap();
        let tiers = repo::tiers(&mut tx, Some("prg_rewards")).await.unwrap();
        assert_eq!(
            tiers.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["vip", "gold", "member"]
        );
        // Rolled back on drop.
    }
}
