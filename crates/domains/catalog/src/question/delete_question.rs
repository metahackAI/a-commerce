//! `delete_question` (docs/api/catalog.md#question): the asker removes their question, with the
//! store's answer if it has one. A question that is gone already gives the same result.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::update_question::askers_question;
use crate::{NOT_AUTHOR, db};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteQuestionInput {
    /// The question to remove; it must be the signed-in shopper's own (`list_questions`).
    #[schemars(regex(pattern = r"^qst_[a-z0-9_]+$"), example = &"qst_313")]
    pub question_id: String,
}

/// What `delete_question` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteQuestionPlan {
    pub question_id: String,
    pub author_id: String,
}

/// The asker removes their question.
#[derive(Debug)]
pub struct DeleteQuestion;

impl Operation for DeleteQuestion {
    const META: OpMeta = OpMeta {
        address: "catalog.question.delete",
        tool: "delete_question",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Delete,
        path: "/v1/catalog/questions/{question_id}",
        summary: "The signed-in shopper removes a question they asked, with the store's answer if it has one.",
        consumes: &[Slot::one("question_id")],
        produces: &[],
        checks: &[],
        errors: &[NOT_AUTHOR, ErrorCode::NOT_FOUND],
        audience: Audience::Shopper,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = DeleteQuestionInput;
    type Output = Value;
}

impl WriteOp<Cx> for DeleteQuestion {
    type Plan = DeleteQuestionPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: DeleteQuestionInput,
    ) -> Result<Planned<DeleteQuestionPlan>, ToolError> {
        let author = crate::signed_in_shopper(cx, "remove a question")?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let q = askers_question(&mut tx, &input.question_id, &author).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut changes = vec![Change {
            op: ChangeOp::Delete,
            entity: "question".into(),
            id: q.id.clone(),
            from: json!({"product_id": q.product_group_id, "text": q.text,
                         "acom:private": q.is_private}),
            to: Value::Null,
            note: Some("It cannot be brought back".into()),
        }];
        if let Some(answer) = &q.answer_id {
            changes.push(Change {
                op: ChangeOp::Delete,
                entity: "answer".into(),
                id: answer.clone(),
                from: json!({"question_id": q.id, "text": q.answer_text}),
                to: Value::Null,
                note: Some("The store's answer goes with the question".into()),
            });
        }
        Ok(Planned {
            changes,
            checks: Vec::new(),
            basis: vec![Basis {
                entity: "catalog.question".into(),
                id: q.id.clone(),
                version: i64::from(q.version),
            }],
            approval: ApprovalNeed::NotNeeded,
            plan: DeleteQuestionPlan {
                question_id: q.id,
                author_id: author,
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &DeleteQuestionPlan,
    ) -> Result<Committed, ToolError> {
        // The answer goes by ON DELETE CASCADE. Gone already is the same result.
        sqlx::query(
            "DELETE FROM catalog.question WHERE tenant_id = $1 AND id = $2 AND author_id = $3",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.question_id)
        .bind(&plan.author_id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
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
        plan: &DeleteQuestionPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let q = super::question(&mut tx, &plan.question_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: q.is_none(),
            mismatches: match q {
                None => Vec::new(),
                Some(_) => vec![format!("{} is still there", plan.question_id)],
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &DeleteQuestionPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} was removed by the shopper who asked it; it cannot be brought back.",
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
    async fn guests_cannot_delete_questions() {
        let e = DeleteQuestion
            .preview(
                &testing::guest(),
                DeleteQuestionInput {
                    question_id: "qst_313".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let schema = acom_contract::schema::closed_schema_for::<DeleteQuestionInput>();
        assert!(acom_contract::schema::violations(&schema).is_empty());
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn the_asker_deletes_a_question_with_its_answer() {
        // qst_311 was asked by Priya and answered by the store.
        let mut cx = testing::shopper_db().await;
        let e = DeleteQuestion
            .preview(
                &cx,
                DeleteQuestionInput {
                    question_id: "qst_311".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "not_author");
        let mut p = (*cx.principal).clone();
        p.subject = "cus_priya".into();
        cx.principal = std::sync::Arc::new(p);
        let planned = DeleteQuestion
            .preview(
                &cx,
                DeleteQuestionInput {
                    question_id: "qst_311".into(),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes[1].entity, "answer");
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        DeleteQuestion
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let gone = super::super::question(&mut tx, "qst_311")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(gone.is_none());
        // Running it again changes nothing and still succeeds.
        DeleteQuestion
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
    }
}
