//! `moderate_question` (docs/api/catalog.md#question): the store hides a question (and its
//! answer) from shoppers, or shows a hidden one again, including one a blocked word held back.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::question_not_found;
use crate::db;

/// What to do with a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuestionModeration {
    /// Take the question and its answer off the product page.
    Hide,
    /// Show it again.
    Restore,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModerateQuestionInput {
    /// The question (`list_questions`).
    #[schemars(regex(pattern = r"^qst_[a-z0-9_]+$"), example = &"qst_313")]
    pub question_id: String,
    /// hide: take the question and its answer off the product page; restore: show it again.
    #[schemars(example = &"hide")]
    pub action: QuestionModeration,
    /// Why, for the change history.
    #[schemars(length(max = 500), example = &"Shares a phone number")]
    pub reason: Option<String>,
}

/// What `moderate_question` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModerateQuestionPlan {
    pub question_id: String,
    pub from: String,
    pub to: String,
}

/// Hide a question or show it again.
#[derive(Debug)]
pub struct ModerateQuestion;

impl Operation for ModerateQuestion {
    const META: OpMeta = OpMeta {
        address: "catalog.question.moderate",
        tool: "moderate_question",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/questions/{question_id}/moderate",
        summary: "Hide a product question and its answer from shoppers, or show a hidden one again.",
        consumes: &[Slot::one("question_id")],
        produces: &[Slot::one("question_id")],
        checks: &[],
        errors: &[ErrorCode::INVALID_STATE, ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = ModerateQuestionInput;
    type Output = Value;
}

/// The state a moderation action moves a question to, or why it cannot.
pub(crate) fn next_status(
    question_id: &str,
    now: &str,
    action: QuestionModeration,
) -> Result<&'static str, ToolError> {
    match (action, now) {
        (QuestionModeration::Hide, "visible") => Ok("hidden"),
        (QuestionModeration::Restore, "hidden") => Ok("visible"),
        _ => Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("{question_id} is already {now}."),
        )
        .detail(json!({"entity": "question", "from": now}))
        .next("list_questions")),
    }
}

async fn set_status(tx: &mut TxCx, id: &str, from: &str, to: &str) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE catalog.question SET creative_work_status = $4, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND creative_work_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(from)
    .bind(to)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

impl WriteOp<Cx> for ModerateQuestion {
    type Plan = ModerateQuestionPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ModerateQuestionInput,
    ) -> Result<Planned<ModerateQuestionPlan>, ToolError> {
        crate::operators_only(cx, "moderate questions")?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let q = super::question(&mut tx, &input.question_id)
            .await?
            .ok_or_else(|| question_not_found(&input.question_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        let to = next_status(&q.id, &q.creative_work_status, input.action)?;
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "question".into(),
                id: q.id.clone(),
                from: json!({"status": q.creative_work_status}),
                to: json!({"status": to, "answered": q.answer_id.is_some()}),
                note: input
                    .reason
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty()),
            }],
            checks: Vec::new(),
            basis: vec![Basis {
                entity: "catalog.question".into(),
                id: q.id.clone(),
                version: i64::from(q.version),
            }],
            approval: ApprovalNeed::NotNeeded,
            plan: ModerateQuestionPlan {
                question_id: q.id,
                from: q.creative_work_status,
                to: to.into(),
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &ModerateQuestionPlan,
    ) -> Result<Committed, ToolError> {
        if set_status(tx, &plan.question_id, &plan.from, &plan.to).await? == 0 {
            // The same plan run again finds it moved already.
            let now = super::question(tx, &plan.question_id)
                .await?
                .ok_or_else(|| question_not_found(&plan.question_id))?;
            if now.creative_work_status != plan.to {
                return Err(ToolError::new(
                    ErrorCode::INVALID_STATE,
                    format!(
                        "{} is {} now, not {}; preview it again.",
                        plan.question_id, now.creative_work_status, plan.from
                    ),
                )
                .detail(json!({"entity": "question", "from": now.creative_work_status}))
                .next("list_questions"));
            }
        }
        let mut produced = Map::new();
        produced.insert("question_id".into(), json!(plan.question_id));
        Ok(Committed {
            id: plan.question_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ModerateQuestionPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let q = super::question(&mut tx, &plan.question_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = q.is_some_and(|q| q.creative_work_status == plan.to);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not {}", plan.question_id, plan.to)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ModerateQuestionPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        set_status(tx, &plan.question_id, &plan.to, &plan.from)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn hide_and_restore_move_between_visible_and_hidden() {
        assert_eq!(
            next_status("qst_1", "visible", QuestionModeration::Hide).ok(),
            Some("hidden")
        );
        assert_eq!(
            next_status("qst_1", "hidden", QuestionModeration::Restore).ok(),
            Some("visible")
        );
        let e = next_status("qst_1", "hidden", QuestionModeration::Hide).unwrap_err();
        assert_eq!(e.code, "invalid_state");
        let schema = acom_contract::schema::closed_schema_for::<ModerateQuestionInput>();
        assert!(acom_contract::schema::violations(&schema).is_empty());
    }

    #[tokio::test]
    async fn shoppers_cannot_moderate() {
        let e = ModerateQuestion
            .preview(
                &testing::shopper(),
                ModerateQuestionInput {
                    question_id: "qst_313".into(),
                    action: QuestionModeration::Hide,
                    reason: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn the_store_hides_a_question_and_undoes_it() {
        let cx = testing::operator_db().await;
        let planned = ModerateQuestion
            .preview(
                &cx,
                ModerateQuestionInput {
                    question_id: "qst_313".into(),
                    action: QuestionModeration::Hide,
                    reason: Some("Test".into()),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes[0].to["status"], "hidden");
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = ModerateQuestion
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        ModerateQuestion
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let q = super::super::question(&mut tx, "qst_313")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(q.map(|q| q.creative_work_status), Some("visible".into()));
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
    }
}
