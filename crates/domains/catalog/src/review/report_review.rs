//! `report_review` (docs/api/catalog.md#review): a signed-in shopper reports a review as
//! inappropriate. The report puts a visible review in the moderation queue (`reported`,
//! `review.report_holds_review`); it stays on the page until `moderate_review` hides it.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::ReportReason;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{NewReport, review_not_found, shopper_id};
use crate::rules::ReportRules;
use crate::{ALREADY_REPORTED, REVIEW_NOT_FOUND};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportReviewInput {
    /// The review to report (`list_reviews`).
    #[schemars(regex(pattern = r"^rev_[a-z0-9_]+$"), example = &"rev_8790")]
    pub review_id: String,
    /// Why: spam; abuse (insults or harassment); obscene; advertisement (promotes something);
    /// fake (not a real buyer's review); irrelevant (not about this product); other.
    #[schemars(example = &"advertisement")]
    pub reason: ReportReason,
    /// The shopper's own words, for the moderator.
    #[schemars(length(max = 500), example = &"Links to another shop.")]
    pub note: Option<String>,
}

/// What `report_review` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportPlan {
    pub report_id: String,
    pub review_id: String,
    pub customer_id: String,
    pub reason: String,
    pub note: Option<String>,
    pub at: DateTime<Utc>,
    /// `(from, to)` moderation state when the report puts the review in the queue.
    pub status_move: Option<(String, String)>,
}

/// Report a review for moderation.
#[derive(Debug)]
pub struct ReportReview;

impl Operation for ReportReview {
    const META: OpMeta = OpMeta {
        address: "catalog.review.report",
        tool: "report_review",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/reviews/{review_id}/report",
        summary: "The signed-in shopper reports a review as spam, abusive, obscene, an advertisement, fake or off-topic; it goes to the moderation queue (moderate_review). One report per shopper and review.",
        consumes: &[Slot::one("review_id")],
        produces: &[Slot::one("review_id")],
        checks: &["review.report_reasons"],
        errors: &[REVIEW_NOT_FOUND, ALREADY_REPORTED],
        audience: Audience::Shopper,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = ReportReviewInput;
    type Output = Value;
}

fn already_reported(review_id: &str, existing: &str) -> ToolError {
    ToolError::new(
        ALREADY_REPORTED,
        format!("You already reported {review_id} ({existing}); the store will look at it."),
    )
    .detail(json!({"entity": "review_report", "existing_id": existing}))
    .next("list_reviews")
}

/// The `review.report_reasons` check: the store takes reports for this reason.
pub(crate) fn reason_check(rules: &ReportRules, reason: &str) -> Check {
    let ok = rules.reasons.iter().any(|r| r == reason);
    Check {
        rule: "review.report_reasons".into(),
        passed: ok,
        detail: Some(if ok {
            format!("The store takes reports for {}", reason.replace('_', " "))
        } else {
            format!(
                "The store takes reports for {} only",
                rules.reasons.join(", ")
            )
        }),
        fix_tool: (!ok).then(|| "report_review".into()),
    }
}

impl WriteOp<Cx> for ReportReview {
    type Plan = ReportPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ReportReviewInput,
    ) -> Result<Planned<ReportPlan>, ToolError> {
        let customer = shopper_id(cx, "report a review")?;
        let rules = ReportRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = super::owned_review(&mut tx, &input.review_id)
            .await?
            .filter(|r| r.creative_work_status != "hidden");
        let existing = match &r {
            Some(r) => super::report_by(&mut tx, &r.id, &customer).await?,
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;
        let r = r.ok_or_else(|| review_not_found(&input.review_id))?;
        if let Some(id) = existing {
            return Err(already_reported(&r.id, &id));
        }
        let reason = input.reason.as_str().to_owned();
        let note = input
            .note
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty());
        let status_move = (rules.holds_review && r.creative_work_status == "visible")
            .then(|| ("visible".to_owned(), "reported".to_owned()));
        let mut changes = vec![Change {
            op: ChangeOp::Create,
            entity: "report".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"review_id": r.id, "reason": reason, "note": note}),
            note: None,
        }];
        if let Some((from, to)) = &status_move {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "review".into(),
                id: r.id.clone(),
                from: json!({"status": from}),
                to: json!({"status": to}),
                note: Some("Waits for the store's moderation; shoppers still see it".into()),
            });
        }
        Ok(Planned {
            changes,
            checks: vec![reason_check(&rules, &reason)],
            basis: vec![Basis {
                entity: "catalog.review".into(),
                id: r.id.clone(),
                version: i64::from(r.version),
            }],
            approval: ApprovalNeed::NotNeeded,
            plan: ReportPlan {
                report_id: format!("rpt_{}", acom_core::id::new_suffix()),
                review_id: r.id,
                customer_id: customer,
                reason,
                note,
                at: cx.clock.now(),
                status_move,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ReportPlan) -> Result<Committed, ToolError> {
        let written = super::insert_report(
            tx,
            &NewReport {
                id: &plan.report_id,
                review_id: &plan.review_id,
                customer_id: &plan.customer_id,
                reason: &plan.reason,
                note: plan.note.as_deref(),
                at: plan.at,
            },
        )
        .await?;
        if !written
            && let Some(id) = super::report_by(tx, &plan.review_id, &plan.customer_id).await?
            && id != plan.report_id
        {
            return Err(already_reported(&plan.review_id, &id));
        }
        if let Some((from, to)) = &plan.status_move {
            // A review moderated in between keeps the moderator's decision.
            super::move_status(tx, &plan.review_id, from, to).await?;
        }
        let mut produced = Map::new();
        produced.insert("review_id".into(), json!(plan.review_id));
        produced.insert("report_id".into(), json!(plan.report_id));
        Ok(Committed {
            id: plan.review_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ReportPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let found = super::report_by(&mut tx, &plan.review_id, &plan.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = found.as_deref() == Some(plan.report_id.as_str());
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("the report on {} was not recorded", plan.review_id)]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &ReportPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "The report on {} is with the store's moderators.",
                plan.review_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn reasons_are_words_the_store_takes() {
        let rules =
            ReportRules::from_policy(&crate::testing::policies()).unwrap_or_else(|e| panic!("{e}"));
        assert!(reason_check(&rules, "advertisement").passed);
        let narrow = ReportRules {
            reasons: vec!["spam".into()],
            holds_review: true,
        };
        let c = reason_check(&narrow, "fake");
        assert!(!c.passed);
        assert_eq!(c.fix_tool.as_deref(), Some("report_review"));
        let schema = acom_contract::schema::closed_schema_for::<ReportReviewInput>();
        assert!(acom_contract::schema::violations(&schema).is_empty());
        assert_eq!(schema["properties"]["reason"]["enum"][3], "advertisement");
    }

    #[tokio::test]
    async fn guests_must_sign_in_to_report() {
        let e = ReportReview
            .preview(
                &testing::guest(),
                ReportReviewInput {
                    review_id: "rev_8790".into(),
                    reason: ReportReason::Spam,
                    note: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_reports_a_review_into_the_queue_and_only_once() {
        let cx = testing::shopper_db().await;
        let planned = ReportReview
            .preview(
                &cx,
                ReportReviewInput {
                    review_id: "rev_8790".into(),
                    reason: ReportReason::Advertisement,
                    note: Some("Links to another shop.".into()),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes[1].to["status"], "reported");
        assert!(planned.checks[0].passed);
        // Run it inside a transaction that is rolled back, so the store is left as it was.
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        ReportReview
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let again = super::super::report_by(&mut tx, "rev_8790", "cus_jamie")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again.as_deref(), Some(planned.plan.report_id.as_str()));
        let mut second = planned.plan.clone();
        second.report_id = "rpt_second".into();
        let e = ReportReview.commit(&mut tx, &second).await.unwrap_err();
        assert_eq!(e.code, "already_reported");
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
    }
}
