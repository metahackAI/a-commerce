//! `issue_coupon` (docs/api/promotion.md#coupon).

use std::collections::BTreeMap;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::CouponApplication;
use acom_core::ids::MemberCouponId;
use acom_customer::service::for_promotion::{self as customers, Member};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Datelike, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{POOL_EXHAUSTED, RECIPIENT_TIER_MISMATCH, policy_number, words};
use crate::model::count_text;
use crate::repo;
use crate::{check_id, not_found};

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
