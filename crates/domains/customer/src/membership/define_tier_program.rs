//! `define_tier_program`: create a tier program or save a new version of it (tiers added, removed,
//! renamed, reordered; requirements, benefits, evaluation schedule and movement rules). Members'
//! tiers and benefits change, so a person always approves. The preview shows how many members
//! would move under the new definition with today's results. Every version is kept
//! (`customer.member_program_version`) and undo restores the one before.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::{SalesChannel, TierChangeCause};
use acom_store::{Cx, TxCx};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{Placed, apply_moves, evaluate_members, grouped, revert_moves};
use crate::access::{always, db, policy, policy_number, staff_only};
use crate::progress::{Direction, Move, TierDef, requirement_list};
use crate::service::providers;
use crate::{
    COUPON_NOT_FOUND, ENTRY_TIER_MISSING, MIGRATION_REQUIRED, TIER_IN_USE, TIER_LIMIT_EXCEEDED,
    TIER_RANK_CONFLICT, repo,
};

/// How several requirements of a tier combine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Combine {
    /// Any one of them is enough.
    Any,
    /// All of them are needed.
    All,
}

/// What a requirement measures (enums.md §1.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    /// Money spent (`threshold`).
    Spend,
    /// Delivered orders (`threshold_count`).
    OrderCount,
    /// Points earned (`threshold_count`).
    PointsEarned,
    /// Only by `set_member_tier`.
    Manual,
    /// By accepting an invite.
    Invite,
    /// An analytics metric (`custom_metric`).
    Custom,
}

/// The kind of window results are counted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindowType {
    /// The last `length` units.
    Rolling,
    /// Since the start of the current month or year.
    Calendar,
    /// Since joining.
    Lifetime,
}

/// A unit of time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TimeUnit {
    /// Days.
    Day,
    /// Weeks.
    Week,
    /// Months.
    Month,
    /// Years.
    Year,
}

/// The window results are counted in.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WindowInput {
    /// rolling, calendar or lifetime.
    #[serde(rename = "type")]
    #[schemars(example = &"rolling")]
    pub window_type: WindowType,
    /// How many units (rolling and calendar).
    #[schemars(range(min = 1, max = 120), example = 6)]
    pub length: Option<u32>,
    /// day, week, month or year.
    #[schemars(example = &"month")]
    pub unit: Option<TimeUnit>,
}

/// What counts towards a requirement.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CountsInput {
    /// Sales channels that count. The store's default when left out.
    #[schemars(example = ["web", "app"])]
    pub channels: Option<Vec<SalesChannel>>,
    /// Take returned and cancelled amounts off.
    #[schemars(example = true)]
    pub net_of_returns: Option<bool>,
    /// Count what was paid with points.
    #[schemars(example = true)]
    pub include_loyalty_spent: Option<bool>,
}

/// One requirement of a tier.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequirementInput {
    /// spend; order_count; points_earned; manual; invite; custom.
    #[schemars(example = &"spend")]
    pub metric: Metric,
    /// The amount to reach, for `spend`.
    #[schemars(example = money_example())]
    pub threshold: Option<Money>,
    /// The number to reach, for `order_count` and `points_earned`.
    #[schemars(range(min = 1), example = 5)]
    pub threshold_count: Option<u32>,
    /// The amount that keeps the tier, for `spend` (lower than `threshold` softens moves).
    #[schemars(example = money_example())]
    pub retain_threshold: Option<Money>,
    /// The number that keeps the tier, for counts.
    #[schemars(range(min = 1), example = 3)]
    pub retain_threshold_count: Option<u32>,
    /// The window results are counted in. The store's default when left out.
    pub window: Option<WindowInput>,
    /// What counts. The store's default when left out.
    pub counts: Option<CountsInput>,
    /// The analytics metric id, for `custom`.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"reviews_written")]
    pub custom_metric: Option<String>,
}

fn money_example() -> Value {
    json!({"value": 300.00, "currency": "USD"})
}

/// The kinds of benefit (enums.md §1.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BenefitType {
    /// Member prices aimed at this tier (`valid_for_member_tier`).
    MemberPrice,
    /// Points earned times `value`.
    PointsMultiplier,
    /// Free shipping, from `min_subtotal` when given.
    FreeShipping,
    /// Free returns.
    FreeReturns,
    /// A coupon (`coupon_id`, `quantity`) on entering the tier.
    CouponOnEntry,
    /// A coupon (`coupon_id`) every `every`.
    RecurringCoupon,
    /// A named benefit (`name`, `description`).
    Custom,
}

/// month or year.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Every {
    /// Every month.
    Month,
    /// Every year.
    Year,
}

/// One benefit of a tier.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BenefitInput {
    /// member_price; points_multiplier; free_shipping; free_returns; coupon_on_entry;
    /// recurring_coupon; custom.
    #[serde(rename = "type")]
    #[schemars(example = &"recurring_coupon")]
    pub benefit_type: BenefitType,
    /// The coupon, for coupon benefits (`search_coupons`).
    #[schemars(regex(pattern = r"^cpn_[a-z0-9_]+$"), example = &"cpn_gold_10")]
    pub coupon_id: Option<String>,
    /// How often a recurring coupon comes.
    #[schemars(example = &"month")]
    pub every: Option<Every>,
    /// How many coupons on entry. 1 when left out.
    #[schemars(range(min = 1, max = 10), example = 1)]
    pub quantity: Option<u32>,
    /// The points multiplier, for `points_multiplier` (1.5 = half again as many points).
    #[schemars(with = "Option<f64>", example = 1.5)]
    pub value: Option<Decimal>,
    /// Free shipping from this subtotal.
    #[schemars(example = money_example())]
    pub min_subtotal: Option<Money>,
    /// The name of a custom benefit.
    #[schemars(length(max = 100), example = &"Priority support")]
    pub name: Option<String>,
    /// What a custom benefit gives.
    #[schemars(length(max = 300), example = &"Inquiries are answered first.")]
    pub description: Option<String>,
}

/// One tier of the program.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TierInput {
    /// The tier id: a lowercase word the store chooses, unique in the store, never `everyone`.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"gold")]
    pub id: String,
    /// The name people see.
    #[schemars(length(min = 1, max = 60), example = &"Gold")]
    pub name: String,
    /// Higher is better; 1 is the entry tier. No two tiers share a rank.
    #[schemars(range(min = 1, max = 100), example = 2)]
    pub rank: u32,
    /// any (default) or all: how several requirements combine.
    #[schemars(example = &"any")]
    pub requirement_combine: Option<Combine>,
    /// What it takes. None for the entry tier.
    pub has_tier_requirement: Option<Vec<RequirementInput>>,
    /// What members of the tier get.
    pub has_tier_benefit: Option<Vec<BenefitInput>>,
}

/// When tiers are evaluated.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleInput {
    /// day, week, month or year.
    #[schemars(example = &"month")]
    pub every: TimeUnit,
    /// The day of the week (1 Monday) or of the month or year to run on.
    #[schemars(range(min = 1, max = 31), example = 1)]
    pub on_day: Option<u32>,
    /// The time of day, store time, `HH:MM`.
    #[schemars(regex(pattern = r"^[0-2][0-9]:[0-5][0-9]$"), example = &"03:00")]
    pub at: Option<String>,
}

/// The program's evaluation settings.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluationInput {
    /// The regular schedule.
    pub schedule: Option<ScheduleInput>,
    /// Days after delivery before an order counts (returns can still take it back).
    #[schemars(range(max = 90), example = 14)]
    pub settle_days: Option<u32>,
}

/// When members move up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum When {
    /// As soon as they qualify.
    Immediate,
    /// At the scheduled evaluation.
    AtSchedule,
}

/// How members move between tiers.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MovementInput {
    /// immediate or at_schedule.
    #[schemars(example = &"immediate")]
    pub upgrade: Option<When>,
    /// immediate or at_schedule.
    #[schemars(example = &"at_schedule")]
    pub downgrade: Option<When>,
    /// At most this many tiers down in one evaluation.
    #[schemars(range(min = 1, max = 20), example = 1)]
    pub max_steps_down: Option<u32>,
    /// Evaluations a member keeps the tier after missing it, with a warning.
    #[schemars(range(max = 12), example = 1)]
    pub grace_periods: Option<u32>,
}

/// Where the members of a removed tier go.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MigrationInput {
    /// The tier being removed.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"silver")]
    pub from_tier_id: String,
    /// The tier its members move to.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"gold")]
    pub to_tier_id: String,
}

/// When the new definition starts to move members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TakesEffect {
    /// At the next scheduled evaluation.
    NextEvaluation,
    /// Now: members are evaluated when the change runs.
    Immediately,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DefineTierProgramInput {
    /// The program; a new id makes a new program.
    #[schemars(regex(pattern = r"^prg_[a-z0-9_]+$"), example = &"prg_rewards")]
    pub program_id: String,
    /// The program's name.
    #[schemars(length(min = 1, max = 100), example = &"Northwind Rewards")]
    pub name: String,
    /// Every tier of the program; a tier left out is removed.
    #[schemars(length(min = 1))]
    pub has_tiers: Vec<TierInput>,
    /// Evaluation settings. The store's defaults (`tier.evaluation`) when left out.
    pub evaluation: Option<EvaluationInput>,
    /// Movement rules. The store's defaults (`tier.movement`) when left out.
    pub movement: Option<MovementInput>,
    /// Where the members of each removed tier go. Needed when a removed tier has members.
    pub tier_migrations: Option<Vec<MigrationInput>>,
    /// next_evaluation (default) or immediately.
    #[schemars(example = &"next_evaluation")]
    pub takes_effect: Option<TakesEffect>,
}

/// A tier as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredTier {
    pub id: String,
    pub name: String,
    pub rank: i32,
    pub has_tier_requirement: Value,
    pub has_tier_benefit: Value,
    pub points_value: Option<Decimal>,
}

/// A whole program definition as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgramDefinition {
    pub name: String,
    pub evaluation: Value,
    pub movement: Value,
    pub version: i32,
    pub tiers: Vec<StoredTier>,
}

impl ProgramDefinition {
    fn json(&self) -> Value {
        json!({"name": self.name, "evaluation": self.evaluation, "movement": self.movement,
               "has_tiers": self.tiers.iter().map(|t| json!({"id": t.id, "name": t.name,
                   "rank": t.rank, "has_tier_requirement": t.has_tier_requirement,
                   "has_tier_benefit": t.has_tier_benefit})).collect::<Vec<_>>()})
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinePlan {
    pub program_id: String,
    pub before: Option<ProgramDefinition>,
    pub after: ProgramDefinition,
    /// `(from, to, members moved)` for removed tiers.
    pub migrations: Vec<(String, String, Vec<String>)>,
    /// With `takes_effect: immediately`, the moves made when it runs.
    pub moves: Vec<(Placed, Move)>,
}

pub struct DefineTierProgram;

impl Operation for DefineTierProgram {
    const META: OpMeta = OpMeta {
        address: "customer.membership.define_tier_program",
        tool: "define_tier_program",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Put,
        path: "/v1/customer/tier-programs/{program_id}",
        summary: "Create a tier program or save a new version: tiers, requirements, benefits, evaluation schedule and movement. The preview shows how many members would move. A person always approves; undo restores the version before.",
        consumes: &[Slot::one("program_id"), Slot::one("coupon_id")],
        produces: &[Slot::one("program_id"), Slot::many("tier_id")],
        checks: &[
            "tier.entry_tier",
            "tier.rank",
            "tier.requirement.higher_rank_needs_more",
            "tier.program_change.block_if_referenced_by",
            "tier.program_change.benefit_coupons_active",
            "tier.program_change.preview_impact",
        ],
        errors: &[
            TIER_RANK_CONFLICT,
            ENTRY_TIER_MISSING,
            TIER_IN_USE,
            MIGRATION_REQUIRED,
            COUPON_NOT_FOUND,
            TIER_LIMIT_EXCEEDED,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = DefineTierProgramInput;
    type Output = Value;
}

fn invalid(field: &str, message: String) -> ToolError {
    ToolError::invalid_input(message)
        .detail(json!({"field": field, "reason": "format"}))
        .next("define_tier_program")
}

/// One requirement as stored (`threshold` is money for spend, a number for counts).
fn stored_requirement(r: &RequirementInput, tier: &str) -> Result<Value, ToolError> {
    let metric = match r.metric {
        Metric::Spend => "spend",
        Metric::OrderCount => "order_count",
        Metric::PointsEarned => "points_earned",
        Metric::Manual => "manual",
        Metric::Invite => "invite",
        Metric::Custom => "custom",
    };
    let mut v = json!({"metric": metric});
    match r.metric {
        Metric::Spend => {
            let t = r.threshold.ok_or_else(|| {
                invalid(
                    "has_tier_requirement.threshold",
                    format!("{tier}: spend needs `threshold`."),
                )
            })?;
            v["threshold"] = json!(t);
            if let Some(k) = r.retain_threshold {
                v["retain_threshold"] = json!(k);
            }
        }
        Metric::OrderCount | Metric::PointsEarned => {
            let t = r.threshold_count.ok_or_else(|| {
                invalid(
                    "has_tier_requirement.threshold_count",
                    format!("{tier}: {metric} needs `threshold_count`."),
                )
            })?;
            v["threshold"] = json!(t);
            if let Some(k) = r.retain_threshold_count {
                v["retain_threshold"] = json!(k);
            }
        }
        Metric::Custom => {
            let m = r.custom_metric.as_ref().ok_or_else(|| {
                invalid(
                    "has_tier_requirement.custom_metric",
                    format!("{tier}: custom needs `custom_metric`."),
                )
            })?;
            v["custom_metric"] = json!(m);
        }
        Metric::Manual | Metric::Invite => {}
    }
    if let Some(w) = &r.window {
        v["window"] = json!(w);
    }
    if let Some(c) = &r.counts {
        v["counts"] = json!(c);
    }
    Ok(v)
}

fn stored_benefit(b: &BenefitInput, tier: &str) -> Result<(Value, Option<Decimal>), ToolError> {
    let word = match b.benefit_type {
        BenefitType::MemberPrice => "member_price",
        BenefitType::PointsMultiplier => "points_multiplier",
        BenefitType::FreeShipping => "free_shipping",
        BenefitType::FreeReturns => "free_returns",
        BenefitType::CouponOnEntry => "coupon_on_entry",
        BenefitType::RecurringCoupon => "recurring_coupon",
        BenefitType::Custom => "custom",
    };
    let mut v = json!({"type": word});
    let mut points = None;
    match b.benefit_type {
        BenefitType::CouponOnEntry | BenefitType::RecurringCoupon => {
            let c = b.coupon_id.as_ref().ok_or_else(|| {
                ToolError::new(
                    COUPON_NOT_FOUND,
                    format!("{tier}: a coupon benefit names `coupon_id`."),
                )
                .detail(json!({"field": "has_tier_benefit.coupon_id", "reason": "missing"}))
                .next("create_coupon")
            })?;
            v["coupon_id"] = json!(c);
            if b.benefit_type == BenefitType::CouponOnEntry {
                v["quantity"] = json!(b.quantity.unwrap_or(1));
            } else {
                v["every"] = json!(b.every.unwrap_or(Every::Month));
            }
        }
        BenefitType::PointsMultiplier => {
            let x = b.value.filter(|x| *x > Decimal::ZERO).ok_or_else(|| {
                invalid(
                    "has_tier_benefit.value",
                    format!("{tier}: points_multiplier needs a `value` above 0."),
                )
            })?;
            v["value"] = json!(x);
            points = Some(x);
        }
        BenefitType::FreeShipping => {
            if let Some(m) = b.min_subtotal {
                v["min_subtotal"] = json!(m);
            }
        }
        BenefitType::Custom => {
            v["name"] = json!(b.name.clone().unwrap_or_default());
            v["description"] = json!(b.description.clone().unwrap_or_default());
        }
        BenefitType::MemberPrice | BenefitType::FreeReturns => {}
    }
    Ok((v, points))
}

/// The thresholds of one metric by rank, for `tier.requirement.higher_rank_needs_more`.
fn rising(tiers: &[StoredTier]) -> Result<(), String> {
    let mut sorted: Vec<&StoredTier> = tiers.iter().collect();
    sorted.sort_by_key(|t| t.rank);
    for metric in ["spend", "order_count", "points_earned"] {
        let mut last: Option<(String, Decimal)> = None;
        for t in &sorted {
            let req = requirement_list(&t.has_tier_requirement);
            let Some(r) = req.iter().find(|r| r["metric"] == metric) else {
                continue;
            };
            let n = match &r["threshold"] {
                Value::Object(o) => o
                    .get("value")
                    .and_then(|v| serde_json::from_value::<Decimal>(v.clone()).ok()),
                v => serde_json::from_value::<Decimal>(v.clone()).ok(),
            };
            let Some(n) = n else { continue };
            if let Some((id, prev)) = &last
                && n <= *prev
            {
                return Err(format!("{} needs no more {metric} than {id}", t.id));
            }
            last = Some((t.id.clone(), n));
        }
    }
    Ok(())
}

async fn write_definition(
    tx: &mut TxCx,
    program_id: &str,
    d: &ProgramDefinition,
    new: bool,
) -> Result<(), ToolError> {
    if new {
        sqlx::query(
            "INSERT INTO customer.member_program (tenant_id, id, name, evaluation, movement, version)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (tenant_id, id) DO UPDATE SET name = $3, evaluation = $4, movement = $5,
               version = $6, updated_at = now()",
        )
    } else {
        sqlx::query(
            "UPDATE customer.member_program SET name = $3, evaluation = $4, movement = $5,
               version = $6, updated_at = now() WHERE tenant_id = $1 AND id = $2",
        )
    }
    .bind(tx.tenant.as_str())
    .bind(program_id)
    .bind(&d.name)
    .bind(&d.evaluation)
    .bind(&d.movement)
    .bind(d.version)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    // Ranks are unique in a program: move the old ones out of the way first.
    sqlx::query(
        "UPDATE customer.member_program_tier SET rank = rank + 1000
         WHERE tenant_id = $1 AND member_program_id = $2 AND rank < 1000",
    )
    .bind(tx.tenant.as_str())
    .bind(program_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    for t in &d.tiers {
        sqlx::query(
            "INSERT INTO customer.member_program_tier (tenant_id, id, member_program_id, name, rank,
               has_tier_requirement, has_tier_benefit, membership_points_earned_value,
               membership_points_earned_unit_text)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (tenant_id, id) DO UPDATE SET name = $4, rank = $5,
               has_tier_requirement = $6, has_tier_benefit = $7,
               membership_points_earned_value = $8, membership_points_earned_unit_text = $9,
               updated_at = now(), version = customer.member_program_tier.version + 1",
        )
        .bind(tx.tenant.as_str())
        .bind(&t.id)
        .bind(program_id)
        .bind(&t.name)
        .bind(t.rank)
        .bind(&t.has_tier_requirement)
        .bind(&t.has_tier_benefit)
        .bind(t.points_value)
        .bind(t.points_value.map(|_| "points per currency unit"))
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    sqlx::query(
        "INSERT INTO customer.member_program_version (tenant_id, member_program_id, program_version,
           definition, agent_id, defined_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (tenant_id, member_program_id, program_version)
         DO UPDATE SET definition = $4, agent_id = $5, defined_at = $6, updated_at = now()",
    )
    .bind(tx.tenant.as_str())
    .bind(program_id)
    .bind(d.version)
    .bind(d.json())
    .bind(&tx.principal.subject)
    .bind(tx.clock.now())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Moves members between tiers (migrations), with a history row each.
async fn migrate(
    tx: &mut TxCx,
    program_id: &str,
    from: &str,
    to: &str,
    ids: &[String],
    reason: &str,
) -> Result<(), ToolError> {
    let now = tx.clock.now();
    for id in ids {
        repo::set_membership_tier(tx, id, program_id, to, now, None).await?;
        repo::add_history(
            tx,
            &repo::TierChange {
                customer_id: id.clone(),
                program_id: program_id.to_owned(),
                from_tier_id: Some(from.to_owned()),
                to_tier_id: Some(to.to_owned()),
                cause: TierChangeCause::ProgramChange.as_str().into(),
                evaluation_id: None,
                snapshot: None,
                agent_id: tx.principal.subject.clone(),
                reason: Some(reason.to_owned()),
                expires: None,
            },
        )
        .await?;
    }
    Ok(())
}

async fn delete_tiers_not_in(
    tx: &mut TxCx,
    program_id: &str,
    keep: &[String],
) -> Result<(), ToolError> {
    sqlx::query(
        "DELETE FROM customer.member_program_tier
         WHERE tenant_id = $1 AND member_program_id = $2 AND NOT (id = ANY($3))",
    )
    .bind(tx.tenant.as_str())
    .bind(program_id)
    .bind(keep)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

impl WriteOp<Cx> for DefineTierProgram {
    type Plan = DefinePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: DefineTierProgramInput,
    ) -> Result<Planned<DefinePlan>, ToolError> {
        staff_only(cx)?;
        let tiers_max = policy_number(cx, "customer.tier.tiers_max")?;
        let programs_max = policy_number(cx, "customer.tier.programs_max")?;
        if i64::try_from(input.has_tiers.len()).unwrap_or(i64::MAX) > tiers_max {
            return Err(ToolError::new(TIER_LIMIT_EXCEEDED, format!("A program has at most {tiers_max} tiers."))
                .detail(json!({"limit": tiers_max, "actual": input.has_tiers.len(), "policy_key": "tier.tiers_max"}))
                .next("list_tiers"));
        }
        // The tiers as they will be stored.
        let mut tiers: Vec<StoredTier> = Vec::new();
        for t in &input.has_tiers {
            if t.id == "everyone" {
                return Err(invalid(
                    "has_tiers.id",
                    "`everyone` is reserved; pick another tier id.".into(),
                ));
            }
            if tiers.iter().any(|x| x.id == t.id) {
                return Err(ToolError::new(
                    TIER_RANK_CONFLICT,
                    format!("Tier {} is listed twice.", t.id),
                )
                .detail(json!({"field": "has_tiers.id", "id": t.id}))
                .next("define_tier_program"));
            }
            let rank = i32::try_from(t.rank).unwrap_or(i32::MAX);
            if let Some(other) = tiers.iter().find(|x| x.rank == rank) {
                return Err(ToolError::new(
                    TIER_RANK_CONFLICT,
                    format!(
                        "{} and {} both have rank {rank}; ranks must differ.",
                        other.id, t.id
                    ),
                )
                .detail(json!({"rule": "tier.rank", "rank": rank}))
                .next("define_tier_program"));
            }
            let reqs = t
                .has_tier_requirement
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|r| stored_requirement(r, &t.id))
                .collect::<Result<Vec<_>, _>>()?;
            let mut benefits = Vec::new();
            let mut points = None;
            for b in t.has_tier_benefit.as_deref().unwrap_or_default() {
                let (v, p) = stored_benefit(b, &t.id)?;
                points = points.or(p);
                benefits.push(v);
            }
            let combine = match t.requirement_combine.unwrap_or(Combine::Any) {
                Combine::Any => "any",
                Combine::All => "all",
            };
            tiers.push(StoredTier {
                id: t.id.clone(),
                name: t.name.trim().to_owned(),
                rank,
                has_tier_requirement: json!({"combine": combine, "requirements": reqs}),
                has_tier_benefit: json!(benefits),
                points_value: points,
            });
        }
        // Entry tier: the lowest rank, with no requirement (tier.entry_tier).
        let lowest = tiers.iter().min_by_key(|t| t.rank).cloned();
        let entry = lowest
            .as_ref()
            .filter(|t| requirement_list(&t.has_tier_requirement).is_empty());
        let Some(entry) = entry else {
            return Err(ToolError::new(
                ENTRY_TIER_MISSING,
                "The lowest tier must have no requirement: it is where new members start.",
            )
            .detail(json!({"rule": "tier.entry_tier"}))
            .next("define_tier_program"));
        };
        let rising_check = rising(&tiers);
        let defaults = policy(cx, "customer.tier").cloned().unwrap_or(Value::Null);
        let evaluation = match &input.evaluation {
            Some(e) => {
                let mut v = defaults
                    .get("evaluation")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if let Some(s) = &e.schedule {
                    v["schedule"] = json!(s);
                }
                if let Some(d) = e.settle_days {
                    v["settle_days"] = json!(d);
                }
                v
            }
            None => defaults
                .get("evaluation")
                .cloned()
                .unwrap_or_else(|| json!({})),
        };
        let movement = match &input.movement {
            Some(m) => {
                let mut v = defaults
                    .get("movement")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let obj = serde_json::to_value(m).unwrap_or(Value::Null);
                if let (Some(o), Some(new)) = (v.as_object_mut(), obj.as_object()) {
                    for (k, x) in new {
                        if !x.is_null() {
                            o.insert(k.clone(), x.clone());
                        }
                    }
                }
                v
            }
            None => defaults
                .get("movement")
                .cloned()
                .unwrap_or_else(|| json!({})),
        };

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let current = repo::program(&mut tx, &input.program_id).await?;
        let current_tiers = repo::tiers(&mut tx, Some(&input.program_id)).await?;
        let all_tiers = repo::tiers(&mut tx, None).await?;
        if current.is_none() {
            let programs: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM customer.member_program WHERE tenant_id = $1",
            )
            .bind(tx.tenant.as_str())
            .fetch_one(&mut *tx.tx)
            .await
            .map_err(db)?;
            if programs >= programs_max {
                return Err(ToolError::new(TIER_LIMIT_EXCEEDED, format!("A store has at most {programs_max} programs."))
                    .detail(json!({"limit": programs_max, "actual": programs, "policy_key": "tier.programs_max"}))
                    .next("list_tiers"));
            }
        }
        // Tier ids are unique in the store (tier.tier_id_scope).
        if let Some(t) = all_tiers
            .iter()
            .find(|x| x.member_program_id != input.program_id && tiers.iter().any(|t| t.id == x.id))
        {
            return Err(ToolError::new(
                TIER_RANK_CONFLICT,
                format!(
                    "Tier id {} is already used by program {}.",
                    t.id, t.member_program_id
                ),
            )
            .detail(json!({"field": "has_tiers.id", "rule": "tier.tier_id_scope"}))
            .next("list_tiers"));
        }
        // Removed tiers: their members need somewhere to go, and nothing may still aim at them.
        let removed: Vec<&repo::TierRow> = current_tiers
            .iter()
            .filter(|c| !tiers.iter().any(|t| t.id == c.id))
            .collect();
        let mut migrations = Vec::new();
        let mut referenced = Vec::new();
        for r in &removed {
            let ids: Vec<String> = sqlx::query_scalar(
                "SELECT customer_id FROM customer.program_membership
                 WHERE tenant_id = $1 AND member_program_id = $2 AND tier_id = $3 ORDER BY customer_id",
            )
            .bind(tx.tenant.as_str())
            .bind(&input.program_id)
            .bind(&r.id)
            .fetch_all(&mut *tx.tx)
            .await
            .map_err(db)?;
            if let Some(read) = providers::tier_references() {
                let refs = read(&mut tx, &r.id).await?;
                if refs.active_promotions + refs.active_coupons > 0 {
                    referenced.push(
                        json!({"tier_id": r.id, "active_promotions": refs.active_promotions,
                                           "active_coupons": refs.active_coupons}),
                    );
                }
            }
            let to = input
                .tier_migrations
                .as_deref()
                .unwrap_or_default()
                .iter()
                .find(|m| m.from_tier_id == r.id)
                .map(|m| m.to_tier_id.clone());
            match to {
                Some(to) if tiers.iter().any(|t| t.id == to) => {
                    migrations.push((r.id.clone(), to, ids))
                }
                _ if ids.is_empty() => {}
                _ => {
                    return Err(ToolError::new(
                        MIGRATION_REQUIRED,
                        format!("{} has {} members; say which tier they move to (tier_migrations).", r.id, ids.len()),
                    )
                    .detail(json!({"rule": "tier.program_change.removing_tier_requires", "tier_id": r.id,
                                   "members": ids.len()}))
                    .next("define_tier_program"));
                }
            }
        }
        if !referenced.is_empty() {
            return Err(ToolError::new(
                TIER_IN_USE,
                "A tier to remove is still named by running promotions or coupons.",
            )
            .detail(
                json!({"rule": "tier.program_change.block_if_referenced_by", "items": referenced}),
            )
            .next("search_promotions"));
        }
        let version = current.as_ref().map_or(1, |p| p.version + 1);
        let after = ProgramDefinition {
            name: input.name.trim().to_owned(),
            evaluation: evaluation.clone(),
            movement: movement.clone(),
            version,
            tiers: tiers.clone(),
        };
        let before = current.as_ref().map(|p| ProgramDefinition {
            name: p.name.clone(),
            evaluation: p.evaluation.clone(),
            movement: p.movement.clone(),
            version: p.version,
            tiers: current_tiers
                .iter()
                .map(|t| StoredTier {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    rank: t.rank,
                    has_tier_requirement: t.has_tier_requirement.clone(),
                    has_tier_benefit: t.has_tier_benefit.clone(),
                    points_value: t.membership_points_earned_value,
                })
                .collect(),
        });
        // The impact: today's members under the new definition.
        let defs: Vec<TierDef> = tiers
            .iter()
            .map(|t| {
                TierDef::from_stored(&t.id, t.rank, &t.has_tier_requirement, &t.has_tier_benefit)
            })
            .collect();
        let (impact, measured) = match &current {
            Some(p) => {
                let o = evaluate_members(
                    &mut tx,
                    p,
                    &defs,
                    &evaluation,
                    &movement,
                    None,
                    cx.clock.now(),
                )
                .await?;
                (o.moves, o.measured)
            }
            None => (vec![], true),
        };
        tx.commit().await.map_err(ToolError::from)?;
        let impact: Vec<(Placed, Move)> = impact
            .into_iter()
            .filter(|(_, m)| matches!(m.direction, Direction::Up | Direction::Down))
            .collect();
        let immediately = input.takes_effect == Some(TakesEffect::Immediately);

        let mut changes = Vec::new();
        for t in &tiers {
            match before
                .as_ref()
                .and_then(|b| b.tiers.iter().find(|x| x.id == t.id))
            {
                None => changes.push(Change {
                    op: ChangeOp::Create,
                    entity: "member_program_tier".into(),
                    id: t.id.clone(),
                    from: Value::Null,
                    to: json!({"name": t.name, "rank": t.rank}),
                    note: None,
                }),
                Some(old) if old != t => changes.push(Change {
                    op: ChangeOp::Update,
                    entity: "member_program_tier".into(),
                    id: t.id.clone(),
                    from: json!({"name": old.name, "rank": old.rank,
                                 "has_tier_requirement": old.has_tier_requirement,
                                 "has_tier_benefit": old.has_tier_benefit}),
                    to: json!({"name": t.name, "rank": t.rank,
                               "has_tier_requirement": t.has_tier_requirement,
                               "has_tier_benefit": t.has_tier_benefit}),
                    note: None,
                }),
                Some(_) => {}
            }
        }
        for r in &removed {
            changes.push(Change {
                op: ChangeOp::Delete,
                entity: "member_program_tier".into(),
                id: r.id.clone(),
                from: json!({"name": r.name, "rank": r.rank}),
                to: Value::Null,
                note: None,
            });
        }
        for (from, to, ids) in &migrations {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "member_tier".into(),
                id: String::new(),
                from: json!(from),
                to: json!(to),
                note: Some(format!(
                    "{} members move when the tier is removed",
                    ids.len()
                )),
            });
        }
        for (from, to, _, n) in grouped(&impact) {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "member_tier".into(),
                id: String::new(),
                from: json!(from),
                to: json!(to),
                note: Some(if immediately {
                    format!("{n} members, now")
                } else {
                    format!("{n} members at the next evaluation")
                }),
            });
        }
        let moving = impact.len() + migrations.iter().map(|m| m.2.len()).sum::<usize>();
        let coupons: Vec<String> = tiers
            .iter()
            .flat_map(|t| t.has_tier_benefit.as_array().cloned().unwrap_or_default())
            .filter_map(|b| {
                b.get("coupon_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        let ranks: Vec<i32> = tiers.iter().map(|t| t.rank).collect();
        let checks = vec![
            Check {
                rule: "tier.entry_tier".into(),
                passed: true,
                detail: Some(format!("{} is the entry tier", entry.id)),
                fix_tool: None,
            },
            Check {
                rule: "tier.rank".into(),
                passed: true,
                detail: Some(format!(
                    "Ranks {}–{}, no duplicates",
                    ranks.iter().min().unwrap_or(&1),
                    ranks.iter().max().unwrap_or(&1)
                )),
                fix_tool: None,
            },
            Check {
                rule: "tier.requirement.higher_rank_needs_more".into(),
                passed: rising_check.is_ok(),
                detail: Some(
                    rising_check
                        .clone()
                        .err()
                        .unwrap_or_else(|| "Higher ranks need more".into()),
                ),
                fix_tool: Some("define_tier_program".into()),
            },
            Check {
                rule: "tier.program_change.block_if_referenced_by".into(),
                passed: true,
                detail: Some(if removed.is_empty() {
                    "No tier is removed".into()
                } else if providers::tier_references().is_some() {
                    "No removed tier is used by promotions or coupons".into()
                } else {
                    "Removed tiers could not be checked against promotions (not connected)".into()
                }),
                fix_tool: Some("search_promotions".into()),
            },
            Check {
                rule: "tier.program_change.benefit_coupons_active".into(),
                passed: true,
                detail: Some(if coupons.is_empty() {
                    "No coupon benefits".into()
                } else {
                    format!(
                        "{} are promotion's coupons; promotion checks them when it issues them",
                        coupons.join(", ")
                    )
                }),
                fix_tool: Some("create_coupon".into()),
            },
            Check {
                rule: "tier.program_change.preview_impact".into(),
                passed: true,
                detail: Some(if measured {
                    format!(
                        "{} members would change tier {}",
                        moving,
                        if immediately {
                            "now"
                        } else {
                            "at the next evaluation"
                        }
                    )
                } else {
                    "Orders are not connected here; only tier removals move members".into()
                }),
                fix_tool: Some("preview_tier_evaluation".into()),
            },
        ];
        let facts = json!({"program_id": input.program_id});
        let what = if current.is_some() {
            format!(
                "Saves version {version} of the tier program; {moving} members would change tier"
            )
        } else {
            "Creates a tier program".into()
        };
        Ok(Planned {
            changes,
            checks,
            basis: current
                .as_ref()
                .map(|p| Basis {
                    entity: "customer.member_program".into(),
                    id: p.id.clone(),
                    version: i64::from(p.version),
                })
                .into_iter()
                .collect(),
            approval: always(cx, Self::META.tool, &facts, what),
            plan: DefinePlan {
                program_id: input.program_id,
                before,
                after,
                migrations,
                moves: if immediately { impact } else { vec![] },
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &DefinePlan) -> Result<Committed, ToolError> {
        let mut produced = Map::new();
        produced.insert("program_id".into(), json!(plan.program_id));
        produced.insert(
            "tier_id".into(),
            json!(plan.after.tiers.iter().map(|t| &t.id).collect::<Vec<_>>()),
        );
        let stored = repo::program(tx, &plan.program_id).await?;
        if stored
            .as_ref()
            .is_some_and(|p| p.version == plan.after.version)
        {
            // A repeated run: this version is saved already.
            return Ok(Committed {
                id: plan.program_id.clone(),
                produced,
            });
        }
        write_definition(tx, &plan.program_id, &plan.after, plan.before.is_none()).await?;
        for (from, to, ids) in &plan.migrations {
            migrate(
                tx,
                &plan.program_id,
                from,
                to,
                ids,
                &format!("{from} was removed"),
            )
            .await?;
        }
        let keep: Vec<String> = plan.after.tiers.iter().map(|t| t.id.clone()).collect();
        delete_tiers_not_in(tx, &plan.program_id, &keep).await?;
        apply_moves(
            tx,
            &plan.program_id,
            None,
            &plan.moves,
            TierChangeCause::ProgramChange,
        )
        .await?;
        produced.insert("version".into(), json!(plan.after.version));
        Ok(Committed {
            id: plan.program_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &DefinePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let p = repo::program(&mut tx, &plan.program_id).await?;
        let tiers = repo::tiers(&mut tx, Some(&plan.program_id)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        if p.as_ref().map(|p| p.version) != Some(plan.after.version) {
            mismatches.push(format!(
                "{} is not at version {}",
                plan.program_id, plan.after.version
            ));
        }
        if tiers.len() != plan.after.tiers.len() {
            mismatches.push(format!(
                "{} has {} tiers, not {}",
                plan.program_id,
                tiers.len(),
                plan.after.tiers.len()
            ));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &DefinePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        revert_moves(
            tx,
            &plan.program_id,
            None,
            &plan.moves,
            TierChangeCause::ProgramChange,
        )
        .await?;
        match &plan.before {
            Some(before) => {
                // The old tiers come back first, then their members, then added tiers go.
                write_definition(tx, &plan.program_id, before, false).await?;
                for (from, to, ids) in &plan.migrations {
                    migrate(tx, &plan.program_id, to, from, ids, "Program change undone").await?;
                }
                let keep: Vec<String> = before.tiers.iter().map(|t| t.id.clone()).collect();
                delete_tiers_not_in(tx, &plan.program_id, &keep).await?;
            }
            None => {
                // A new program had no members yet: its tiers go; the empty program stays.
                delete_tiers_not_in(tx, &plan.program_id, &[]).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_without_data_carrying_enums() {
        let s = closed_schema_for::<DefineTierProgramInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(DefineTierProgram::META.kind, Kind::WriteApproval);
    }

    #[test]
    fn higher_ranks_need_more() {
        let tier = |id: &str, rank: i32, spend: f64| StoredTier {
            id: id.into(),
            name: id.into(),
            rank,
            has_tier_requirement: json!({"combine": "any", "requirements": [
                {"metric": "spend", "threshold": {"value": spend, "currency": "USD"}}]}),
            has_tier_benefit: json!([]),
            points_value: None,
        };
        assert!(rising(&[tier("gold", 2, 300.0), tier("vip", 3, 800.0)]).is_ok());
        assert_eq!(
            rising(&[tier("gold", 2, 300.0), tier("vip", 3, 200.0)]).unwrap_err(),
            "vip needs no more spend than gold"
        );
    }
}
