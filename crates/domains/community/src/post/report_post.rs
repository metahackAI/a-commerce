//! `report_post` (docs/api/community.md#post): a signed-in shopper reports a post. When the
//! number of different reporters reaches `moderation.report_threshold`, the post goes to the
//! moderation queue or is blinded at once (`moderation.blind.on_threshold`); a board without a
//! queue (`none`) only records reports. One report per shopper and post: reporting again gives the
//! first report back.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::ReportReason;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::model::{self, PostAccess, post_access};
use crate::repo::{self, NewReport};
use crate::rules::ReportRules;
use crate::{POST_NOT_FOUND, Viewer, policy_approval, post_not_found, trimmed};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReportPostInput {
    /// The post to report (`list_posts`).
    #[schemars(regex(pattern = r"^post_[a-z0-9_]+$"), example = &"post_8811")]
    pub post_id: String,
    /// Why: spam; abuse (insults or harassment); obscene; advertisement (promotes something);
    /// fake (misleading or made up); irrelevant (off the board's topic); other.
    #[serde(rename = "acom:report_reason", alias = "report_reason")]
    #[schemars(example = &"advertisement")]
    pub report_reason: ReportReason,
    /// The shopper's own words, for the moderator.
    #[schemars(length(max = 500), example = &"Links to another shop.")]
    pub text: Option<String>,
}

/// What `report_post` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportPostPlan {
    pub report_id: String,
    pub post_id: String,
    pub customer_id: String,
    pub reason: String,
    pub text: Option<String>,
    pub at: DateTime<Utc>,
    /// The shopper's earlier report on this post: running returns it.
    pub existing: Option<String>,
}

/// Report a post for moderation.
#[derive(Debug)]
pub struct ReportPost;

impl Operation for ReportPost {
    const META: OpMeta = OpMeta {
        address: "community.post.report_post",
        tool: "report_post",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/community/posts/{post_id}/report",
        summary: "The signed-in shopper reports a community post as spam, abusive, obscene, an advertisement, fake, off-topic or other. Enough different reporters put it in the moderation queue. One report per shopper and post.",
        consumes: &[Slot::one("post_id")],
        produces: &[Slot::one("report_id")],
        checks: &[
            "moderation.report_reasons",
            "moderation.report_threshold",
            "moderation.blind",
        ],
        errors: &[POST_NOT_FOUND],
        audience: Audience::Shopper,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = ReportPostInput;
    type Output = Value;
}

/// The `moderation.report_reasons` check, or `invalid_input` when the store takes no reports for
/// this reason.
pub(crate) fn reason_check(rules: &ReportRules, reason: &str) -> Result<Check, ToolError> {
    if rules.reasons.iter().any(|r| r == reason) {
        Ok(Check {
            rule: "moderation.report_reasons".into(),
            passed: true,
            detail: Some(format!(
                "The store takes reports for {}",
                reason.replace('_', " ")
            )),
            fix_tool: None,
        })
    } else {
        Err(ToolError::invalid_input(format!(
            "The store takes reports for {} only.",
            rules.reasons.join(", ")
        ))
        .detail(json!({"field": "acom:report_reason", "reason": "enum",
                       "allowed": rules.reasons, "rule": "moderation.report_reasons"}))
        .next("report_post"))
    }
}

/// Holds or blinds the post when its reporters have reached the threshold now. A post moderated
/// in between keeps the moderator's decision.
async fn apply_threshold(tx: &mut TxCx, post_id: &str) -> Result<(), ToolError> {
    let rules = ReportRules::from_policy(&tx.policy)?;
    let Some(p) = repo::post(tx, post_id).await? else {
        return Ok(());
    };
    let mode = repo::board(tx, &p.board_id)
        .await?
        .map(|b| b.moderation_mode)
        .unwrap_or_default();
    let reporters = repo::reporters(tx, post_id).await?;
    let (_, to) = model::threshold_check(&rules, &mode, &p.moderation_status, reporters);
    if let Some(to) = to {
        repo::move_post(tx, post_id, "published", to, Some("report_threshold")).await?;
    }
    Ok(())
}

impl WriteOp<Cx> for ReportPost {
    type Plan = ReportPostPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ReportPostInput,
    ) -> Result<Planned<ReportPostPlan>, ToolError> {
        let customer = cx
            .principal
            .customer_id()
            .map(str::to_owned)
            .ok_or_else(|| {
                ToolError::new(ErrorCode::FORBIDDEN, "Sign in to report a post.")
                    .detail(json!({"reason": "owner"}))
            })?;
        let rules = ReportRules::from_policy(&cx.policy)?;
        let reason = input.report_reason.as_str();
        let reasons = reason_check(&rules, reason)?;
        let viewer = Viewer::of(&cx.principal);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let p = repo::post(&mut tx, &input.post_id)
            .await?
            .filter(|p| {
                p.moderation_status == "published"
                    && post_access(&viewer, &p.author_id, &p.moderation_status, p.is_secret)
                        == PostAccess::Full
            })
            .ok_or_else(|| post_not_found(&input.post_id))?;
        let mode = repo::board(&mut tx, &p.board_id)
            .await?
            .map(|b| b.moderation_mode)
            .unwrap_or_default();
        let existing = repo::report_by(&mut tx, &p.id, &customer).await?;
        let reporters = repo::reporters(&mut tx, &p.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let after = reporters + i64::from(existing.is_none());
        let (threshold, to) = model::threshold_check(&rules, &mode, &p.moderation_status, after);
        let text = trimmed(input.text.as_deref());
        let mut changes = match &existing {
            Some(first) => vec![Change {
                op: ChangeOp::Exclude,
                entity: "report".into(),
                id: first.clone(),
                from: Value::Null,
                to: Value::Null,
                note: Some(format!(
                    "You already reported this post ({first}); the first report stands"
                )),
            }],
            None => vec![Change {
                op: ChangeOp::Create,
                entity: "report".into(),
                id: String::new(),
                from: Value::Null,
                to: json!(reason),
                note: text.clone(),
            }],
        };
        if let (None, Some(to)) = (&existing, to) {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "post".into(),
                id: p.id.clone(),
                from: json!(p.moderation_status),
                to: json!(to),
                note: Some(format!(
                    "{after} different shoppers reported it (moderation.blind.on_threshold)"
                )),
            });
        }
        Ok(Planned {
            changes,
            checks: vec![reasons, threshold],
            basis: vec![Basis {
                entity: "community.discussion_forum_posting".into(),
                id: p.id.clone(),
                version: i64::from(p.version),
            }],
            approval: policy_approval(cx, Self::META.tool, &json!({"reason": reason})),
            plan: ReportPostPlan {
                report_id: format!("rpt_{}", acom_core::id::new_suffix()),
                post_id: p.id,
                customer_id: customer,
                reason: reason.to_owned(),
                text,
                at: cx.clock.now(),
                existing,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ReportPostPlan) -> Result<Committed, ToolError> {
        let done = |id: &str| {
            let mut produced = Map::new();
            produced.insert("report_id".into(), json!(id));
            produced.insert("post_id".into(), json!(plan.post_id));
            Committed {
                id: id.to_owned(),
                produced,
            }
        };
        if let Some(first) = &plan.existing {
            return Ok(done(first));
        }
        let written = repo::insert_report(
            tx,
            &NewReport {
                id: &plan.report_id,
                post_id: &plan.post_id,
                customer_id: &plan.customer_id,
                reason: &plan.reason,
                description: plan.text.as_deref(),
                at: plan.at,
            },
        )
        .await?;
        if !written
            && let Some(first) = repo::report_by(tx, &plan.post_id, &plan.customer_id).await?
            && first != plan.report_id
        {
            return Ok(done(&first));
        }
        apply_threshold(tx, &plan.post_id).await?;
        Ok(done(&plan.report_id))
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ReportPostPlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let found = repo::report_by(&mut tx, &plan.post_id, &plan.customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = found.as_deref() == Some(done.id.as_str());
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("the report on {} was not recorded", plan.post_id)]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &ReportPostPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "The report on {} is with the store's moderators.",
                plan.post_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    fn input(post: &str) -> ReportPostInput {
        ReportPostInput {
            post_id: post.into(),
            report_reason: ReportReason::Advertisement,
            text: Some("Links to another shop.".into()),
        }
    }

    #[test]
    fn reasons_are_the_ones_the_store_takes() {
        let rules =
            ReportRules::from_policy(&testing::policies()).unwrap_or_else(|e| panic!("{e}"));
        assert!(reason_check(&rules, "advertisement").is_ok_and(|c| c.passed));
        let narrow = ReportRules {
            reasons: vec!["spam".into()],
            ..rules
        };
        let e = reason_check(&narrow, "fake").unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("invalid_input", Some("report_post"))
        );
        let schema = acom_contract::schema::closed_schema_for::<ReportPostInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            schema["properties"]["acom:report_reason"]["enum"][3],
            "advertisement"
        );
    }

    #[tokio::test]
    async fn guests_must_sign_in_to_report() {
        let e = ReportPost
            .preview(&testing::guest(), input("post_8811"))
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn the_fifth_reporter_holds_the_post_and_a_second_report_is_the_first() {
        // post_8811 has three reports (Jamie, Grace, Theo). Sam is the fourth, Riley the fifth.
        let sam = testing::as_subject(&testing::on_db(testing::shopper()).await, "cus_sam");
        let fourth = ReportPost
            .preview(&sam, input("post_8811"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            fourth.checks[1].detail.as_deref(),
            Some("4 of 5 reports; one more holds it for review")
        );
        assert_eq!(fourth.changes.len(), 1);
        let mut tx = sam.begin().await.unwrap_or_else(|e| panic!("{e}"));
        ReportPost
            .commit(&mut tx, &fourth.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let riley = testing::as_subject(&sam, "cus_riley");
        let fifth = ReportPost
            .preview(&riley, input("post_8811"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        // The preview reads outside this transaction, so it still counts three reports.
        assert_eq!(fifth.changes.len(), 1);
        let done = ReportPost
            .commit(&mut tx, &fifth.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(done.id, fifth.plan.report_id);
        let post = repo::post(&mut tx, "post_8811")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            post.map(|p| (p.moderation_status, p.report_count)),
            Some(("held_for_review".into(), 5))
        );
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));

        // Jamie reported it already: the first report comes back.
        let jamie = testing::on_db(testing::shopper()).await;
        let again = ReportPost
            .preview(&jamie, input("post_8811"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again.changes[0].op, ChangeOp::Exclude);
        assert_eq!(again.plan.existing.as_deref(), Some("rpt_8811_1"));
    }
}
