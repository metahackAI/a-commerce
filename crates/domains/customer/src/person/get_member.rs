//! `get_member`.

use super::{customer_not_found, record_unmask};
use crate::access::{only_fields, staff_only};
use crate::wording::mask_name;
use crate::{ACCESS_REASON_REQUIRED, CUSTOMER_NOT_FOUND, UNMASK_FORBIDDEN, repo};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetMemberInput {
    /// The member.
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_alex")]
    pub customer_id: String,
    /// Show the name in full. Needs `access_reason`, which goes to the action log.
    #[schemars(example = false)]
    pub unmask: Option<bool>,
    /// Why the data is unmasked, in a sentence. Required with `unmask: true`.
    #[schemars(length(max = 500), example = &"Checking the return request of order 20402.")]
    pub access_reason: Option<String>,
    /// Top-level fields to return. All when left out.
    #[schemars(example = ["member_of", "state"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetMember;

impl Operation for GetMember {
    const META: OpMeta = OpMeta {
        address: "customer.person.get",
        tool: "get_member",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/{customer_id}",
        summary: "One member: type, tier membership, account state and consent summary. The name is masked unless unmasked with a reason.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::one("customer_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND, ACCESS_REASON_REQUIRED, UNMASK_FORBIDDEN],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetMemberInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetMember {
    async fn read(&self, cx: &Cx, input: GetMemberInput) -> Result<Value, ToolError> {
        staff_only(cx)?;
        let unmasked = record_unmask(
            cx,
            "get_member",
            input.unmask.unwrap_or(false),
            input.access_reason.as_deref(),
            Some(&input.customer_id),
        )
        .await?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let person = repo::person(&mut tx, &input.customer_id)
            .await?
            .ok_or_else(|| customer_not_found(&input.customer_id))?;
        let membership = repo::memberships(&mut tx, &person.id)
            .await?
            .into_iter()
            .next();
        tx.commit().await.map_err(ToolError::from)?;
        let name = if unmasked {
            person.name.clone()
        } else {
            mask_name(&person.name)
        };
        let member_of = membership.map(|m| {
            json!({"program_id": m.member_program_id, "tier_id": m.tier_id,
                   "membership_number": m.membership_number})
        });
        // Consent records and business verification are not stored yet.
        let member = json!({"id": person.id, "name": name, "type": person.member_type,
            "member_of": member_of, "state": person.member_state,
            "acom:business_verification": null, "consent": null});
        Ok(only_fields(member, &input.fields.unwrap_or_default()))
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_the_path_is_the_spec_path() {
        let s = closed_schema_for::<GetMemberInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(GetMember::META.path, "/v1/customer/people/{customer_id}");
    }
}
