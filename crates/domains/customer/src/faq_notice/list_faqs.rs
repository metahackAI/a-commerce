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
        let query = input
            .query
            .map(|q| q.trim().to_owned())
            .filter(|q| !q.is_empty());
        let filter = "tenant_id = $1 AND ($2::text IS NULL OR category = $2)
            AND ($3::text IS NULL OR name ILIKE '%' || $3 || '%'
                 OR accepted_answer ILIKE '%' || $3 || '%')";
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
             ORDER BY category, position, id LIMIT $4 OFFSET $5"
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
    fn input_is_closed_and_anyone_may_read() {
        let s = closed_schema_for::<ListFaqsInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(ListFaqs::META.audience, Audience::Both);
        assert_eq!(ListFaqs::META.produces, &[Slot::many("faq_id")]);
    }
}
