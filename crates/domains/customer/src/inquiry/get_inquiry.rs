//! `get_inquiry`: one inquiry with its whole thread.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{comment_json, thread, visible_inquiry};
use crate::INQUIRY_NOT_FOUND;
use acom_contract::fields;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetInquiryInput {
    /// The inquiry (`search_inquiries`).
    #[schemars(regex(pattern = r"^inq_[a-z0-9_]+$"), example = &"inq_771")]
    pub inquiry_id: String,
    /// Top-level fields to return. All when left out.
    #[schemars(example = ["status", "thread"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetInquiry;

impl Operation for GetInquiry {
    const META: OpMeta = OpMeta {
        address: "customer.inquiry.get",
        tool: "get_inquiry",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/inquiries/{inquiry_id}",
        summary: "One customer inquiry: its type, state, order, assigned operator and the whole thread of messages. A shopper sees only their own.",
        consumes: &[Slot::one("inquiry_id")],
        produces: &[],
        checks: &[],
        errors: &[INQUIRY_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetInquiryInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetInquiry {
    async fn read(&self, cx: &Cx, input: GetInquiryInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = visible_inquiry(cx, &mut tx, &input.inquiry_id).await?;
        let comments = thread(&mut tx, &r.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let out = json!({
            "id": r.id, "inquiry_type": r.inquiry_type, "status": r.inquiry_status,
            "customer_id": r.customer_id, "order_id": r.order_id,
            "admin_user_id": r.admin_user_id,
            "created_at": cx.time.format(r.date_received),
            "resolution": r.resolution,
            "thread": comments.iter().map(|c| comment_json(cx, c)).collect::<Vec<_>>(),
        });
        fields::keep(out, &input.fields.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<GetInquiryInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(GetInquiry::META.path, "/v1/customer/inquiries/{inquiry_id}");
    }
}
