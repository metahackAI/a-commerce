//! `get_my_profile`.

use super::{address_json, memory_json};
use crate::access::{not_signed_in, signed_in_shopper};
use crate::{NOT_SIGNED_IN, repo};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError, fields,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// A top-level field of the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileField {
    /// The shopper's id.
    Id,
    /// Display name.
    Name,
    /// Current tier id.
    TierId,
    /// Known sizes.
    Sizes,
    /// Remembered preferences.
    Preferences,
    /// Saved addresses.
    Addresses,
    /// The saved card, last four digits only.
    Card,
}

impl ProfileField {
    fn word(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Name => "name",
            Self::TierId => "tier_id",
            Self::Sizes => "sizes",
            Self::Preferences => "preferences",
            Self::Addresses => "addresses",
            Self::Card => "card",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetMyProfileInput {
    /// Return only these fields. All when left out.
    #[schemars(example = ["sizes", "preferences"])]
    pub fields: Option<Vec<ProfileField>>,
}

pub struct GetMyProfile;

impl Operation for GetMyProfile {
    const META: OpMeta = OpMeta {
        address: "customer.person.get_my_profile",
        tool: "get_my_profile",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/me",
        summary: "The signed-in shopper's own profile: sizes, remembered preferences, saved addresses, tier, and the saved card (last four digits only).",
        consumes: &[],
        produces: &[
            Slot::one("customer_id"),
            Slot::one("tier_id"),
            Slot::many("address_id"),
            Slot::many("memory_id"),
        ],
        checks: &[],
        errors: &[NOT_SIGNED_IN],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetMyProfileInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetMyProfile {
    async fn read(&self, cx: &Cx, input: GetMyProfileInput) -> Result<Value, ToolError> {
        let me = signed_in_shopper(cx)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let person = repo::person(&mut tx, &me)
            .await?
            .ok_or_else(not_signed_in)?;
        let tier = repo::memberships(&mut tx, &me)
            .await?
            .into_iter()
            .next()
            .map(|m| m.tier_id);
        let memories = repo::all_preferences(&mut tx, &me).await?;
        let (_, addresses) = repo::addresses(&mut tx, &me, i64::from(Page::MAX_LIMIT), 0).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let size = |k: &str| person.sizes.get(k).cloned().unwrap_or(Value::Null);
        let card = person
            .card_last4
            .as_ref()
            .map(|l| json!({"label": format!("Card ending {l}"), "last4": l}));
        let profile = json!({
            "id": person.id, "name": person.name, "tier_id": tier,
            "sizes": {"tops": size("tops"), "bottoms": size("bottoms"), "shoes": size("shoes")},
            "preferences": memories.iter().map(memory_json).collect::<Vec<_>>(),
            "addresses": addresses.iter().map(address_json).collect::<Vec<_>>(),
            "card": card,
        });
        let fields: Vec<String> = input
            .fields
            .unwrap_or_default()
            .iter()
            .map(|f| f.word().to_owned())
            .collect();
        Ok(fields::pick(profile, &fields))
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_the_path_is_the_spec_path() {
        let s = closed_schema_for::<GetMyProfileInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(GetMyProfile::META.path, "/v1/customer/people/me");
    }

    #[test]
    fn profile_fields_are_the_spec_words() {
        let all = [
            ProfileField::Id,
            ProfileField::Name,
            ProfileField::TierId,
            ProfileField::Sizes,
            ProfileField::Preferences,
            ProfileField::Addresses,
            ProfileField::Card,
        ];
        let words: Vec<&str> = all.iter().map(|f| f.word()).collect();
        assert_eq!(
            words,
            [
                "id",
                "name",
                "tier_id",
                "sizes",
                "preferences",
                "addresses",
                "card"
            ]
        );
    }
}
