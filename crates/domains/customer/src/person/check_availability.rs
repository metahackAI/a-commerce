//! `check_availability`: whether an email or sign-in id can be used to sign up. Never says
//! whether it belonged to a member who left: that is `reserved`, like any other held id.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{held_identifier, identifier_taken};
use crate::{INVALID_FORMAT, secret, wording};

/// Which identifier is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierField {
    /// An email address.
    Email,
    /// A sign-in id.
    LoginId,
}

impl IdentifierField {
    fn word(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::LoginId => "login_id",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckAvailabilityInput {
    /// email or login_id.
    #[schemars(example = &"email")]
    pub field: IdentifierField,
    /// The email or sign-in id to check.
    #[schemars(length(min = 1, max = 200), example = &"alex@example.com")]
    pub value: String,
}

pub struct CheckAvailability;

impl Operation for CheckAvailability {
    const META: OpMeta = OpMeta {
        address: "customer.person.check_availability",
        tool: "check_availability",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/availability",
        summary: "Whether an email or sign-in id is free to sign up with, before registering. Anyone may ask; the answer is available, taken, reserved or invalid_format.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[INVALID_FORMAT],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = CheckAvailabilityInput;
    type Output = Value;
}

impl ReadOp<Cx> for CheckAvailability {
    async fn read(&self, cx: &Cx, input: CheckAvailabilityInput) -> Result<Value, ToolError> {
        let field = input.field.word();
        let value = input.value.trim().to_owned();
        let well_formed = match input.field {
            IdentifierField::Email => wording::looks_like_email(&value),
            IdentifierField::LoginId => wording::looks_like_login_id(&value),
        };
        if !well_formed {
            return Err(ToolError::new(
                INVALID_FORMAT,
                format!(
                    "`{value}` is not a well-formed {}.",
                    field.replace('_', " ")
                ),
            )
            .detail(json!({"field": "value", "reason": "format", "checked": field}))
            .next("check_availability"));
        }
        let normalized = secret::normalize(&value);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let taken = identifier_taken(&mut tx, field, &normalized).await?;
        let held = !taken && held_identifier(&mut tx, &normalized, cx.clock.now()).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let reason = if taken {
            Some("taken")
        } else if held {
            Some("reserved")
        } else {
            None
        };
        Ok(
            json!({"field": field, "value": value, "available": reason.is_none(),
                  "reason": reason}),
        )
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_with_field_words() {
        let s = closed_schema_for::<CheckAvailabilityInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            s["properties"]["field"]["enum"],
            json!(["email", "login_id"])
        );
        assert_eq!(CheckAvailability::META.audience, Audience::Both);
    }
}
