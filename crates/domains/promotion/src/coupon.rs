//! `create_coupon`, `issue_coupon`, `search_coupons`, `get_coupon`, `list_member_coupons`
//! (docs/api/promotion.md#coupon). A coupon records its promotion's benefit on the order; it
//! never adds a second discount (`coupon.records_benefit_only`).

use std::collections::BTreeMap;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorClass,
    ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned, ReadOp, Slot, ToolError,
    UndoSupport, Verification, WriteOp,
};
use acom_core::enums::{CouponApplication, CouponStacking, CouponStatus};
use acom_core::ids::MemberCouponId;
use acom_customer::service::for_promotion::{self as customers, Member};
use acom_store::{Cx, Page, TxCx};
use chrono::{DateTime, Datelike, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{check_member_benefits, count_text, decimal};
use crate::repo::{self, CouponFilter, CouponRow};
use crate::{
    INVALID_FILTER, TIER_NOT_FOUND, audience, check_id, known_member, known_tier, not_found, pick,
    time_or_null, whose,
};

pub const CODE_TAKEN: ErrorCode = ErrorCode::new("code_taken", ErrorClass::AlreadyExists);
pub const COUPON_TIER_MISMATCH: ErrorCode =
    ErrorCode::new("coupon_tier_mismatch", ErrorClass::Domain);
pub const RECIPIENT_TIER_MISMATCH: ErrorCode =
    ErrorCode::new("recipient_tier_mismatch", ErrorClass::Domain);
pub const STACKING_CONFLICT: ErrorCode =
    ErrorCode::new("stacking_conflict", ErrorClass::PolicyViolation);
pub const POOL_EXHAUSTED: ErrorCode = ErrorCode::new("pool_exhausted", ErrorClass::LimitExceeded);

/// A coupon as the reads show it.
fn coupon_json(cx: &Cx, c: &CouponRow) -> Value {
    json!({
        "id": c.id,
        "code": c.discount_code,
        "name": c.name.clone().unwrap_or_else(|| c.promotion_name.clone()),
        "promotion_id": c.promotion_id,
        "valid_for_member_tier": audience(c.valid_for_member_tier.as_deref()),
        "application": c.application,
        "stacking": c.stacking,
        "valid_from": time_or_null(&cx.time, c.valid_from),
        "valid_through": time_or_null(&cx.time, c.valid_through),
    })
}

/// A string list from a policy key.
fn words(cx: &Cx, key: &str) -> Vec<String> {
    cx.policy
        .lookup(key)
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|w| w.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn policy_number(cx: &Cx, key: &str) -> Result<i64, ToolError> {
    cx.policy
        .lookup(key)
        .and_then(decimal)
        .and_then(|d| i64::try_from(d.trunc()).ok())
        .ok_or_else(|| {
            ToolError::new(
                ErrorCode::INTERNAL,
                format!("The policy value `{key}` is missing or not readable."),
            )
        })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCouponInput {
    /// The code, unique in the store: 4–20 capital letters or digits.
    #[schemars(regex(pattern = r"^[A-Z0-9]{4,20}$"), example = &"VIPWEEKEND20")]
    pub code: String,
    /// The promotion whose benefit the coupon records (`set_sale_prices`,
    /// `list_active_promotions`).
    #[schemars(regex(pattern = r"^promo_[a-z0-9_]+$"), example = &"promo_autumn_knit")]
    pub promotion_id: String,
    /// Tier allowed to use it: a tier id (`list_tiers`) or `everyone`. Must be the promotion's
    /// tier.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub valid_for_member_tier: String,
    /// How the coupon reaches an order. automatic_at_checkout: applies to every eligible
    /// member's checkout, nothing to issue; code_entry: the shopper types the code; download:
    /// the shopper keeps it in their wallet first.
    #[schemars(example = &"automatic_at_checkout")]
    pub application: CouponApplication,
    /// Which other coupons may be used in the same order. no_tier_coupons: not with tier
    /// coupons; with_tier_coupons: together with them; no_other_coupons: with no other coupon.
    /// Default: the policy's `coupon.stacking.default`.
    #[schemars(example = &"no_tier_coupons")]
    pub stacking: Option<CouponStacking>,
}

/// What a run of `create_coupon` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CouponRun {
    /// `cpn_` + the code in lower case.
    pub coupon_id: String,
    pub code: String,
    /// The promotion id, or a `$item[n].promotion_id` placeholder until the runner binds it.
    pub promotion_id: String,
    /// `None` for everyone.
    pub valid_for_member_tier: Option<String>,
    pub application: String,
    pub stacking: String,
    pub records_benefit_only: bool,
}

pub struct CreateCoupon;

impl Operation for CreateCoupon {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.create",
        tool: "create_coupon",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/promotion/coupons",
        summary: "Create a coupon tied to a promotion. It records the promotion's benefit on the order (one discount line in the cart), never a second discount; its period follows the promotion. Always needs a person's approval.",
        consumes: &[Slot::one("promotion_id"), Slot::one("tier_id")],
        produces: &[Slot::one("coupon_id")],
        checks: &["member_benefits_per_order"],
        errors: &[
            ErrorCode::NOT_FOUND,
            CODE_TAKEN,
            COUPON_TIER_MISMATCH,
            STACKING_CONFLICT,
            TIER_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateCouponInput;
    type Output = Value;
}

/// The approval sentence of a new coupon: how it reaches whom.
pub(crate) fn coupon_reason(
    application: CouponApplication,
    code: &str,
    tier: Option<&customers::Tier>,
) -> String {
    let who = match tier {
        Some(t) => format!("{} {} members", count_text(t.member_count), t.name),
        None => "everyone".to_owned(),
    };
    match application {
        CouponApplication::AutomaticAtCheckout => {
            format!("Applies automatically at checkout for {who}")
        }
        CouponApplication::CodeEntry => format!("Code {code} works for {who}"),
        CouponApplication::Download => format!("Can be downloaded by {who}"),
    }
}

fn tier_mismatch(coupon: Option<&str>, promotion: Option<&str>) -> ToolError {
    ToolError::new(
        COUPON_TIER_MISMATCH,
        format!(
            "The coupon is for {} but the promotion is for {}; they must match.",
            audience(coupon),
            audience(promotion)
        ),
    )
    .detail(
        json!({"rule": "coupon.tier_match", "coupon": audience(coupon),
                   "promotion": audience(promotion)}),
    )
    .next("get_promotion")
}

impl WriteOp<Cx> for CreateCoupon {
    type Plan = CouponRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateCouponInput,
    ) -> Result<Planned<CouponRun>, ToolError> {
        let code = input.code.trim().to_owned();
        let code_ok = (4..=20).contains(&code.len())
            && code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if !code_ok {
            return Err(ToolError::invalid_input(
                "`code` must be 4–20 capital letters or digits, for example VIPWEEKEND20.",
            )
            .detail(json!({"field": "code", "reason": "format"})));
        }
        let placeholder = check_id("promotion_id", "promo", &input.promotion_id, true)?;
        let application = input.application.as_str();
        if !words(cx, "promotion.coupon.applications")
            .iter()
            .any(|a| a == application)
        {
            return Err(ToolError::invalid_input(format!(
                "This store does not use `{application}` coupons."
            ))
            .detail(json!({"field": "application", "reason": "enum",
                           "allowed": words(cx, "promotion.coupon.applications")})));
        }
        let stacking = match input.stacking {
            Some(s) => s.as_str().to_owned(),
            None => cx
                .policy
                .lookup("promotion.coupon.stacking.default")
                .and_then(Value::as_str)
                .unwrap_or(CouponStacking::NoTierCoupons.as_str())
                .to_owned(),
        };
        let allowed = words(cx, "promotion.coupon.stacking.allowed");
        if !allowed.contains(&stacking) {
            return Err(ToolError::invalid_input(format!(
                "Stacking `{stacking}` is not allowed in this store."
            ))
            .detail(json!({"field": "stacking", "reason": "enum", "allowed": allowed})));
        }
        let per_order = policy_number(cx, "promotion.member_benefits_per_order")?;
        let records_benefit_only = cx
            .policy
            .lookup("promotion.coupon.records_benefit_only")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if let Some(existing) = repo::coupon_with_code(&mut tx, &code).await? {
            return Err(
                ToolError::new(CODE_TAKEN, format!("The code {code} is already used."))
                    .detail(json!({"entity": "coupon", "field": "code", "existing_id": existing}))
                    .next("search_coupons"),
            );
        }
        let tier = known_tier(&mut tx, &input.valid_for_member_tier).await?;
        let tier_id = tier.as_ref().map(|t| t.id.clone());
        let eligible = match &tier {
            Some(t) => t.member_count,
            None => customers::tiers(&mut tx)
                .await?
                .iter()
                .map(|t| t.member_count)
                .sum(),
        };
        let mut basis = Vec::new();
        let mut period = (Value::Null, Value::Null);
        if !placeholder {
            let p = repo::promotion(&mut tx, &input.promotion_id)
                .await?
                .ok_or_else(|| {
                    not_found("promotion", &input.promotion_id, "list_active_promotions")
                })?;
            if p.valid_for_member_tier != tier_id {
                return Err(tier_mismatch(
                    tier_id.as_deref(),
                    p.valid_for_member_tier.as_deref(),
                ));
            }
            if application == CouponApplication::AutomaticAtCheckout.as_str() {
                let (_, existing) = repo::coupons(
                    &mut tx,
                    &CouponFilter {
                        promotion_id: Some(p.id.clone()),
                        application: Some(application.to_owned()),
                        ..CouponFilter::default()
                    },
                    None,
                )
                .await?;
                if let Some(other) = existing.first() {
                    return Err(ToolError::new(
                        STACKING_CONFLICT,
                        format!(
                            "{} already records {} automatically; a second automatic coupon would record it twice.",
                            other.discount_code, p.name
                        ),
                    )
                    .detail(json!({"rule": "coupon.stacking", "items": [other.id]}))
                    .next("search_coupons"));
                }
            }
            period = (
                json!(cx.time.format(p.valid_from)),
                time_or_null(&cx.time, p.valid_through),
            );
            basis.push(Basis {
                entity: "promotion.promotion".into(),
                id: p.id,
                version: i64::from(p.version),
            });
        }
        tx.commit().await.map_err(ToolError::from)?;

        let reason = coupon_reason(input.application, &code, tier.as_ref());
        let check: Check = check_member_benefits(per_order, &stacking);
        let change = Change {
            op: ChangeOp::Create,
            entity: "coupon".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({
                "code": code,
                "promotion_id": input.promotion_id,
                "valid_for_member_tier": audience(tier_id.as_deref()),
                "application": application,
                "stacking": stacking,
                "valid_from": period.0,
                "valid_through": period.1,
                "eligible_members": eligible,
            }),
            note: placeholder.then(|| {
                "Valid from and through follow the promotion the earlier item creates".into()
            }),
        };
        Ok(Planned {
            plan: CouponRun {
                coupon_id: format!("cpn_{}", code.to_lowercase()),
                code,
                promotion_id: input.promotion_id,
                valid_for_member_tier: tier_id,
                application: application.to_owned(),
                stacking,
                records_benefit_only,
            },
            changes: vec![change],
            checks: vec![check],
            basis,
            approval: ApprovalNeed::Needed { reason },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &CouponRun) -> Result<Committed, ToolError> {
        // The promotion may have been a placeholder at preview; it is real now.
        let p = repo::promotion(tx, &run.promotion_id)
            .await?
            .ok_or_else(|| not_found("promotion", &run.promotion_id, "list_active_promotions"))?;
        if p.valid_for_member_tier != run.valid_for_member_tier {
            return Err(tier_mismatch(
                run.valid_for_member_tier.as_deref(),
                p.valid_for_member_tier.as_deref(),
            ));
        }
        repo::insert_coupon(
            tx,
            &repo::NewCoupon {
                id: &run.coupon_id,
                code: &run.code,
                promotion_id: &run.promotion_id,
                tier: run.valid_for_member_tier.as_deref(),
                application: &run.application,
                stacking: &run.stacking,
                records_benefit_only: run.records_benefit_only,
            },
        )
        .await?;
        Ok(Committed {
            id: run.coupon_id.clone(),
            produced: serde_json::Map::from_iter([("coupon_id".to_owned(), json!(run.coupon_id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &CouponRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let stored = repo::coupon(&mut tx, &run.coupon_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(compare_coupon(run, stored.as_ref()))
    }

    async fn undo(&self, tx: &mut TxCx, run: &CouponRun, _: &Committed) -> Result<(), ToolError> {
        // A coupon nobody holds is removed; otherwise the unused copies are revoked.
        if !repo::delete_coupon_if_unheld(tx, &run.coupon_id).await? {
            repo::revoke_unused(tx, &run.coupon_id, None).await?;
        }
        Ok(())
    }
}

/// Read-back check of a created coupon.
pub(crate) fn compare_coupon(run: &CouponRun, stored: Option<&CouponRow>) -> Verification {
    let mut mismatches = Vec::new();
    match stored {
        None => mismatches.push(format!("coupon {} is missing", run.coupon_id)),
        Some(c) => {
            let pairs = [
                ("code", c.discount_code.as_str(), run.code.as_str()),
                (
                    "promotion",
                    c.promotion_id.as_str(),
                    run.promotion_id.as_str(),
                ),
                (
                    "application",
                    c.application.as_str(),
                    run.application.as_str(),
                ),
                ("stacking", c.stacking.as_str(), run.stacking.as_str()),
                (
                    "tier",
                    audience(c.valid_for_member_tier.as_deref()),
                    audience(run.valid_for_member_tier.as_deref()),
                ),
            ];
            for (what, got, want) in pairs {
                if got != want {
                    mismatches.push(format!(
                        "{} {what} is {got} instead of {want}",
                        run.coupon_id
                    ));
                }
            }
        }
    }
    Verification {
        matched: mismatches.is_empty(),
        mismatches,
    }
}

/// How `issue_coupon` picks the members.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueMode {
    /// The members in `customer_ids`.
    Customers,
    /// The members of `segment_id`.
    Segment,
    /// The members who meet `condition` now.
    OnCondition,
}

impl IssueMode {
    fn word(self) -> &'static str {
        match self {
            IssueMode::Customers => "customers",
            IssueMode::Segment => "segment",
            IssueMode::OnCondition => "on_condition",
        }
    }
}

/// Which members `on_condition` picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueCondition {
    /// Members whose birthday is in the current month (store time).
    Birthday,
    /// Members who have not placed an order yet.
    FirstOrder,
    /// Members who entered the coupon's tier this month.
    TierEntry,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IssueCouponInput {
    /// The coupon (`code_entry` or `download`; automatic coupons need no issuing).
    #[schemars(regex(pattern = r"^cpn_[a-z0-9_]+$"), example = &"cpn_birthday")]
    pub coupon_id: String,
    /// Who gets it: `customers` needs `customer_ids`, `segment` needs `segment_id`,
    /// `on_condition` needs `condition`.
    #[schemars(example = &"on_condition")]
    pub issue_mode: IssueMode,
    /// The members (`search_members`) when `issue_mode` is `customers`; 1–1000.
    #[schemars(
        length(min = 1, max = 1000),
        inner(regex(pattern = r"^cus_[a-z0-9_]+$")),
        example = ["cus_alex"]
    )]
    pub customer_ids: Option<Vec<String>>,
    /// The segment when `issue_mode` is `segment`.
    #[schemars(regex(pattern = r"^seg_[a-z0-9_]+$"), example = &"seg_lapsed_vip")]
    pub segment_id: Option<String>,
    /// The condition when `issue_mode` is `on_condition`.
    #[schemars(example = &"birthday")]
    pub condition: Option<IssueCondition>,
}

/// What a run of `issue_coupon` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueRun {
    pub coupon_id: String,
    /// `(member coupon id, customer id)`.
    pub holders: Vec<(String, String)>,
    /// When the copies expire: the coupon's period end, if it has one.
    pub owned_through: Option<DateTime<Utc>>,
}

pub struct IssueCoupon;

impl Operation for IssueCoupon {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.issue",
        tool: "issue_coupon",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/promotion/coupons/{coupon_id}/issue",
        summary: "Give a code-entry or download coupon to members: a list, a segment, or everyone who meets a condition (birthday this month, entered the tier this month). Reaches customers, so it always needs a person's approval.",
        consumes: &[
            Slot::one("coupon_id"),
            Slot::many("customer_id"),
            Slot::one("segment_id"),
        ],
        produces: &[Slot::many("member_coupon_id")],
        checks: &[
            "coupon.per_member_limit",
            "coupon.total_pool_limit",
            "coupon.tier_match",
        ],
        errors: &[
            POOL_EXHAUSTED,
            RECIPIENT_TIER_MISMATCH,
            ErrorCode::INVALID_STATE,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = IssueCouponInput;
    type Output = Value;
}

/// Members picked by the mode.
async fn recipients(
    cx: &Cx,
    tx: &mut TxCx,
    input: &IssueCouponInput,
    tier: Option<&str>,
) -> Result<Vec<Member>, ToolError> {
    let field = |f: &str, m: &str| {
        ToolError::invalid_input(m.to_owned()).detail(json!({"field": f, "reason": "missing"}))
    };
    match input.issue_mode {
        IssueMode::Customers => {
            let ids = input
                .customer_ids
                .clone()
                .filter(|l| !l.is_empty() && l.len() <= 1000)
                .ok_or_else(|| {
                    field(
                        "customer_ids",
                        "`issue_mode: customers` needs 1–1000 `customer_ids`.",
                    )
                })?;
            for id in &ids {
                check_id("customer_ids", "cus", id, false)?;
            }
            let found = customers::members(tx, &ids).await?;
            if let Some(missing) = ids
                .iter()
                .find(|id| !found.iter().any(|m| m.customer_id == **id))
            {
                return Err(not_found("customer", missing, "search_members"));
            }
            Ok(found)
        }
        IssueMode::Segment => {
            let id = input
                .segment_id
                .clone()
                .ok_or_else(|| field("segment_id", "`issue_mode: segment` needs `segment_id`."))?;
            check_id("segment_id", "seg", &id, false)?;
            // Segments are not stored yet, so no segment can be found.
            Err(not_found("segment", &id, "preview_segment"))
        }
        IssueMode::OnCondition => {
            let condition = input.condition.ok_or_else(|| {
                field("condition", "`issue_mode: on_condition` needs `condition`.")
            })?;
            let local = cx.clock.now().with_timezone(&cx.time.zone);
            match condition {
                IssueCondition::Birthday => {
                    customers::members_with_birthday_in(tx, tier, local.month()).await
                }
                IssueCondition::TierEntry => {
                    let start = local.date_naive().with_day(1).unwrap_or(local.date_naive());
                    let since = cx
                        .time
                        .parse(&format!("{start}T00:00"))
                        .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
                    customers::members_entered_tier_since(tx, tier, since).await
                }
                IssueCondition::FirstOrder => Err(ToolError::invalid_input(
                    "`first_order` needs order history, which coupons cannot read yet; pick the members with `customers`.",
                )
                .detail(json!({"field": "condition", "reason": "enum",
                               "allowed": ["birthday", "tier_entry"]}))),
            }
        }
    }
}

impl WriteOp<Cx> for IssueCoupon {
    type Plan = IssueRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: IssueCouponInput,
    ) -> Result<Planned<IssueRun>, ToolError> {
        let placeholder = check_id("coupon_id", "cpn", &input.coupon_id, true)?;
        if !words(cx, "promotion.coupon.issue_modes")
            .iter()
            .any(|m| m == input.issue_mode.word())
        {
            return Err(ToolError::invalid_input(format!(
                "This store does not issue coupons by `{}`.",
                input.issue_mode.word()
            ))
            .detail(json!({"field": "issue_mode", "reason": "enum"})));
        }
        let per_member = policy_number(cx, "promotion.coupon.per_member_limit")?;
        let pool = policy_number(cx, "promotion.coupon.total_pool_limit")?;

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let coupon = if placeholder {
            None
        } else {
            let c = repo::coupon(&mut tx, &input.coupon_id)
                .await?
                .ok_or_else(|| not_found("coupon", &input.coupon_id, "search_coupons"))?;
            if c.application == CouponApplication::AutomaticAtCheckout.as_str() {
                return Err(ToolError::new(
                    ErrorCode::INVALID_STATE,
                    format!(
                        "{} applies automatically at checkout; there is nothing to issue.",
                        c.discount_code
                    ),
                )
                .detail(json!({"entity": "coupon", "from": c.application}))
                .next("get_coupon"));
            }
            Some(c)
        };
        let tier = coupon
            .as_ref()
            .and_then(|c| c.valid_for_member_tier.clone());
        let picked = recipients(cx, &mut tx, &input, tier.as_deref()).await?;
        if let Some(t) = &tier {
            let wrong: Vec<&String> = picked
                .iter()
                .filter(|m| m.tier_id.as_deref() != Some(t.as_str()))
                .map(|m| &m.customer_id)
                .collect();
            if !wrong.is_empty() {
                return Err(ToolError::new(
                    RECIPIENT_TIER_MISMATCH,
                    format!(
                        "{} of the members are not in the {t} tier the coupon is for.",
                        wrong.len()
                    ),
                )
                .detail(json!({"rule": "coupon.tier_match", "items": wrong}))
                .next("preview_segment"));
            }
        }
        let ids: Vec<String> = picked.iter().map(|m| m.customer_id.clone()).collect();
        let held: BTreeMap<String, (i64, i64)> = match &coupon {
            Some(c) => repo::holdings(&mut tx, &c.id, &ids)
                .await?
                .into_iter()
                .map(|(id, unused, all)| (id, (unused, all)))
                .collect(),
            None => BTreeMap::new(),
        };
        let issued = match &coupon {
            Some(c) => repo::issued_count(&mut tx, &c.id).await?,
            None => 0,
        };
        tx.commit().await.map_err(ToolError::from)?;

        let mut changes = Vec::new();
        let mut holders = Vec::new();
        let mut skipped = 0usize;
        for m in &picked {
            let (unused, all) = held.get(&m.customer_id).copied().unwrap_or((0, 0));
            let why = if m.member_state != "active" {
                Some(format!(
                    "{} is {}",
                    m.name,
                    m.member_state.replace('_', " ")
                ))
            } else if unused > 0 {
                Some(format!("{} already holds it", m.name))
            } else if per_member > 0 && all >= per_member {
                Some(format!("{} reached the limit of {per_member}", m.name))
            } else {
                None
            };
            match why {
                Some(note) => {
                    skipped += 1;
                    changes.push(Change {
                        op: ChangeOp::Exclude,
                        entity: "member_coupon".into(),
                        id: m.customer_id.clone(),
                        from: Value::Null,
                        to: json!({"why": "coupon.per_member_limit", "customer_id": m.customer_id}),
                        note: Some(note),
                    });
                }
                None => holders.push((
                    MemberCouponId::generate().to_string(),
                    m.customer_id.clone(),
                )),
            }
        }
        if pool > 0 && issued + holders.len() as i64 > pool {
            return Err(ToolError::new(
                POOL_EXHAUSTED,
                format!(
                    "Only {} of the {pool} copies are left; this would give out {}.",
                    (pool - issued).max(0),
                    holders.len()
                ),
            )
            .detail(
                json!({"limit": pool, "actual": issued + holders.len() as i64,
                           "policy_key": "coupon.total_pool_limit"}),
            ));
        }
        let n = holders.len() as i64;
        changes.insert(
            0,
            Change {
                op: ChangeOp::Create,
                entity: "member_coupon".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"coupon_id": input.coupon_id, "customers": n}),
                note: None,
            },
        );
        let checks = vec![
            Check {
                rule: "coupon.per_member_limit".into(),
                passed: true,
                detail: Some(if skipped == 0 {
                    "Nobody gets a second one".into()
                } else {
                    format!("{skipped} already have one or cannot get one and are skipped")
                }),
                fix_tool: None,
            },
            Check {
                rule: "coupon.total_pool_limit".into(),
                passed: true,
                detail: Some(if pool == 0 {
                    "No pool limit".into()
                } else {
                    format!("{} of {pool} left after this", pool - issued - n)
                }),
                fix_tool: None,
            },
            Check {
                rule: "coupon.tier_match".into(),
                passed: true,
                detail: Some(match &tier {
                    Some(t) => format!("All {} are in the {t} tier", count_text(n)),
                    None if placeholder => "The coupon's tier is checked when it runs".into(),
                    None => "The coupon is for everyone".into(),
                }),
                fix_tool: None,
            },
        ];
        let basis = coupon
            .as_ref()
            .map(|c| Basis {
                entity: "promotion.coupon".into(),
                id: c.id.clone(),
                version: i64::from(c.version),
            })
            .into_iter()
            .collect();
        Ok(Planned {
            plan: IssueRun {
                coupon_id: input.coupon_id,
                holders,
                owned_through: coupon.and_then(|c| c.valid_through),
            },
            changes,
            checks,
            basis,
            approval: ApprovalNeed::Needed {
                reason: format!("Reaches {} customers", count_text(n)),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &IssueRun) -> Result<Committed, ToolError> {
        let now = tx.clock.now();
        repo::insert_member_coupons(tx, &run.coupon_id, &run.holders, now, run.owned_through)
            .await?;
        let ids: Vec<&String> = run.holders.iter().map(|(id, _)| id).collect();
        Ok(Committed {
            id: run.coupon_id.clone(),
            produced: serde_json::Map::from_iter([("member_coupon_id".to_owned(), json!(ids))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &IssueRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let ids: Vec<String> = run.holders.iter().map(|(id, _)| id.clone()).collect();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let states = repo::member_coupon_states(&mut tx, &ids).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let found: BTreeMap<String, String> = states.into_iter().collect();
        let mut mismatches = Vec::new();
        let missing = ids.iter().filter(|id| !found.contains_key(*id)).count();
        if missing > 0 {
            mismatches.push(format!(
                "{missing} of {} member coupons are missing",
                ids.len()
            ));
        }
        let revoked = found.values().filter(|s| *s == "revoked").count();
        if revoked > 0 {
            mismatches.push(format!("{revoked} member coupons are revoked"));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, tx: &mut TxCx, run: &IssueRun, _: &Committed) -> Result<(), ToolError> {
        let ids: Vec<String> = run.holders.iter().map(|(id, _)| id.clone()).collect();
        repo::revoke_unused(tx, &run.coupon_id, Some(&ids)).await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchCouponsInput {
    /// The exact code (any letter case).
    #[schemars(length(min = 4, max = 20), example = &"VIPWEEKEND20")]
    pub code: Option<String>,
    /// Only the coupons of this promotion.
    #[schemars(regex(pattern = r"^promo_[a-z0-9_]+$"), example = &"promo_birthday")]
    pub promotion_id: Option<String>,
    /// Only coupons that reach an order this way. automatic_at_checkout, code_entry or download.
    #[schemars(example = &"download")]
    pub application: Option<CouponApplication>,
    /// Only coupons for this tier id, or `everyone` for those without a tier.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"gold")]
    pub valid_for_member_tier: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchCoupons;

impl Operation for SearchCoupons {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.search",
        tool: "search_coupons",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/coupons",
        summary: "Coupon definitions by code, promotion, application or tier, with the period each follows.",
        consumes: &[],
        produces: &[Slot::many("coupon_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchCouponsInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchCoupons {
    async fn read(&self, cx: &Cx, input: SearchCouponsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        if let Some(p) = &input.promotion_id {
            check_id("promotion_id", "promo", p, false)
                .map_err(|e| ToolError::new(INVALID_FILTER, e.message).next("list_enums"))?;
        }
        let filter = CouponFilter {
            code: input.code.map(|c| c.trim().to_uppercase()),
            promotion_id: input.promotion_id,
            application: input.application.map(|a| a.as_str().to_owned()),
            tier: input.valid_for_member_tier,
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (total, rows) = repo::coupons(&mut tx, &filter, Some(page)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|c| coupon_json(cx, c)).collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCouponInput {
    /// The coupon.
    #[schemars(regex(pattern = r"^cpn_[a-z0-9_]+$"), example = &"cpn_birthday")]
    pub coupon_id: String,
    /// Only these fields of the coupon (`id` always comes back).
    #[schemars(example = ["code", "counts"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetCoupon;

impl Operation for GetCoupon {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.get",
        tool: "get_coupon",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/coupons/{coupon_id}",
        summary: "One coupon definition with how many member copies are in each status (issued, available, used, restored, expired, revoked).",
        consumes: &[Slot::one("coupon_id")],
        produces: &[Slot::one("promotion_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCouponInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCoupon {
    async fn read(&self, cx: &Cx, input: GetCouponInput) -> Result<Value, ToolError> {
        check_id("coupon_id", "cpn", &input.coupon_id, false)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::coupon(&mut tx, &input.coupon_id)
            .await?
            .ok_or_else(|| not_found("coupon", &input.coupon_id, "search_coupons"))?;
        let counted: BTreeMap<String, i64> = repo::coupon_counts(&mut tx, &c.id)
            .await?
            .into_iter()
            .collect();
        tx.commit().await.map_err(ToolError::from)?;
        let counts: serde_json::Map<String, Value> = CouponStatus::ALL
            .iter()
            .map(|s| {
                (
                    s.as_str().to_owned(),
                    json!(counted.get(s.as_str()).copied().unwrap_or(0)),
                )
            })
            .collect();
        let mut out = coupon_json(cx, &c);
        out["records_benefit_only"] = json!(c.is_benefit_record_only);
        out["counts"] = Value::Object(counts);
        Ok(pick(out, input.fields.as_deref()))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListMemberCouponsInput {
    /// The member, or `me` for the signed-in shopper.
    #[schemars(regex(pattern = r"^(cus_[a-z0-9_]+|me)$"), example = &"me")]
    pub customer_id: String,
    /// Only these statuses (unused copies past their end read as expired).
    #[schemars(example = ["available"])]
    pub status: Option<Vec<CouponStatus>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListMemberCoupons;

impl Operation for ListMemberCoupons {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.list_member_coupons",
        tool: "list_member_coupons",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/customers/{customer_id}/coupons",
        summary: "The coupons one member holds and their status. A shopper uses customer_id `me` for their own.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::many("member_coupon_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListMemberCouponsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListMemberCoupons {
    async fn read(&self, cx: &Cx, input: ListMemberCouponsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let customer_id = whose(cx, &input.customer_id)?;
        let statuses: Vec<String> = input
            .status
            .unwrap_or_default()
            .iter()
            .map(|s| s.as_str().to_owned())
            .collect();
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        known_member(&mut tx, &customer_id).await?;
        let (total, rows) =
            repo::member_coupons(&mut tx, &customer_id, now, &statuses, page).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|m| {
                json!({"id": m.id, "coupon_id": m.coupon_id, "code": m.code, "name": m.name,
                       "status": m.status, "order_id": m.order_id,
                       "valid_from": cx.time.format(m.owned_from),
                       "valid_through": time_or_null(&cx.time, m.owned_through),
                       "used_at": time_or_null(&cx.time, m.used_at)})
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
