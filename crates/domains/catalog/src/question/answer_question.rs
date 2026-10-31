//! `answer_question` (docs/api/catalog.md#question): the store answers a product question. Who
//! may answer is the store's rule (`question.answered_by`: the store, and other shoppers when it
//! says so); a question has one answer. Shoppers read it as soon as it runs, so it is not undone:
//! a wrong answer is hidden with `moderate_question` and the question asked again.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::update_question::already_answered;
use super::{QuestionRow, question_not_found};
use crate::rules::QuestionRules;
use crate::{ALREADY_ANSWERED, PROFANITY_BLOCKED, db, is_operator};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnswerQuestionInput {
    /// The question to answer (`list_questions` with `answered: false`).
    #[schemars(regex(pattern = r"^qst_[a-z0-9_]+$"), example = &"qst_313")]
    pub question_id: String,
    /// The answer, word for word as shoppers will read it.
    #[schemars(length(min = 1, max = 2000), example = &"It packs into its own chest pocket; the hood folds in with it.")]
    pub text: String,
}

/// What `answer_question` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerPlan {
    pub answer_id: String,
    pub question_id: String,
    /// Who answers: `adm_…`, `sys_…`, or a shopper `cus_…`.
    pub author_id: String,
    /// The masked name of a shopper who answers; `None` when the store answers.
    pub author_name: Option<String>,
    pub text: String,
    pub at: DateTime<Utc>,
    pub blocked_word: Option<String>,
}

/// The store answers a product question.
#[derive(Debug)]
pub struct AnswerQuestion;

impl Operation for AnswerQuestion {
    const META: OpMeta = OpMeta {
        address: "catalog.question.answer",
        tool: "answer_question",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/questions/{question_id}/answer",
        summary: "Answer a shopper's product question; the answer shows under the question at once and cannot be taken back. One answer per question.",
        consumes: &[Slot::one("question_id")],
        produces: &[Slot::one("answer_id")],
        checks: &["question.answered_by", "question.profanity_filter"],
        errors: &[ALREADY_ANSWERED, PROFANITY_BLOCKED, ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = AnswerQuestionInput;
    type Output = Value;
}

/// Who is answering, as `question.answered_by` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answerer {
    /// The store: an operator, the operator's agent, or the system.
    Store,
    /// A signed-in shopper.
    Shopper,
    /// A guest.
    Nobody,
}

/// The `question.answered_by` check: whether this answerer may answer this question.
pub(crate) fn answerer_check(
    rules: &QuestionRules,
    who: Answerer,
    q: &QuestionRow,
    me: &str,
) -> Check {
    let (ok, detail) = match who {
        Answerer::Store if rules.answered_by.iter().any(|w| w == "operator") => {
            (true, "The store answers its product questions".to_owned())
        }
        Answerer::Shopper
            if rules.answered_by.iter().any(|w| w == "shopper")
                && q.author_id != me
                && !q.is_private
                && q.creative_work_status == "visible" =>
        {
            (
                true,
                "Shoppers may answer each other's questions here".to_owned(),
            )
        }
        Answerer::Shopper if q.author_id == me => {
            (false, "You asked it; the store answers".to_owned())
        }
        _ => (
            false,
            format!("Only {} may answer", rules.answered_by.join(" and ")),
        ),
    };
    Check {
        rule: "question.answered_by".into(),
        passed: ok,
        detail: Some(detail),
        fix_tool: None,
    }
}

impl WriteOp<Cx> for AnswerQuestion {
    type Plan = AnswerPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: AnswerQuestionInput,
    ) -> Result<Planned<AnswerPlan>, ToolError> {
        let text = input.text.trim().to_owned();
        if text.is_empty() {
            return Err(ToolError::invalid_input("The answer is empty.")
                .detail(json!({"field": "text", "reason": "missing"})));
        }
        let rules = QuestionRules::from_policy(&cx.policy)?;
        let who = if is_operator(&cx.principal) {
            Answerer::Store
        } else if cx.principal.customer_id().is_some() {
            Answerer::Shopper
        } else {
            Answerer::Nobody
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let q = super::question(&mut tx, &input.question_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        // Shoppers only reach questions they can read.
        let q = q
            .filter(|q| {
                who == Answerer::Store
                    || (q.creative_work_status == "visible"
                        && (!q.is_private
                            || Some(q.author_id.as_str()) == cx.principal.customer_id()))
            })
            .ok_or_else(|| question_not_found(&input.question_id))?;
        if q.answer_id.is_some() {
            return Err(already_answered(&q.id));
        }
        let allowed = answerer_check(&rules, who, &q, &cx.principal.subject);
        if !allowed.passed {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                allowed.detail.clone().unwrap_or_default(),
            )
            .detail(json!({"reason": "role", "rule": "question.answered_by"})));
        }
        // An answer is read the moment it is posted, so there is nothing to hold it in: a blocked
        // word refuses it under either filter setting.
        let blocked = crate::model::blocked_word(&text, &rules.blocked_words);
        let words = crate::blocked_words_check(
            "question.profanity_filter",
            true,
            blocked.as_deref(),
            "answer_question",
        );
        let author_name = match who {
            Answerer::Shopper => Some(crate::shown_author(cx)?),
            _ => None,
        };
        let shown = author_name.clone().unwrap_or_else(|| "The store".into());
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "answer".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"question_id": q.id, "question": q.text, "text": text,
                           "author": shown, "acom:private": q.is_private}),
                note: Some("Shoppers read it as soon as it runs; it cannot be taken back".into()),
            }],
            checks: vec![allowed, words],
            basis: vec![Basis {
                entity: "catalog.question".into(),
                id: q.id.clone(),
                version: i64::from(q.version),
            }],
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan: AnswerPlan {
                answer_id: format!("ans_{}", acom_core::id::new_suffix()),
                question_id: q.id,
                author_id: cx.principal.subject.clone(),
                author_name,
                text,
                at: cx.clock.now(),
                blocked_word: blocked,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &AnswerPlan) -> Result<Committed, ToolError> {
        if let Some(w) = &plan.blocked_word {
            return Err(crate::profanity_blocked(
                "question.profanity_filter",
                w,
                "answer_question",
            ));
        }
        let written = sqlx::query(
            "INSERT INTO catalog.answer (tenant_id, id, question_id, author_id, author_name, text,
               date_created)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.answer_id)
        .bind(&plan.question_id)
        .bind(&plan.author_id)
        .bind(&plan.author_name)
        .bind(&plan.text)
        .bind(plan.at)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?
        .rows_affected();
        if written == 0 {
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT id FROM catalog.answer WHERE tenant_id = $1 AND question_id = $2",
            )
            .bind(tx.tenant.as_str())
            .bind(&plan.question_id)
            .fetch_optional(&mut *tx.tx)
            .await
            .map_err(db)?;
            if existing.as_deref() != Some(plan.answer_id.as_str()) {
                return Err(already_answered(&plan.question_id));
            }
        }
        let mut produced = Map::new();
        produced.insert("answer_id".into(), json!(plan.answer_id));
        produced.insert("question_id".into(), json!(plan.question_id));
        Ok(Committed {
            id: plan.answer_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &AnswerPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let q = super::question(&mut tx, &plan.question_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = q.is_some_and(|q| {
            q.answer_id.as_deref() == Some(plan.answer_id.as_str())
                && q.answer_text.as_deref() == Some(plan.text.as_str())
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} has no answer {}",
                    plan.question_id, plan.answer_id
                )]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &AnswerPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "Shoppers may have read {} already; hide the question with moderate_question instead.",
                plan.answer_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;
    use acom_contract::ApprovalNeed;

    fn question(author: &str, private: bool) -> QuestionRow {
        QuestionRow {
            id: "qst_313".into(),
            product_group_id: "prd_windbreaker".into(),
            product_name: "Packable windbreaker".into(),
            author_id: author.into(),
            author_name: "Da***".into(),
            text: "Does the hood roll into the collar?".into(),
            is_private: private,
            creative_work_status: "visible".into(),
            date_created: chrono::DateTime::default(),
            version: 1,
            answer_id: None,
            answer_text: None,
            answer_author_name: None,
            answer_author_id: None,
            answer_date_created: None,
        }
    }

    #[test]
    fn the_store_answers_and_shoppers_only_when_the_policy_says() {
        let mut rules = QuestionRules::from_policy(&crate::testing::policies())
            .unwrap_or_else(|e| panic!("{e}"));
        let q = question("cus_dana", false);
        assert!(answerer_check(&rules, Answerer::Store, &q, "adm_sj").passed);
        assert!(!answerer_check(&rules, Answerer::Shopper, &q, "cus_jamie").passed);
        assert!(!answerer_check(&rules, Answerer::Nobody, &q, "ses_x").passed);
        rules.answered_by.push("shopper".into());
        assert!(answerer_check(&rules, Answerer::Shopper, &q, "cus_jamie").passed);
        // Not their own question, and not a private one.
        assert!(!answerer_check(&rules, Answerer::Shopper, &q, "cus_dana").passed);
        let private = question("cus_dana", true);
        assert!(!answerer_check(&rules, Answerer::Shopper, &private, "cus_jamie").passed);
    }

    #[tokio::test]
    async fn an_empty_answer_is_refused() {
        let e = AnswerQuestion
            .preview(
                &testing::cx(acom_core::enums::ActorKind::Operator, "adm_sj"),
                AnswerQuestionInput {
                    question_id: "qst_313".into(),
                    text: " ".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn the_store_answers_the_windbreaker_question_once() {
        let cx = testing::operator_db().await;
        let input = |id: &str| AnswerQuestionInput {
            question_id: id.into(),
            text: "It packs into its own chest pocket; the hood folds in with it.".into(),
        };
        let e = AnswerQuestion
            .preview(&cx, input("qst_311"))
            .await
            .unwrap_err();
        assert_eq!(e.code, "already_answered");
        let planned = AnswerQuestion
            .preview(&cx, input("qst_313"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes[0].to["author"], "The store");
        assert_eq!(planned.approval, ApprovalNeed::NotNeeded);
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        AnswerQuestion
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let mut second = planned.plan.clone();
        second.answer_id = "ans_second".into();
        let e = AnswerQuestion.commit(&mut tx, &second).await.unwrap_err();
        assert_eq!(e.code, "already_answered");
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
        // Shoppers cannot answer under the default policy.
        let shopper = testing::shopper_db().await;
        let e = AnswerQuestion
            .preview(&shopper, input("qst_313"))
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }
}
