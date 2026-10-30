//! `moderate_review` (docs/api/catalog.md#review): hide a review or show it again. Reviews show
//! at once and are moderated afterwards (`review.moderation: post_review`); a hidden review is
//! left out of summaries.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::REVIEW_NOT_FOUND;
use crate::write::{self, ReviewState};

/// What to do with a review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModerationAction {
    /// Hide it from shoppers and from summaries.
    Hide,
    /// Show it again.
    Restore,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModerateReviewInput {
    /// The review.
    #[schemars(regex(pattern = r"^rev_[a-z0-9_]+$"), example = &"rev_8812")]
    pub review_id: String,
    /// hide: take it off the product page and out of summaries; restore: show it again.
    #[schemars(example = &"hide")]
    pub action: ModerationAction,
    /// Why, for the change history.
    #[schemars(length(max = 500), example = &"Mentions a competitor's store")]
    pub reason: Option<String>,
}

/// What `moderate_review` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeratePlan {
    pub review_id: String,
    pub from: String,
    pub to: String,
}

/// Hide a review or show it again.
#[derive(Debug)]
pub struct ModerateReview;

impl Operation for ModerateReview {
    const META: OpMeta = OpMeta {
        address: "catalog.review.moderate",
        tool: "moderate_review",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/reviews/{review_id}/moderate",
        summary: "Hide a review (it leaves the product page and the review summary) or show a hidden one again.",
        consumes: &[Slot::one("review_id")],
        produces: &[Slot::one("review_id")],
        checks: &["review.moderation"],
        errors: &[ErrorCode::INVALID_STATE, REVIEW_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = ModerateReviewInput;
    type Output = Value;
}

fn not_found(id: &str) -> ToolError {
    ToolError::new(REVIEW_NOT_FOUND, format!("There is no review {id}."))
        .detail(json!({"entity": "review", "id": id}))
        .next("list_reviews")
}

/// The status a moderation action moves a review to, or why it cannot.
pub(crate) fn next_status(
    review_id: &str,
    now: &str,
    action: ModerationAction,
) -> Result<&'static str, ToolError> {
    match (action, now) {
        (ModerationAction::Hide, "visible" | "reported") => Ok("hidden"),
        (ModerationAction::Restore, "hidden" | "reported") => Ok("visible"),
        _ => Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("{review_id} is already {now}."),
        )
        .detail(json!({"entity": "review", "from": now}))
        .next("list_reviews")),
    }
}

impl WriteOp<Cx> for ModerateReview {
    type Plan = ModeratePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ModerateReviewInput,
    ) -> Result<Planned<ModeratePlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r: ReviewState = write::review_state(&mut tx, &input.review_id)
            .await?
            .ok_or_else(|| not_found(&input.review_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        let to = next_status(&r.id, &r.creative_work_status, input.action)?;
        let moderation = cx
            .policy
            .lookup("catalog.review.moderation")
            .and_then(Value::as_str)
            .unwrap_or("post_review")
            .replace('_', " ");
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "review".into(),
                id: r.id.clone(),
                from: json!({"status": r.creative_work_status}),
                to: json!({"status": to}),
                note: input
                    .reason
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty()),
            }],
            checks: vec![Check {
                rule: "review.moderation".into(),
                passed: true,
                detail: Some(format!(
                    "Reviews are moderated after posting ({moderation}); a hidden review leaves the summary of {}",
                    r.product_group_id
                )),
                fix_tool: None,
            }],
            basis: vec![Basis {
                entity: "catalog.review".into(),
                id: r.id.clone(),
                version: i64::from(r.version),
            }],
            approval: crate::policy_approval(
                cx,
                Self::META.tool,
                &json!({"change_type": "content"}),
            ),
            plan: ModeratePlan {
                review_id: r.id,
                from: r.creative_work_status,
                to: to.into(),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ModeratePlan) -> Result<Committed, ToolError> {
        if write::set_review_status(tx, &plan.review_id, &plan.to).await? == 0 {
            return Err(not_found(&plan.review_id));
        }
        let mut produced = Map::new();
        produced.insert("review_id".into(), json!(plan.review_id));
        Ok(Committed {
            id: plan.review_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ModeratePlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = write::review_state(&mut tx, &plan.review_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = r.is_some_and(|r| r.creative_work_status == plan.to);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not {}", plan.review_id, plan.to)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ModeratePlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        write::set_review_status(tx, &plan.review_id, &plan.from)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hide_and_restore_move_between_visible_and_hidden() {
        assert_eq!(
            next_status("rev_1", "visible", ModerationAction::Hide).unwrap(),
            "hidden"
        );
        assert_eq!(
            next_status("rev_1", "reported", ModerationAction::Hide).unwrap(),
            "hidden"
        );
        assert_eq!(
            next_status("rev_1", "hidden", ModerationAction::Restore).unwrap(),
            "visible"
        );
        let e = next_status("rev_1", "hidden", ModerationAction::Hide).unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("invalid_state", Some("list_reviews"))
        );
        let schema = acom_contract::schema::closed_schema_for::<ModerateReviewInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            schema["properties"]["action"]["enum"],
            json!(["hide", "restore"])
        );
    }
}
