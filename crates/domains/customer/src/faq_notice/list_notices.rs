//! `list_notices`: store notices, pinned first, newest first.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{NOTICE_COLUMNS, NoticeRow, notice_item, sees_drafts};
use crate::access::{db, parse_time};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListNoticesInput {
    /// Only this notice section (a word the store chooses, for example `shipping`).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"shipping")]
    pub category: Option<String>,
    /// Published at or after this time (store time).
    #[schemars(example = &"2026-11-01")]
    pub published_from: Option<String>,
    /// Published at or before this time (store time).
    #[schemars(example = &"2026-11-30T23:59")]
    pub published_through: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListNotices;

impl Operation for ListNotices {
    const META: OpMeta = OpMeta {
        address: "customer.faq_notice.list_notices",
        tool: "list_notices",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/notices",
        summary: "Store notices, pinned first and then newest first. Shoppers see published notices; operators also see drafts.",
        consumes: &[],
        produces: &[Slot::many("notice_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListNoticesInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListNotices {
    async fn read(&self, cx: &Cx, input: ListNoticesInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let from = parse_time(cx, "published_from", input.published_from.as_deref())?;
        let through = parse_time(cx, "published_through", input.published_through.as_deref())?;
        let drafts = sees_drafts(cx);
        let filter = "tenant_id = $1 AND ($2::text IS NULL OR category = $2)
            AND ($3::timestamptz IS NULL OR date_published >= $3)
            AND ($4::timestamptz IS NULL OR date_published <= $4)
            AND ($5 OR (notice_status = 'published' AND date_published <= $6))";
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM customer.news_article WHERE {filter}"
        )))
        .bind(tx.tenant.as_str())
        .bind(input.category.as_deref())
        .bind(from)
        .bind(through)
        .bind(drafts)
        .bind(cx.clock.now())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows = sqlx::query_as::<_, NoticeRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {NOTICE_COLUMNS} FROM customer.news_article WHERE {filter}
             ORDER BY is_pinned DESC, date_published DESC NULLS FIRST, id
             LIMIT $7 OFFSET $8"
        )))
        .bind(tx.tenant.as_str())
        .bind(input.category.as_deref())
        .bind(from)
        .bind(through)
        .bind(drafts)
        .bind(cx.clock.now())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|n| notice_item(cx, n)).collect();
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
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<ListNoticesInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(ListNotices::META.produces, &[Slot::many("notice_id")]);
    }
}
