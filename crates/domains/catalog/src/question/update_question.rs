//! `update_question` (docs/api/catalog.md#question): the asker changes their question's text or
//! privacy, until the store answers it (`question.edit_until: answered`).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::ask_question::private_check;
use super::{QuestionRow, question_not_found, words_check};
use crate::rules::QuestionRules;
use crate::{ALREADY_ANSWERED, NOT_AUTHOR, PROFANITY_BLOCKED, db};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateQuestionInput {
    /// The question to change; it must be the signed-in shopper's own (`list_questions`).
    #[schemars(regex(pattern = r"^qst_[a-z0-9_]+$"), example = &"qst_313")]
    pub question_id: String,
    /// The new question text. Leave out to keep it.
    #[schemars(length(min = 1, max = 2000), example = &"Does the hood roll into the collar, or pack into the pocket?")]
    pub text: Option<String>,
    /// true: only the shopper and the store can read it; false: everyone can. Leave out to keep it.
    #[serde(rename = "acom:private", alias = "private")]
    #[schemars(example = true)]
    pub private: Option<bool>,
}

/// What `update_question` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateQuestionPlan {
    pub question_id: String,
    pub author_id: String,
    pub text: String,
    pub private: bool,
    pub status: String,
    pub blocked_word: Option<String>,
    pub private_refused: bool,
}

/// The asker changes their unanswered question.
#[derive(Debug)]
pub struct UpdateQuestion;

impl Operation for UpdateQuestion {
    const META: OpMeta = OpMeta {
        address: "catalog.question.update",
        tool: "update_question",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/catalog/questions/{question_id}",
        summary: "The signed-in shopper changes the text or privacy of a question they asked, while it has no answer yet.",
        consumes: &[Slot::one("question_id")],
        produces: &[Slot::one("question_id")],
        checks: &[
            "question.edit_until",
            "question.profanity_filter",
            "question.private_allowed",
        ],
        errors: &[
            NOT_AUTHOR,
            ALREADY_ANSWERED,
            PROFANITY_BLOCKED,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateQuestionInput;
    type Output = Value;
}

/// `already_answered`: an answered question is kept as it was answered.
pub(crate) fn already_answered(question_id: &str) -> ToolError {
    ToolError::new(
        ALREADY_ANSWERED,
        format!("{question_id} has its answer already; ask a new question instead."),
    )
    .detail(json!({"entity": "question", "id": question_id, "from": "answered"}))
    .next("list_questions")
}

/// The asker's question, or `not_found` / `not_author`.
pub(crate) async fn askers_question(
    tx: &mut TxCx,
    id: &str,
    shopper: &str,
) -> Result<QuestionRow, ToolError> {
    let q = super::question(tx, id)
        .await?
        .ok_or_else(|| question_not_found(id))?;
    if q.author_id != shopper {
        return Err(crate::not_author("question", id, "list_questions"));
    }
    Ok(q)
}

impl WriteOp<Cx> for UpdateQuestion {
    type Plan = UpdateQuestionPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateQuestionInput,
    ) -> Result<Planned<UpdateQuestionPlan>, ToolError> {
        let author = crate::signed_in_shopper(cx, "change a question")?;
        if input.text.is_none() && input.private.is_none() {
            return Err(
                ToolError::invalid_input("Give the new `text` or `acom:private`.")
                    .detail(json!({"field": "text", "reason": "missing"}))
                    .next("update_question"),
            );
        }
        let text = input.text.map(|t| t.trim().to_owned());
        if text.as_deref().is_some_and(str::is_empty) {
            return Err(ToolError::invalid_input("The question is empty.")
                .detail(json!({"field": "text", "reason": "missing"})));
        }
        let rules = QuestionRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let q = askers_question(&mut tx, &input.question_id, &author).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if rules.edit_until_answered && q.answer_id.is_some() {
            return Err(already_answered(&q.id));
        }
        let new_text = text.unwrap_or_else(|| q.text.clone());
        let private = input.private.unwrap_or(q.is_private);
        let (words, blocked, held) = words_check(&rules, &new_text, "update_question");
        let private_ok = private_check(&rules, private && !q.is_private);
        let status = if held {
            "hidden".to_owned()
        } else {
            q.creative_work_status.clone()
        };
        let edit = Check {
            rule: "question.edit_until".into(),
            passed: true,
            detail: Some("No answer yet, so it can still change".into()),
            fix_tool: None,
        };
        let mut from = Map::new();
        let mut to = Map::new();
        if new_text != q.text {
            from.insert("text".into(), json!(q.text));
            to.insert("text".into(), json!(new_text));
        }
        if private != q.is_private {
            from.insert("acom:private".into(), json!(q.is_private));
            to.insert("acom:private".into(), json!(private));
        }
        if status != q.creative_work_status {
            from.insert("status".into(), json!(q.creative_work_status));
            to.insert("status".into(), json!(status));
        }
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "question".into(),
                id: q.id.clone(),
                note: from.is_empty().then(|| "Nothing changes".into()),
                from: Value::Object(from),
                to: Value::Object(to),
            }],
            checks: vec![edit, words, private_ok.clone()],
            basis: vec![Basis {
                entity: "catalog.question".into(),
                id: q.id.clone(),
                version: i64::from(q.version),
            }],
            approval: ApprovalNeed::NotNeeded,
            plan: UpdateQuestionPlan {
                question_id: q.id,
                author_id: author,
                text: new_text,
                private,
                status,
                blocked_word: blocked,
                private_refused: !private_ok.passed,
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &UpdateQuestionPlan,
    ) -> Result<Committed, ToolError> {
        if let Some(w) = &plan.blocked_word {
            return Err(crate::profanity_blocked(
                "question.profanity_filter",
                w,
                "update_question",
            ));
        }
        if plan.private_refused {
            return Err(ToolError::new(
                ErrorCode::POLICY_VIOLATION,
                "This store takes public questions only.",
            )
            .detail(json!({"rule": "question.private_allowed"}))
            .next("update_question"));
        }
        let rules = QuestionRules::from_policy(&tx.policy)?;
        if rules.edit_until_answered && super::is_answered(tx, &plan.question_id).await? {
            return Err(already_answered(&plan.question_id));
        }
        let n = sqlx::query(
            "UPDATE catalog.question SET text = $4, is_private = $5, creative_work_status = $6,
               updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2 AND author_id = $3",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.question_id)
        .bind(&plan.author_id)
        .bind(&plan.text)
        .bind(plan.private)
        .bind(&plan.status)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?
        .rows_affected();
        if n == 0 {
            return Err(question_not_found(&plan.question_id));
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
        plan: &UpdateQuestionPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let q = super::question(&mut tx, &plan.question_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = q.is_some_and(|q| q.text == plan.text && q.is_private == plan.private);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} does not read back as changed",
                    plan.question_id
                )]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &UpdateQuestionPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} is the shopper's own question; they change it again with update_question.",
                plan.question_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    async fn something_must_change_and_guests_must_sign_in() {
        let input = || UpdateQuestionInput {
            question_id: "qst_313".into(),
            text: None,
            private: None,
        };
        let e = UpdateQuestion
            .preview(&testing::shopper(), input())
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        let e = UpdateQuestion
            .preview(&testing::guest(), input())
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        assert_eq!(already_answered("qst_311").code, "already_answered");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn only_the_asker_edits_and_only_before_the_answer() {
        let cx = testing::shopper_db().await;
        let input = |id: &str| UpdateQuestionInput {
            question_id: id.into(),
            text: Some("Changed".into()),
            private: None,
        };
        // qst_313 was asked by another shopper.
        let e = UpdateQuestion
            .preview(&cx, input("qst_313"))
            .await
            .unwrap_err();
        assert_eq!(e.code, "not_author");
        let e = UpdateQuestion
            .preview(&cx, input("qst_nothing"))
            .await
            .unwrap_err();
        assert_eq!(e.code, "not_found");
    }
}
