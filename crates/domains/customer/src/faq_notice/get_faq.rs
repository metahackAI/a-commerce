//! `get_faq`: one FAQ entry with its answer.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{faq_not_found, question};
use crate::FAQ_NOT_FOUND;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetFaqInput {
    /// The FAQ entry (`list_faqs`).
    #[schemars(regex(pattern = r"^faq_[a-z0-9_]+$"), example = &"faq_returns")]
    pub faq_id: String,
}

pub struct GetFaq;

impl Operation for GetFaq {
    const META: OpMeta = OpMeta {
        address: "customer.faq_notice.get_faq",
        tool: "get_faq",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/faqs/{faq_id}",
        summary: "One FAQ entry: the question and the store's accepted answer.",
        consumes: &[Slot::one("faq_id")],
        produces: &[],
        checks: &[],
        errors: &[FAQ_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetFaqInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetFaq {
    async fn read(&self, cx: &Cx, input: GetFaqInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let q = question(&mut tx, &input.faq_id)
            .await?
            .ok_or_else(|| faq_not_found(&input.faq_id))?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(
            json!({"id": q.id, "name": q.name, "accepted_answer": q.accepted_answer,
                  "category": q.category}),
        )
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<GetFaqInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(GetFaq::META.path, "/v1/customer/faqs/{faq_id}");
    }
}
