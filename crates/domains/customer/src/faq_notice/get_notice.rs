//! `get_notice`: one store notice.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{notice, notice_not_found, sees_drafts, visible_to_shoppers};
use crate::NOTICE_NOT_FOUND;
use crate::access::day;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetNoticeInput {
    /// The notice (`list_notices`).
    #[schemars(regex(pattern = r"^ntc_[a-z0-9_]+$"), example = &"ntc_holiday")]
    pub notice_id: String,
}

pub struct GetNotice;

impl Operation for GetNotice {
    const META: OpMeta = OpMeta {
        address: "customer.faq_notice.get_notice",
        tool: "get_notice",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/notices/{notice_id}",
        summary: "One store notice with its text, status and publication date.",
        consumes: &[Slot::one("notice_id")],
        produces: &[],
        checks: &[],
        errors: &[NOTICE_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetNoticeInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetNotice {
    async fn read(&self, cx: &Cx, input: GetNoticeInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let n = notice(&mut tx, &input.notice_id)
            .await?
            .filter(|n| sees_drafts(cx) || visible_to_shoppers(n, cx.clock.now()))
            .ok_or_else(|| notice_not_found(&input.notice_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(
            json!({"id": n.id, "headline": n.headline, "article_body": n.article_body,
                  "category": n.category,
                  "date_published": n.date_published.map(|d| day(cx.time, d)),
                  "status": n.notice_status, "pinned": n.is_pinned}),
        )
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<GetNoticeInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(GetNotice::META.consumes, &[Slot::one("notice_id")]);
    }
}
