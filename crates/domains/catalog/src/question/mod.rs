//! The `question` resource (docs/api/catalog.md#question): shoppers ask about a product, the
//! store answers (one answer per question, `question.answered_by`). A private question is read
//! by its asker and the store only; a hidden one leaves the product page with its answer. The
//! asker may edit a question until it is answered (`question.edit_until`) and delete it.

pub mod answer_question;
pub mod ask_question;
pub mod delete_question;
pub mod list_questions;
pub mod moderate_question;
pub mod update_question;

use acom_contract::{Check, ToolError};
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::rules::QuestionRules;
use crate::{blocked_words_check, db, model, not_found};

/// `not_found` for a question, pointing at `list_questions`.
fn question_not_found(id: &str) -> ToolError {
    not_found("question", id, "list_questions")
}

/// A question with its answer, as stored.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct QuestionRow {
    pub id: String,
    pub product_group_id: String,
    pub product_name: String,
    pub author_id: String,
    pub author_name: String,
    pub text: String,
    pub is_private: bool,
    pub creative_work_status: String,
    pub date_created: DateTime<Utc>,
    pub version: i32,
    pub answer_id: Option<String>,
    pub answer_text: Option<String>,
    pub answer_author_name: Option<String>,
    pub answer_author_id: Option<String>,
    pub answer_date_created: Option<DateTime<Utc>>,
}

const QUESTION_SELECT: &str = "SELECT q.id, q.product_group_id, g.name AS product_name,
       q.author_id, q.author_name, q.text, q.is_private, q.creative_work_status,
       q.date_created, q.version, a.id AS answer_id, a.text AS answer_text,
       a.author_name AS answer_author_name, a.author_id AS answer_author_id,
       a.date_created AS answer_date_created
  FROM catalog.question q
  JOIN catalog.product_group g ON g.tenant_id = q.tenant_id AND g.id = q.product_group_id
  LEFT JOIN catalog.answer a ON a.tenant_id = q.tenant_id AND a.question_id = q.id";

pub(crate) async fn question(tx: &mut TxCx, id: &str) -> Result<Option<QuestionRow>, ToolError> {
    sqlx::query_as::<_, QuestionRow>(sqlx::AssertSqlSafe(format!(
        "{QUESTION_SELECT} WHERE q.tenant_id = $1 AND q.id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Filters of [`questions`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct QuestionFilter<'a> {
    pub product_id: Option<&'a str>,
    pub answered: Option<bool>,
    pub status: Option<&'a str>,
    /// A shopper sees visible questions only, and private ones only when they asked them.
    /// `None` for the store, which sees everything.
    pub shopper: Option<&'a str>,
    pub limit: i64,
    pub offset: i64,
}

const QUESTION_WHERE: &str = "WHERE q.tenant_id = $1
      AND ($2::text IS NULL OR q.product_group_id = $2)
      AND ($3::boolean IS NULL OR (a.id IS NOT NULL) = $3)
      AND ($4::text IS NULL OR q.creative_work_status = $4)
      AND ($5::text IS NULL OR (q.creative_work_status = 'visible'
                                AND (NOT q.is_private OR q.author_id = $5)))";

/// Questions newest first, with the total that match.
pub(crate) async fn questions(
    tx: &mut TxCx,
    f: &QuestionFilter<'_>,
) -> Result<(Vec<QuestionRow>, i64), ToolError> {
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM catalog.question q
         LEFT JOIN catalog.answer a ON a.tenant_id = q.tenant_id AND a.question_id = q.id
         {QUESTION_WHERE}"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.product_id)
    .bind(f.answered)
    .bind(f.status)
    .bind(f.shopper)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, QuestionRow>(sqlx::AssertSqlSafe(format!(
        "{QUESTION_SELECT} {QUESTION_WHERE}
         ORDER BY q.date_created DESC, q.id DESC LIMIT $6 OFFSET $7"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.product_id)
    .bind(f.answered)
    .bind(f.status)
    .bind(f.shopper)
    .bind(f.limit)
    .bind(f.offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((rows, total))
}

/// A new question (`ask_question`).
pub(crate) struct NewQuestion<'a> {
    pub id: &'a str,
    pub product_group_id: &'a str,
    pub author_id: &'a str,
    pub author_name: &'a str,
    pub text: &'a str,
    pub is_private: bool,
    pub status: &'a str,
    pub at: DateTime<Utc>,
}

/// Inserts the question unless it is there already (the same plan run twice).
pub(crate) async fn insert_question(tx: &mut TxCx, q: &NewQuestion<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO catalog.question (tenant_id, id, product_group_id, author_id, author_name,
           text, is_private, creative_work_status, date_created)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(q.id)
    .bind(q.product_group_id)
    .bind(q.author_id)
    .bind(q.author_name)
    .bind(q.text)
    .bind(q.is_private)
    .bind(q.status)
    .bind(q.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Whether a question has its answer.
pub(crate) async fn is_answered(tx: &mut TxCx, question_id: &str) -> Result<bool, ToolError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM catalog.answer WHERE tenant_id = $1 AND question_id = $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(question_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A `question.profanity_filter` check on some text: the check, the blocked word when the
/// store refuses such text, and whether the text is held for review instead.
pub(crate) fn words_check(
    rules: &QuestionRules,
    text: &str,
    fix_tool: &str,
) -> (Check, Option<String>, bool) {
    let found = model::blocked_word(text, &rules.blocked_words);
    let check = blocked_words_check(
        "question.profanity_filter",
        rules.blocks(),
        found.as_deref(),
        fix_tool,
    );
    let held = found.is_some() && !rules.blocks();
    (check, found.filter(|_| rules.blocks()), held)
}

/// A question as lists show it. The store's answer shows the store as its author.
pub(crate) fn question_json(q: &QuestionRow, time: &acom_core::StoreTime) -> Value {
    let answer = q.answer_id.as_ref().map(|id| {
        let author = match q.answer_author_name.as_deref() {
            Some(name) => json!({"@type": "Person", "name": name}),
            None => json!({"@type": "Organization", "name": "The store"}),
        };
        json!({"id": id, "@type": "Answer", "text": q.answer_text, "author": author,
               "date_created": q.answer_date_created.map(|t| time.format(t))})
    });
    json!({
        "id": q.id, "@type": "Question", "product_id": q.product_group_id,
        "product_name": q.product_name, "text": q.text,
        "author": {"@type": "Person", "name": q.author_name},
        "acom:private": q.is_private, "status": q.creative_work_status,
        "date_created": time.format(q.date_created),
        "answer_count": i32::from(answer.is_some()),
        "accepted_answer": answer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(filter: &str) -> QuestionRules {
        QuestionRules {
            profanity_filter: filter.into(),
            blocked_words: vec!["junk".into()],
            private_allowed: true,
            edit_until_answered: true,
            answered_by: vec!["operator".into()],
        }
    }

    #[test]
    fn blocked_words_are_refused_or_held_as_the_store_says() {
        let (c, word, held) = words_check(&rules("block"), "Is this junk?", "ask_question");
        assert!(!c.passed && !held);
        assert_eq!(word.as_deref(), Some("junk"));
        assert_eq!(c.fix_tool.as_deref(), Some("ask_question"));
        let (c, word, held) = words_check(&rules("hold_for_review"), "Is this junk?", "x");
        assert!(c.passed && held && word.is_none());
        let (c, _, held) = words_check(&rules("block"), "Does it fit over a hoodie?", "x");
        assert!(c.passed && !held);
    }
}
