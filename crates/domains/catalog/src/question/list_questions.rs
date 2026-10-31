//! `list_questions` (docs/api/catalog.md#question): product questions with their answers.
//! Shoppers see visible questions, and private ones only when they asked them; the store sees
//! all of them.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::QuestionStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{QuestionFilter, question_json};
use crate::{INVALID_FILTER, is_operator, list};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListQuestionsInput {
    /// Only questions about this product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_wool_overcoat")]
    pub product_id: Option<String>,
    /// true: only answered questions; false: only questions still waiting for an answer.
    #[schemars(example = false)]
    pub answered: Option<bool>,
    /// Only questions in this state (store only): visible, or hidden by moderation.
    #[schemars(example = &"visible")]
    pub status: Option<QuestionStatus>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// Product questions and their answers.
#[derive(Debug)]
pub struct ListQuestions;

impl Operation for ListQuestions {
    const META: OpMeta = OpMeta {
        address: "catalog.question.list",
        tool: "list_questions",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/questions",
        summary: "Questions shoppers asked about products, newest first, with the store's answer when there is one; by product, answered or waiting, and moderation state.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("question_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListQuestionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListQuestions {
    async fn read(&self, cx: &Cx, input: ListQuestionsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let operator = is_operator(&cx.principal);
        if !operator && input.status == Some(QuestionStatus::Hidden) {
            return Err(ToolError::new(
                INVALID_FILTER,
                "Shoppers see visible questions only; leave out `status`.",
            )
            .detail(json!({"field": "status", "reason": "enum", "allowed": ["visible"]}))
            .next("list_questions"));
        }
        // A guest has no private questions: an id no question has keeps them out.
        let shopper = (!operator).then(|| cx.principal.customer_id().unwrap_or("cus_none"));
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (rows, total) = super::questions(
            &mut tx,
            &QuestionFilter {
                product_id: input.product_id.as_deref(),
                answered: input.answered,
                status: input.status.map(QuestionStatus::as_str),
                shopper,
                limit: page.limit,
                offset: page.offset,
            },
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(list(
            rows.iter().map(|q| question_json(q, &cx.time)).collect(),
            total,
            &page,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    async fn shoppers_cannot_list_hidden_questions() {
        let e = ListQuestions
            .read(
                &testing::shopper(),
                ListQuestionsInput {
                    product_id: None,
                    answered: None,
                    status: Some(QuestionStatus::Hidden),
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_filter");
        let schema = acom_contract::schema::closed_schema_for::<ListQuestionsInput>();
        assert!(acom_contract::schema::violations(&schema).is_empty());
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn overcoat_questions_read_for_shoppers_and_the_store() {
        let input = |answered| ListQuestionsInput {
            product_id: Some("prd_wool_overcoat".into()),
            answered,
            status: None,
            limit: None,
            cursor: None,
        };
        let shopper = testing::shopper_db().await;
        let out = ListQuestions
            .read(&shopper, input(Some(true)))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["total"], json!(1));
        assert_eq!(out["items"][0]["id"], "qst_311");
        assert_eq!(out["items"][0]["accepted_answer"]["id"], "ans_311");
        assert_eq!(out["items"][0]["author"]["name"], "Pr***");
        let mut operator = testing::operator_db().await;
        operator.clock = shopper.clock.clone();
        let all = ListQuestions
            .read(&operator, input(None))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let mine = ListQuestions
            .read(&shopper, input(None))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        // The store also sees the private question another shopper asked.
        assert!(
            all["total"].as_i64() > mine["total"].as_i64(),
            "{all} {mine}"
        );
    }
}
