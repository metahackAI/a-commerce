//! `search_inquiries`: customer service inquiries by type, state, assignee, order or customer.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::{InquiryStatus, InquiryType};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{INQUIRY_COLUMNS, InquiryRow};
use crate::INVALID_FILTER;
use crate::access::{db, parse_time, staff_only};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchInquiriesInput {
    /// Only this kind: order_status; return; product_question; account; callback; other.
    #[schemars(example = &"return")]
    pub inquiry_type: Option<InquiryType>,
    /// Only inquiries in this state: received; in_progress; resolved.
    #[schemars(example = &"received")]
    pub status: Option<InquiryStatus>,
    /// Only inquiries this operator handles.
    #[schemars(regex(pattern = r"^adm_[a-z0-9_]+$"), example = &"adm_sj")]
    pub admin_user_id: Option<String>,
    /// Only inquiries about this order.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20402")]
    pub order_id: Option<String>,
    /// Only inquiries of this customer.
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_alex")]
    pub customer_id: Option<String>,
    /// Received at or after this time (store time).
    #[schemars(example = &"2026-11-01")]
    pub created_from: Option<String>,
    /// Received at or before this time (store time).
    #[schemars(example = &"2026-11-06T23:59")]
    pub created_through: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchInquiries;

impl Operation for SearchInquiries {
    const META: OpMeta = OpMeta {
        address: "customer.inquiry.search",
        tool: "search_inquiries",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/inquiries",
        summary: "Find customer service inquiries by type, state, assigned operator, order, customer or date, oldest open first. Customer details stay masked.",
        consumes: &[],
        produces: &[Slot::many("inquiry_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchInquiriesInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchInquiries {
    async fn read(&self, cx: &Cx, input: SearchInquiriesInput) -> Result<Value, ToolError> {
        staff_only(cx)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let from = parse_time(cx, "created_from", input.created_from.as_deref())?;
        let through = parse_time(cx, "created_through", input.created_through.as_deref())?;
        if let (Some(f), Some(t)) = (from, through)
            && f > t
        {
            return Err(ToolError::new(
                INVALID_FILTER,
                "`created_from` is after `created_through`.",
            )
            .detail(json!({"field": "created_from", "reason": "range"}))
            .next("search_inquiries"));
        }
        let filter = "tenant_id = $1 AND ($2::text IS NULL OR inquiry_type = $2)
            AND ($3::text IS NULL OR inquiry_status = $3)
            AND ($4::text IS NULL OR admin_user_id = $4)
            AND ($5::text IS NULL OR order_id = $5)
            AND ($6::text IS NULL OR customer_id = $6)
            AND ($7::timestamptz IS NULL OR date_received >= $7)
            AND ($8::timestamptz IS NULL OR date_received <= $8)";
        let kind = input.inquiry_type.map(InquiryType::as_str);
        let status = input.status.map(InquiryStatus::as_str);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM customer.customer_inquiry WHERE {filter}"
        )))
        .bind(tx.tenant.as_str())
        .bind(kind)
        .bind(status)
        .bind(input.admin_user_id.as_deref())
        .bind(input.order_id.as_deref())
        .bind(input.customer_id.as_deref())
        .bind(from)
        .bind(through)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        // Open inquiries first, the oldest on top; resolved ones after, newest first.
        let rows = sqlx::query_as::<_, InquiryRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {INQUIRY_COLUMNS} FROM customer.customer_inquiry WHERE {filter}
             ORDER BY inquiry_status = 'resolved',
                      CASE WHEN inquiry_status = 'resolved' THEN NULL ELSE date_received END,
                      date_received DESC, id
             LIMIT $9 OFFSET $10"
        )))
        .bind(tx.tenant.as_str())
        .bind(kind)
        .bind(status)
        .bind(input.admin_user_id.as_deref())
        .bind(input.order_id.as_deref())
        .bind(input.customer_id.as_deref())
        .bind(from)
        .bind(through)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({"id": r.id, "inquiry_type": r.inquiry_type, "status": r.inquiry_status,
                       "customer_id": r.customer_id, "order_id": r.order_id,
                       "admin_user_id": r.admin_user_id,
                       "created_at": cx.time.format(r.date_received)})
            })
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
    fn input_is_closed_with_word_filters() {
        let s = closed_schema_for::<SearchInquiriesInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            s["properties"]["status"]["enum"],
            json!(["received", "in_progress", "resolved"])
        );
    }
}
