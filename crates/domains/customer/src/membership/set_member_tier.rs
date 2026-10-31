//! `set_member_tier`: put a member in a tier by hand (goodwill, an invite, a correction), with a
//! reason and an end; until then evaluations leave it alone. `clear_override` ends a manual tier
//! early. A person always approves.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::TierChangeCause;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::tier_not_found;
use crate::access::{always, day, parse_time, policy_number, staff_only};
use crate::person::account_or_not_found;
use crate::{
    CUSTOMER_NOT_FOUND, MEMBER_NOT_ACTIVE, OVERRIDE_TOO_LONG, REASON_REQUIRED, TIER_NOT_FOUND, repo,
};

/// What happens when the manual tier ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AfterExpiry {
    /// The next evaluation decides.
    ReEvaluate,
    /// Back to the tier before.
    RevertToPrevious,
}

impl AfterExpiry {
    fn word(self) -> &'static str {
        match self {
            Self::ReEvaluate => "re_evaluate",
            Self::RevertToPrevious => "revert_to_previous",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetMemberTierInput {
    /// The member (`search_members`).
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_alex")]
    pub customer_id: String,
    /// The tier to set (`list_tiers`). Not needed with `clear_override`.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"gold")]
    pub tier_id: Option<String>,
    /// Why, in a sentence (`tier.override.reason_required`).
    #[schemars(
        length(max = 500),
        example = &"Goodwill after a defective pair of boots, inq_771"
    )]
    pub reason: String,
    /// When the manual tier ends, store time. Now + `tier.override.max_days` when left out; never
    /// later than that.
    #[schemars(example = &"2027-03-31T23:59")]
    pub expires_at: Option<String>,
    /// re_evaluate (the next evaluation decides) or revert_to_previous. The store's default
    /// (`tier.override.after_expiry`) when left out.
    #[schemars(example = &"re_evaluate")]
    pub after_expiry: Option<AfterExpiry>,
    /// Also give the tier's entry benefits (coupons). Default `tier.override.issue_entry_benefits`.
    #[schemars(example = false)]
    pub issue_entry_benefits: Option<bool>,
    /// `true` ends the member's manual tier now instead of setting one.
    #[schemars(example = false)]
    pub clear_override: Option<bool>,
}

/// A membership's tier fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TierPlace {
    pub tier_id: String,
    pub since: DateTime<Utc>,
    pub tier_override: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetTierPlan {
    pub customer_id: String,
    pub program_id: String,
    pub before: TierPlace,
    pub after: TierPlace,
    pub reason: String,
    pub expires: Option<DateTime<Utc>>,
}

pub struct SetMemberTier;

impl Operation for SetMemberTier {
    const META: OpMeta = OpMeta {
        address: "customer.membership.set_member_tier",
        tool: "set_member_tier",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/customer/people/{customer_id}/tier",
        summary: "Set a member's tier by hand with a reason and an end date (evaluations leave it until then), or clear a manual tier. A person always approves.",
        consumes: &[Slot::one("customer_id"), Slot::one("tier_id")],
        produces: &[Slot::one("customer_id")],
        checks: &["tier.override.reason_required", "tier.override.max_days"],
        errors: &[
            TIER_NOT_FOUND,
            REASON_REQUIRED,
            OVERRIDE_TOO_LONG,
            MEMBER_NOT_ACTIVE,
            CUSTOMER_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetMemberTierInput;
    type Output = Value;
}

impl WriteOp<Cx> for SetMemberTier {
    type Plan = SetTierPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetMemberTierInput,
    ) -> Result<Planned<SetTierPlan>, ToolError> {
        staff_only(cx)?;
        let reason = input.reason.trim().to_owned();
        if reason.is_empty() {
            return Err(
                ToolError::new(REASON_REQUIRED, "A manual tier needs a reason.")
                    .detail(json!({"field": "reason", "reason": "missing",
                               "rule": "tier.override.reason_required"}))
                    .next("set_member_tier"),
            );
        }
        let now = cx.clock.now();
        let max_days = policy_number(cx, "customer.tier.override.max_days")?;
        let latest = now + Duration::days(max_days);
        let expires = parse_time(cx, "expires_at", input.expires_at.as_deref())?.unwrap_or(latest);
        let clear = input.clear_override.unwrap_or(false);
        if !clear && (expires > latest || expires <= now) {
            return Err(ToolError::new(
                OVERRIDE_TOO_LONG,
                format!("A manual tier ends within {max_days} days; pick an end before {}.", day(cx.time, latest)),
            )
            .detail(json!({"field": "expires_at", "reason": "range",
                           "limit": cx.time.format(latest), "policy_key": "tier.override.max_days"}))
            .next("set_member_tier"));
        }
        let after_expiry = input.after_expiry.map_or_else(
            || {
                crate::access::policy(cx, "customer.tier.override.after_expiry")
                    .and_then(Value::as_str)
                    .unwrap_or("re_evaluate")
                    .to_owned()
            },
            |a| a.word().to_owned(),
        );
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let a = account_or_not_found(&mut tx, &input.customer_id).await?;
        if !matches!(a.member_state.as_str(), "active" | "dormant") {
            return Err(ToolError::new(
                MEMBER_NOT_ACTIVE,
                format!(
                    "{} is {}; their tier is not set by hand.",
                    a.id,
                    a.member_state.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "person", "from": a.member_state}))
            .next("get_member"));
        }
        let memberships = repo::memberships(&mut tx, &a.id).await?;
        let target = match (&input.tier_id, clear) {
            (_, true) => None,
            (Some(t), false) => Some(
                repo::tier(&mut tx, t)
                    .await?
                    .ok_or_else(|| tier_not_found(t))?,
            ),
            (None, false) => {
                return Err(
                    ToolError::invalid_input("Say which tier to set (`tier_id`).")
                        .detail(json!({"field": "tier_id", "reason": "missing"}))
                        .next("list_tiers"),
                );
            }
        };
        tx.commit().await.map_err(ToolError::from)?;
        let program_id = target
            .as_ref()
            .map(|t| t.member_program_id.clone())
            .or_else(|| memberships.first().map(|m| m.member_program_id.clone()))
            .ok_or_else(|| tier_not_found(input.tier_id.as_deref().unwrap_or("none")))?;
        let current = memberships
            .iter()
            .find(|m| m.member_program_id == program_id)
            .ok_or_else(|| {
                ToolError::new(TIER_NOT_FOUND, format!("{} is not in {program_id}.", a.id))
                    .detail(json!({"entity": "program_membership", "id": program_id}))
                    .next("list_tiers")
            })?;
        let before = TierPlace {
            tier_id: current.tier_id.clone(),
            since: current.tier_since,
            tier_override: current.tier_override.clone(),
        };
        let after = match &target {
            Some(t) => TierPlace {
                tier_id: t.id.clone(),
                since: if t.id == current.tier_id {
                    current.tier_since
                } else {
                    now
                },
                tier_override: Some(
                    json!({"reason": reason, "expires_at": cx.time.format(expires),
                    "after_expiry": after_expiry, "by": cx.principal.subject,
                    "previous_tier_id": current.tier_id,
                    "issue_entry_benefits": input.issue_entry_benefits.unwrap_or(false)}),
                ),
            },
            None => TierPlace {
                tier_override: None,
                ..before.clone()
            },
        };
        if clear && before.tier_override.is_none() {
            return Err(
                ToolError::invalid_input(format!("{} has no manual tier to clear.", a.id))
                    .detail(json!({"field": "clear_override", "reason": "unchanged"}))
                    .next("get_member_tier"),
            );
        }
        let why = if clear {
            format!("Ends {}'s manual tier; the next evaluation decides", a.id)
        } else {
            format!(
                "Sets a member's tier by hand until {}",
                day(cx.time, expires)
            )
        };
        let facts = json!({"customer_id": a.id});
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "member_tier".into(),
                id: a.id.clone(),
                from: json!(before.tier_id),
                to: json!(after.tier_id),
                note: Some(if clear {
                    "Manual tier cleared".into()
                } else {
                    format!("Manual until {}", day(cx.time, expires))
                }),
            }],
            checks: vec![
                Check {
                    rule: "tier.override.reason_required".into(),
                    passed: true,
                    detail: Some("Reason recorded".into()),
                    fix_tool: None,
                },
                Check {
                    rule: "tier.override.max_days".into(),
                    passed: true,
                    detail: Some(if clear {
                        "Nothing to limit: the manual tier ends now".to_owned()
                    } else {
                        format!(
                            "Expires {}, within the {max_days}-day limit; then {}",
                            day(cx.time, expires),
                            after_expiry.replace('_', " ")
                        )
                    }),
                    fix_tool: Some("set_member_tier".into()),
                },
            ],
            basis: vec![Basis {
                entity: "customer.person".into(),
                id: a.id.clone(),
                version: i64::from(a.version),
            }],
            approval: always(cx, Self::META.tool, &facts, why),
            plan: SetTierPlan {
                customer_id: a.id,
                program_id,
                before,
                after,
                reason,
                expires: (!clear).then_some(expires),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SetTierPlan) -> Result<Committed, ToolError> {
        let p = &plan.after;
        repo::set_membership_tier(
            tx,
            &plan.customer_id,
            &plan.program_id,
            &p.tier_id,
            p.since,
            p.tier_override.as_ref(),
        )
        .await?;
        repo::add_history(
            tx,
            &repo::TierChange {
                customer_id: plan.customer_id.clone(),
                program_id: plan.program_id.clone(),
                from_tier_id: Some(plan.before.tier_id.clone()),
                to_tier_id: Some(p.tier_id.clone()),
                cause: TierChangeCause::ManualOverride.as_str().into(),
                evaluation_id: None,
                snapshot: None,
                agent_id: p
                    .tier_override
                    .as_ref()
                    .and_then(|o| o.get("by"))
                    .and_then(Value::as_str)
                    .map_or_else(|| tx.principal.subject.clone(), str::to_owned),
                reason: Some(plan.reason.clone()),
                expires: plan.expires,
            },
        )
        .await?;
        let mut produced = Map::new();
        produced.insert("customer_id".into(), json!(plan.customer_id));
        Ok(Committed {
            id: plan.customer_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SetTierPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let m = repo::memberships(&mut tx, &plan.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match m.iter().find(|m| m.member_program_id == plan.program_id) {
            Some(m) if m.tier_id == plan.after.tier_id => vec![],
            Some(m) => vec![format!("{} is in {}", plan.customer_id, m.tier_id)],
            None => vec![format!("{} left {}", plan.customer_id, plan.program_id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SetTierPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        let b = &plan.before;
        repo::set_membership_tier(
            tx,
            &plan.customer_id,
            &plan.program_id,
            &b.tier_id,
            b.since,
            b.tier_override.as_ref(),
        )
        .await?;
        repo::add_history(
            tx,
            &repo::TierChange {
                customer_id: plan.customer_id.clone(),
                program_id: plan.program_id.clone(),
                from_tier_id: Some(plan.after.tier_id.clone()),
                to_tier_id: Some(b.tier_id.clone()),
                cause: TierChangeCause::ManualOverride.as_str().into(),
                evaluation_id: None,
                snapshot: None,
                agent_id: tx.principal.subject.clone(),
                reason: Some("Undone".into()),
                expires: None,
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn it_always_waits_for_a_person() {
        let s = closed_schema_for::<SetMemberTierInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(SetMemberTier::META.kind, Kind::WriteApproval);
        assert_eq!(
            s["properties"]["after_expiry"]["enum"],
            json!(["re_evaluate", "revert_to_previous"])
        );
    }
}
