//! `list_faqs`: the store's FAQ, by section and words.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::QuestionRow;
use crate::access::db;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListFaqsInput {
    /// Only this FAQ section (a word the store chooses, for example `returns`, `shipping`).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"returns")]
    pub category: Option<String>,
    /// Words in the question or the answer.
    #[schemars(length(max = 200), example = &"return")]
    pub query: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// The words of a query worth matching: lower case, three letters or more, without the words
/// every question has ("how", "do", "the").
fn words(query: &str) -> Vec<String> {
    const COMMON: &[&str] = &[
        "the",
        "and",
        "for",
        "you",
        "your",
        "how",
        "what",
        "when",
        "where",
        "which",
        "who",
        "why",
        "can",
        "does",
        "did",
        "have",
        "has",
        "are",
        "was",
        "with",
        "about",
        "that",
        "this",
        "something",
        "anything",
        "there",
        "from",
        "will",
        "would",
        "could",
        "should",
        "much",
        "many",
    ];
    let mut out: Vec<String> = Vec::new();
    for w in query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3 && !COMMON.contains(&w.as_str()))
    {
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

pub struct ListFaqs;

impl Operation for ListFaqs {
    const META: OpMeta = OpMeta {
        address: "customer.faq_notice.list_faqs",
        tool: "list_faqs",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/faqs",
        summary: "The store's frequently asked questions, by section or words in the question or answer. Anyone may read them.",
        consumes: &[],
        produces: &[Slot::many("faq_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListFaqsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListFaqs {
    async fn read(&self, cx: &Cx, input: ListFaqsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        // Words, not a phrase: "how long to return" finds "How long do I have to return?".
        let query = input.query.as_deref().map(words).filter(|w| !w.is_empty());
        let hits = "(SELECT count(*) FROM unnest($3::text[]) w
                     WHERE name ILIKE '%' || w || '%' OR accepted_answer ILIKE '%' || w || '%')";
        let filter = format!(
            "tenant_id = $1 AND ($2::text IS NULL OR category = $2)
             AND ($3::text[] IS NULL OR {hits} > 0)"
        );
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM customer.question WHERE {filter}"
        )))
        .bind(tx.tenant.as_str())
        .bind(input.category.as_deref())
        .bind(query.as_deref())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows = sqlx::query_as::<_, QuestionRow>(sqlx::AssertSqlSafe(format!(
            "SELECT id, name, accepted_answer, category FROM customer.question WHERE {filter}
             ORDER BY (CASE WHEN $3::text[] IS NULL THEN 0 ELSE {hits} END) DESC,
                      category, position, id LIMIT $4 OFFSET $5"
        )))
        .bind(tx.tenant.as_str())
        .bind(input.category.as_deref())
        .bind(query.as_deref())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|q| json!({"id": q.id, "name": q.name, "category": q.category}))
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn a_query_is_its_words_not_a_phrase() {
        assert_eq!(
            words("How long do I have to return something?"),
            ["long", "return"]
        );
        assert_eq!(words("return window"), ["return", "window"]);
        assert_eq!(words("  "), Vec::<String>::new());
    }

    #[test]
    fn input_is_closed_and_anyone_may_read() {
        let s = closed_schema_for::<ListFaqsInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(ListFaqs::META.audience, Audience::Both);
        assert_eq!(ListFaqs::META.produces, &[Slot::many("faq_id")]);
    }
}
