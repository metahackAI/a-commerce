//! `list_tier_history`: every tier change of a member, newest first, with why and by whom.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::access::{customer_for, db, parse_time};
use crate::person::customer_not_found;
use crate::{CUSTOMER_NOT_FOUND, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTierHistoryInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"cus_jamie")]
    pub customer_id: String,
    /// Only this program. Every program when left out.
    #[schemars(regex(pattern = r"^prg_[a-z0-9_]+$"), example = &"prg_rewards")]
    pub program_id: Option<String>,
    /// Changes at or after this time (store time).
    #[schemars(example = &"2026-01-01")]
    pub at_from: Option<String>,
    /// Changes at or before this time (store time).
    #[schemars(example = &"2026-11-06T23:59")]
    pub at_through: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListTierHistory;

impl Operation for ListTierHistory {
    const META: OpMeta = OpMeta {
        address: "customer.membership.list_tier_history",
        tool: "list_tier_history",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/{customer_id}/tier-history",
        summary: "A member's tier changes, newest first: from and to, the cause (evaluation, manual, grace, withdrawal), the result it was based on, and who changed it and why.",
        consumes: &[Slot::one("customer_id")],
        produces: &[],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListTierHistoryInput;
    type Output = Value;
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    member_program_id: String,
    from_tier_id: Option<String>,
    to_tier_id: Option<String>,
    cause: String,
    evaluation_id: Option<String>,
    snapshot: Option<Value>,
    agent_id: String,
    reason: Option<String>,
    expires: Option<DateTime<Utc>>,
    changed_at: DateTime<Utc>,
}

impl ReadOp<Cx> for ListTierHistory {
    async fn read(&self, cx: &Cx, input: ListTierHistoryInput) -> Result<Value, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let from = parse_time(cx, "at_from", input.at_from.as_deref())?;
        let through = parse_time(cx, "at_through", input.at_through.as_deref())?;
        let filter = "tenant_id = $1 AND customer_id = $2
            AND ($3::text IS NULL OR member_program_id = $3)
            AND ($4::timestamptz IS NULL OR changed_at >= $4)
            AND ($5::timestamptz IS NULL OR changed_at <= $5)";
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if repo::person(&mut tx, &customer_id).await?.is_none() {
            return Err(customer_not_found(&customer_id));
        }
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM customer.program_membership_event WHERE {filter}"
        )))
        .bind(tx.tenant.as_str())
        .bind(&customer_id)
        .bind(input.program_id.as_deref())
        .bind(from)
        .bind(through)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!(
            "SELECT member_program_id, from_tier_id, to_tier_id, cause, evaluation_id, snapshot,
                    agent_id, reason, expires, changed_at
             FROM customer.program_membership_event WHERE {filter}
             ORDER BY changed_at DESC, position DESC LIMIT $6 OFFSET $7"
        )))
        .bind(tx.tenant.as_str())
        .bind(&customer_id)
        .bind(input.program_id.as_deref())
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
                let mut v = json!({"at": cx.time.format(r.changed_at),
                    "program_id": r.member_program_id,
                    "from_tier_id": r.from_tier_id, "to_tier_id": r.to_tier_id,
                    "cause": r.cause, "evaluation_id": r.evaluation_id,
                    "snapshot": r.snapshot, "actor": r.agent_id, "reason": r.reason});
                if let Some(e) = r.expires {
                    v["expires_at"] = json!(cx.time.format(e));
                }
                v
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
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<ListTierHistoryInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(ListTierHistory::META.audience, Audience::Both);
    }
}
