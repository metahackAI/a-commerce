//! `ask_question` (docs/api/catalog.md#question): a signed-in shopper asks about a product,
//! publicly or privately (`question.private_allowed`). The asker's name is masked
//! (`author_masking`); a blocked word refuses the question or holds it hidden for the store.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{NewQuestion, words_check};
use crate::rules::QuestionRules;
use crate::{PRODUCT_NOT_FOUND, PROFANITY_BLOCKED, product_not_found, visible_group};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AskQuestionInput {
    /// The product asked about (`search_products`).
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_wool_overcoat")]
    pub product_id: String,
    /// The question, in the shopper's words.
    #[schemars(length(min = 1, max = 2000), example = &"Does the M fit over a hoodie?")]
    pub text: String,
    /// true: only the shopper and the store can read it. Default false (everyone can).
    #[serde(rename = "acom:private", alias = "private")]
    #[schemars(example = false)]
    pub private: Option<bool>,
}

/// What `ask_question` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskPlan {
    pub question_id: String,
    pub product_id: String,
    pub author_id: String,
    pub author_name: String,
    pub text: String,
    pub private: bool,
    /// `visible`, or `hidden` when a blocked word holds it for the store.
    pub status: String,
    pub at: DateTime<Utc>,
    /// The blocked word found, when the store refuses such text.
    pub blocked_word: Option<String>,
    /// A private question the store does not take.
    pub private_refused: bool,
}

/// A shopper asks about a product.
#[derive(Debug)]
pub struct AskQuestion;

impl Operation for AskQuestion {
    const META: OpMeta = OpMeta {
        address: "catalog.question.ask",
        tool: "ask_question",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/questions",
        summary: "The signed-in shopper asks a question about a product, for everyone to read or privately for the store. The store answers with answer_question.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("question_id")],
        checks: &["question.profanity_filter", "question.private_allowed"],
        errors: &[PROFANITY_BLOCKED, PRODUCT_NOT_FOUND],
        audience: Audience::Shopper,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = AskQuestionInput;
    type Output = Value;
}

/// `question.private_allowed`: a private question only when the store takes them.
pub(crate) fn private_check(rules: &QuestionRules, private: bool) -> Check {
    let refused = private && !rules.private_allowed;
    Check {
        rule: "question.private_allowed".into(),
        passed: !refused,
        detail: Some(match (private, refused) {
            (false, _) => "Public: every shopper can read it".into(),
            (true, false) => "Private: only you and the store can read it".into(),
            (true, true) => "This store takes public questions only".into(),
        }),
        fix_tool: refused.then(|| "ask_question".into()),
    }
}

impl WriteOp<Cx> for AskQuestion {
    type Plan = AskPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: AskQuestionInput,
    ) -> Result<Planned<AskPlan>, ToolError> {
        let author = crate::signed_in_shopper(cx, "ask about a product")?;
        let text = input.text.trim().to_owned();
        if text.is_empty() {
            return Err(ToolError::invalid_input("The question is empty.")
                .detail(json!({"field": "text", "reason": "missing"})));
        }
        let rules = QuestionRules::from_policy(&cx.policy)?;
        let author_name = crate::shown_author(cx)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = visible_group(cx, &mut tx, &input.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let group = group
            .ok_or_else(|| product_not_found("product", &input.product_id, "search_products"))?;
        let private = input.private.unwrap_or(false);
        let (words, blocked, held) = words_check(&rules, &text, "ask_question");
        let private_ok = private_check(&rules, private);
        let status = if held { "hidden" } else { "visible" };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "question".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"product_id": group.id, "text": text, "acom:private": private,
                           "status": status, "author": author_name}),
                note: held.then(|| "Held for the store to look at before shoppers see it".into()),
            }],
            checks: vec![words, private_ok.clone()],
            basis: vec![Basis {
                entity: "catalog.product_group".into(),
                id: group.id.clone(),
                version: i64::from(group.version),
            }],
            approval: ApprovalNeed::NotNeeded,
            plan: AskPlan {
                question_id: format!("qst_{}", acom_core::id::new_suffix()),
                product_id: group.id,
                author_id: author,
                author_name,
                text,
                private,
                status: status.into(),
                at: cx.clock.now(),
                blocked_word: blocked,
                private_refused: !private_ok.passed,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &AskPlan) -> Result<Committed, ToolError> {
        if let Some(w) = &plan.blocked_word {
            return Err(crate::profanity_blocked(
                "question.profanity_filter",
                w,
                "ask_question",
            ));
        }
        if plan.private_refused {
            return Err(ToolError::new(
                ErrorCode::POLICY_VIOLATION,
                "This store takes public questions only.",
            )
            .detail(json!({"rule": "question.private_allowed"}))
            .next("ask_question"));
        }
        super::insert_question(
            tx,
            &NewQuestion {
                id: &plan.question_id,
                product_group_id: &plan.product_id,
                author_id: &plan.author_id,
                author_name: &plan.author_name,
                text: &plan.text,
                is_private: plan.private,
                status: &plan.status,
                at: plan.at,
            },
        )
        .await?;
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
        plan: &AskPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let q = super::question(&mut tx, &plan.question_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched =
            q.is_some_and(|q| q.text == plan.text && q.product_group_id == plan.product_id);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} does not read back as asked", plan.question_id)]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &AskPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} is the shopper's own question; they remove it with delete_question.",
                plan.question_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn private_questions_follow_the_store() {
        let mut rules = QuestionRules::from_policy(&crate::testing::policies())
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(private_check(&rules, true).passed);
        rules.private_allowed = false;
        let c = private_check(&rules, true);
        assert!(!c.passed);
        assert!(private_check(&rules, false).passed);
        let schema = acom_contract::schema::closed_schema_for::<AskQuestionInput>();
        assert!(acom_contract::schema::violations(&schema).is_empty());
        assert!(schema["properties"].get("acom:private").is_some());
    }

    #[tokio::test]
    async fn guests_must_sign_in_and_questions_need_text() {
        let input = |text: &str| AskQuestionInput {
            product_id: "prd_wool_overcoat".into(),
            text: text.into(),
            private: None,
        };
        let e = AskQuestion
            .preview(&testing::guest(), input("Warm?"))
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let e = AskQuestion
            .preview(&testing::shopper(), input("  "))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_asks_about_the_overcoat() {
        let cx = testing::shopper_db().await;
        let planned = AskQuestion
            .preview(
                &cx,
                AskQuestionInput {
                    product_id: "prd_wool_overcoat".into(),
                    text: "Does the M fit over a hoodie?".into(),
                    private: Some(true),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes[0].to["author"], "Ja***");
        assert!(planned.checks.iter().all(|c| c.passed));
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = AskQuestion
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let q = super::super::question(&mut tx, &done.id)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("not written"));
        assert!(q.is_private && q.answer_id.is_none());
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
    }
}
