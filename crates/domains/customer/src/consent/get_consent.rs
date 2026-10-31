//! `get_consent`: a member's current answer for every consent type.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::current;
use crate::access::customer_for;
use crate::person::customer_not_found;
use crate::{CUSTOMER_NOT_FOUND, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetConsentInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"cus_alex")]
    pub customer_id: String,
}

pub struct GetConsent;

impl Operation for GetConsent {
    const META: OpMeta = OpMeta {
        address: "customer.consent.get",
        tool: "get_consent",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/{customer_id}/consent",
        summary: "What a member agreed to: the latest record of each consent (terms, privacy, marketing by email, SMS, push, in app) and when it was given.",
        consumes: &[Slot::one("customer_id")],
        produces: &[],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetConsentInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetConsent {
    async fn read(&self, cx: &Cx, input: GetConsentInput) -> Result<Value, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if repo::person(&mut tx, &customer_id).await?.is_none() {
            return Err(customer_not_found(&customer_id));
        }
        let mut rows = current(&mut tx, &customer_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        rows.sort_by(|a, b| b.recorded_at.cmp(&a.recorded_at).then(a.id.cmp(&b.id)));
        let items: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({"id": r.id, "consent_type": r.consent_type, "granted": r.granted,
                       "at": cx.time.format(r.recorded_at)})
            })
            .collect();
        Ok(json!({"items": items, "total": items.len()}))
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<GetConsentInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            GetConsent::META.path,
            "/v1/customer/people/{customer_id}/consent"
        );
    }
}
