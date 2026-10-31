//! `set_seller_grade`: confirms the computed grade for a period, or adjusts it. The grade is
//! computed from the calendar month before the period starts; a different grade needs a reason
//! (`grade.override_requires_reason`). The preview shows what the grade changes in commission
//! and exposure (`grade.benefits`).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::{GradeLevel, SellerStatus};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{basis_wire, rates_wire, review_period};
use crate::repo::{self, NewGrade};
use crate::rules::Rules;
use crate::{
    INVALID_LEVEL, OVERRIDE_NOT_ALLOWED, SELLER_NOT_FOUND, activity, model, parse_time,
    seller_not_found, store, viewer,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetSellerGradeInput {
    /// The seller, from `search_sellers`.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_fieldstone")]
    pub seller_id: String,
    /// The grade: platinum, gold, silver, bronze, or watch (below every level).
    #[serde(rename = "acom:grade_level", alias = "grade_level")]
    #[schemars(example = &"gold")]
    pub grade_level: GradeLevel,
    /// Why the grade differs from the computed one; needed when it does.
    #[serde(rename = "acom:adjustment_reason", alias = "adjustment_reason")]
    #[schemars(length(min = 1, max = 500), example = &"Shipping delays came from a carrier strike")]
    pub adjustment_reason: Option<String>,
    /// First moment the grade applies (store time).
    #[schemars(example = &"2026-12-01T00:00")]
    pub valid_from: String,
    /// Last moment the grade applies (store time).
    #[schemars(example = &"2026-12-31T23:59")]
    pub valid_through: String,
}

/// The grade to confirm, as previewed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradePlan {
    pub seller_id: String,
    pub grade: NewGrade,
    /// A different grade without the reason the policy wants (the check failed).
    pub needs_reason: bool,
}

pub struct SetSellerGrade;

impl Operation for SetSellerGrade {
    const META: OpMeta = OpMeta {
        address: "partner.grade.set_seller_grade",
        tool: "set_seller_grade",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/partner/sellers/{seller_id}/grade",
        summary: "Confirm a seller's computed grade for a period, or set another grade with a reason. The preview shows the computed grade and what the grade changes in commission and exposure.",
        consumes: &[Slot::one("seller_id")],
        produces: &[Slot::one("seller_id")],
        checks: &[
            "grade.computed_metrics",
            "grade.override_requires_reason",
            "grade.benefits",
        ],
        errors: &[
            OVERRIDE_NOT_ALLOWED,
            INVALID_LEVEL,
            SELLER_NOT_FOUND,
            ErrorCode::INVALID_STATE,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetSellerGradeInput;
    type Output = Value;
}

/// `grade.override_requires_reason`: whether a grade other than the computed one has a reason.
pub(crate) fn reason_check(
    rules: &Rules,
    level: GradeLevel,
    computed: Option<GradeLevel>,
    reason: Option<&str>,
) -> Check {
    let differs = computed != Some(level);
    let base = computed.map_or("no computed grade", GradeLevel::as_str);
    let (passed, detail) = match (differs, reason) {
        (false, _) => (true, format!("{} is the computed grade", level.as_str())),
        (true, Some(_)) => (
            true,
            format!("Reason given for moving {base} → {}", level.as_str()),
        ),
        (true, None) if !rules.override_requires_reason => (
            true,
            format!(
                "{base} → {} without a reason; the store allows it",
                level.as_str()
            ),
        ),
        (true, None) => (
            false,
            format!(
                "Moving {base} → {} needs a reason (acom:adjustment_reason)",
                level.as_str()
            ),
        ),
    };
    Check {
        rule: "grade.override_requires_reason".into(),
        passed,
        detail: Some(detail),
        fix_tool: (!passed).then(|| "set_seller_grade".into()),
    }
}

impl WriteOp<Cx> for SetSellerGrade {
    type Plan = GradePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetSellerGradeInput,
    ) -> Result<Planned<GradePlan>, ToolError> {
        viewer::staff(cx)?;
        let from = parse_time(cx, "valid_from", &input.valid_from)?;
        let through = parse_time(cx, "valid_through", &input.valid_through)?;
        if through <= from {
            return Err(
                ToolError::invalid_input("`valid_through` must come after `valid_from`.")
                    .detail(json!({"field": "valid_through", "reason": "range"})),
            );
        }
        let rules = Rules::load(&cx.policy)?;
        let level = input.grade_level;
        if level != GradeLevel::Watch && !rules.levels.iter().any(|(l, _)| *l == level) {
            return Err(ToolError::new(
                INVALID_LEVEL,
                format!(
                    "The store does not grade with {}; it uses {} and watch.",
                    level.as_str(),
                    rules
                        .levels
                        .iter()
                        .map(|(l, _)| l.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .detail(json!({"field": "acom:grade_level", "reason": "enum", "rule": "grade.levels"}))
            .next("set_seller_grade"));
        }
        let reason = input
            .adjustment_reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned);

        let mut tx = cx.read().await.map_err(store)?;
        viewer::viewer(cx, &mut tx)
            .await?
            .store_only("set a seller's grade")?;
        let s = repo::seller(&mut tx, &input.seller_id)
            .await?
            .ok_or_else(|| seller_not_found(&input.seller_id))?;
        if s.seller_status == SellerStatus::Terminated.as_str() {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("{} is terminated; it is no longer graded.", s.legal_name),
            )
            .detail(json!({"entity": "seller", "from": s.seller_status}))
            .next("get_seller"));
        }
        let (review_from, review_through) = review_period(model::store_date(&cx.time, from));
        let products = activity::products(&mut tx, &s.id).await?;
        let m = activity::metrics(&mut tx, &rules, &products, review_from, review_through).await?;
        let before = repo::grade_at(&mut tx, &s.id, from).await?;
        tx.commit().await.map_err(store)?;

        let computed = (m.lines > 0).then(|| model::grade_for(&m, &rules));
        let reason_check = reason_check(&rules, level, computed, reason.as_deref());
        let needs_reason = !reason_check.passed;
        let from_level = before.as_ref().map(|g| g.grade_level.clone());
        let benefits_from = rules.benefits_of(from_level.as_deref());
        let benefits_to = rules.benefits_of(Some(level.as_str()));
        let benefit_note = if benefits_from == benefits_to {
            if benefits_to.as_object().is_none_or(Map::is_empty) {
                "No grade benefits are set in the store's policy, so commission and exposure stay the same".to_owned()
            } else {
                "Commission and exposure stay the same".to_owned()
            }
        } else {
            format!("Benefits change from {benefits_from} to {benefits_to}")
        };
        let mut metrics = match rates_wire(&m, &rules) {
            Value::Object(o) => Value::Object(
                o.into_iter()
                    .map(|(k, v)| (k.trim_start_matches("acom:").to_owned(), v))
                    .collect(),
            ),
            other => other,
        };
        metrics["review_period"] = json!({"valid_from": review_from.to_string(),
                                          "valid_through": review_through.to_string()});
        let grade = NewGrade {
            grade_level: level.as_str().to_owned(),
            computed_grade_level: computed.map(|c| c.as_str().to_owned()),
            adjustment_reason: reason.clone(),
            metrics,
            valid_from: from,
            valid_through: through,
        };
        let checks = vec![
            Check {
                rule: "grade.computed_metrics".into(),
                passed: true,
                detail: Some(match computed {
                    Some(c) => format!(
                        "{} computes {} from {review_from} to {review_through} ({} orders)",
                        rules.computed_metrics.join(", ").replace('_', " "),
                        c.as_str(),
                        m.order_count
                    ),
                    None => format!(
                        "No paid orders from {review_from} to {review_through}, so no grade is computed"
                    ),
                }),
                fix_tool: None,
            },
            reason_check,
            Check {
                rule: "grade.benefits".into(),
                passed: true,
                detail: Some(benefit_note),
                fix_tool: None,
            },
        ];
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "seller_grade".into(),
                id: s.id.clone(),
                from: json!({"acom:grade_level": from_level, "acom:benefits": benefits_from}),
                to: json!({"acom:grade_level": level, "acom:computed_grade_level": computed,
                           "valid_from": cx.time.format(from),
                           "valid_through": cx.time.format(through),
                           "acom:benefits": benefits_to,
                           "acom:basis": basis_wire(&m)}),
                note: reason.clone().map(|r| format!("Reason: {r}")),
            }],
            checks,
            basis: vec![Basis {
                entity: "partner.seller".into(),
                id: s.id.clone(),
                version: i64::from(s.version),
            }],
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Sets a seller's grade".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
            plan: GradePlan {
                seller_id: s.id,
                grade,
                needs_reason,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &GradePlan) -> Result<Committed, ToolError> {
        if plan.needs_reason {
            return Err(ToolError::new(
                OVERRIDE_NOT_ALLOWED,
                format!(
                    "{} differs from the computed grade {}; give acom:adjustment_reason or confirm the computed grade.",
                    plan.grade.grade_level,
                    plan.grade.computed_grade_level.as_deref().unwrap_or("(none)")
                ),
            )
            .detail(json!({"rule": "grade.override_requires_reason"}))
            .next("get_seller_grade"));
        }
        repo::insert_grade(tx, &plan.seller_id, &plan.grade).await?;
        Ok(Committed {
            id: plan.seller_id.clone(),
            produced: Map::from_iter([("seller_id".to_owned(), json!(plan.seller_id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &GradePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(store)?;
        let found = repo::find_grade(&mut tx, &plan.seller_id, &plan.grade).await?;
        tx.commit().await.map_err(store)?;
        Ok(match found {
            Some(_) => Verification {
                matched: true,
                mismatches: Vec::new(),
            },
            None => Verification {
                matched: false,
                mismatches: vec![format!(
                    "no {} grade for {} in that period",
                    plan.grade.grade_level, plan.seller_id
                )],
            },
        })
    }

    async fn undo(&self, tx: &mut TxCx, plan: &GradePlan, _: &Committed) -> Result<(), ToolError> {
        if let Some(position) = repo::find_grade(tx, &plan.seller_id, &plan.grade).await? {
            repo::delete_grade(tx, &plan.seller_id, position).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::rules;
    use crate::tests::{input, offline};

    #[test]
    fn a_different_grade_needs_a_reason() {
        let r = rules();
        let c = reason_check(&r, GradeLevel::Platinum, Some(GradeLevel::Gold), None);
        assert!(!c.passed);
        assert_eq!(c.fix_tool.as_deref(), Some("set_seller_grade"));
        let c = reason_check(
            &r,
            GradeLevel::Platinum,
            Some(GradeLevel::Gold),
            Some("Carrier strike"),
        );
        assert!(c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("Reason given for moving gold → platinum")
        );
        assert!(reason_check(&r, GradeLevel::Gold, Some(GradeLevel::Gold), None).passed);
    }

    #[tokio::test]
    async fn a_level_the_policy_left_out_is_invalid() {
        let mut cx = offline("adm_sj");
        let mut p = (*cx.policy).clone();
        p = p
            .with_overrides([("partner", "grade: {levels: {platinum: null}}")])
            .unwrap();
        cx.policy = std::sync::Arc::new(p);
        let e = SetSellerGrade
            .preview(
                &cx,
                input(
                    json!({"seller_id": "slr_fieldstone", "acom:grade_level": "platinum",
                             "valid_from": "2026-12-01", "valid_through": "2026-12-31T23:59"}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_level");
    }
}
