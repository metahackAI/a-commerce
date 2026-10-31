//! `search_members`.

use super::record_unmask;
use crate::access::{only_fields, parse_time, staff_only};
use crate::wording::{mask_email, mask_name};
use crate::{ACCESS_REASON_REQUIRED, INVALID_FILTER, UNMASK_FORBIDDEN, repo};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::{MemberState, MemberType};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// A field of a member row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemberField {
    /// The customer id.
    Id,
    /// Name (masked unless unmasked).
    Name,
    /// Email (masked unless unmasked).
    Email,
    /// Current tier id.
    TierId,
    /// Account state.
    State,
}

impl MemberField {
    fn word(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Name => "name",
            Self::Email => "email",
            Self::TierId => "tier_id",
            Self::State => "state",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchMembersInput {
    /// Words in the name, email or telephone, or a customer id.
    #[schemars(length(max = 200), example = &"alex")]
    pub query: Option<String>,
    /// Only this member type.
    #[serde(rename = "type")]
    #[schemars(example = &"individual")]
    pub member_type: Option<MemberType>,
    /// Only members in this account state.
    #[schemars(example = &"active")]
    pub state: Option<MemberState>,
    /// Only members of this tier (`list_tiers`).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub tier_id: Option<String>,
    /// Only members of this segment. Segments are not available yet; the call is refused.
    #[schemars(regex(pattern = r"^seg_[a-z0-9_]+$"), example = &"seg_hikers")]
    pub segment_id: Option<String>,
    /// Joined at or after this time (store time).
    #[schemars(example = &"2026-01-01")]
    pub joined_from: Option<String>,
    /// Joined at or before this time (store time).
    #[schemars(example = &"2026-11-06T23:59")]
    pub joined_through: Option<String>,
    /// Show names and emails in full. Needs `access_reason`, which goes to the action log.
    #[schemars(example = false)]
    pub unmask: Option<bool>,
    /// Why the data is unmasked, in a sentence. Required with `unmask: true`.
    #[schemars(length(max = 500), example = &"Customer called about order 20402.")]
    pub access_reason: Option<String>,
    /// Return only these fields of each member. All when left out.
    #[schemars(example = ["id", "tier_id"])]
    pub fields: Option<Vec<MemberField>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchMembers;

impl Operation for SearchMembers {
    const META: OpMeta = OpMeta {
        address: "customer.person.search",
        tool: "search_members",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people",
        summary: "Find members by words, type, state, tier and join date. Names and emails are masked unless unmasked with a reason.",
        consumes: &[],
        produces: &[Slot::many("customer_id")],
        checks: &[],
        errors: &[ACCESS_REASON_REQUIRED, UNMASK_FORBIDDEN, INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchMembersInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchMembers {
    async fn read(&self, cx: &Cx, input: SearchMembersInput) -> Result<Value, ToolError> {
        staff_only(cx)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        if input.segment_id.is_some() {
            return Err(ToolError::new(
                INVALID_FILTER,
                "Segments are not available yet; search without `segment_id`.",
            )
            .detail(json!({"field": "segment_id", "reason": "unsupported"}))
            .next("search_members"));
        }
        let filter = repo::MemberFilter {
            query: input
                .query
                .map(|q| q.trim().to_owned())
                .filter(|q| !q.is_empty()),
            member_type: input.member_type.map(|t| t.as_str().to_owned()),
            member_state: input.state.map(|s| s.as_str().to_owned()),
            tier_id: input.tier_id,
            joined_from: parse_time(cx, "joined_from", input.joined_from.as_deref())?,
            joined_through: parse_time(cx, "joined_through", input.joined_through.as_deref())?,
        };
        let unmasked = record_unmask(
            cx,
            "search_members",
            input.unmask.unwrap_or(false),
            input.access_reason.as_deref(),
            None,
        )
        .await?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (total, rows) = repo::search_members(&mut tx, &filter, page.limit, page.offset).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let fields: Vec<String> = input
            .fields
            .unwrap_or_default()
            .iter()
            .map(|f| f.word().to_owned())
            .collect();
        let items: Vec<Value> = rows
            .iter()
            .map(|r| {
                let (name, email) = if unmasked {
                    (r.name.clone(), r.email.clone())
                } else {
                    (mask_name(&r.name), r.email.as_deref().map(mask_email))
                };
                only_fields(
                    json!({"id": r.id, "name": name, "email": email, "tier_id": r.tier_id,
                           "state": r.member_state}),
                    &fields,
                )
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
    fn input_is_closed_with_word_types() {
        let search = closed_schema_for::<SearchMembersInput>();
        assert_eq!(violations(&search), Vec::<String>::new(), "{search:#}");
        let words = search["properties"]["type"]["enum"].as_array().unwrap();
        assert!(words.contains(&json!("individual")) && words.contains(&json!("business")));
        assert_eq!(SearchMembers::META.audience, Audience::Operator);
    }
}
