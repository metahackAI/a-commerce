//! `get_my_profile`, `get_return_history`, `search_members`, `get_member`.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::enums::{ActionStatus, MemberState, MemberType};
use acom_store::{Cx, Page};
use chrono::Duration;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::access::{
    not_signed_in, only_fields, parse_time, policy_number, signed_in_shopper, staff_only,
};
use crate::service::providers;
use crate::wording::{mask_email, mask_name};
use crate::{
    ACCESS_REASON_REQUIRED, CUSTOMER_NOT_FOUND, INVALID_FILTER, NOT_SIGNED_IN, UNMASK_FORBIDDEN,
    repo,
};

pub(crate) fn customer_not_found(id: &str) -> ToolError {
    ToolError::new(CUSTOMER_NOT_FOUND, format!("There is no customer {id}."))
        .detail(json!({"entity": "customer", "id": id}))
        .next("search_members")
}

/// A saved address as profiles and lists show it.
pub(crate) fn address_json(a: &repo::AddressRow) -> Value {
    json!({"id": a.id, "label": a.label, "street_address": a.street_address,
           "address_locality": a.address_locality, "address_region": a.address_region,
           "postal_code": a.postal_code, "address_country": a.address_country,
           "is_primary": a.is_primary})
}

/// A memory as the profile shows it.
fn memory_json(p: &repo::PreferenceRow) -> Value {
    json!({"memory_id": p.id, "key": p.preference_key, "value": p.value,
           "text": p.description, "source": p.source})
}

// ── get_my_profile ──────────────────────────────────────────────────────────────────────

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
        Ok(only_fields(profile, &fields))
    }
}

// ── get_return_history ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetReturnHistoryInput {
    /// The customer (`search_return_requests` gives it).
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_4431")]
    pub customer_id: String,
    /// Days back from today (store clock), 1–365. Default: the return history flag's days (90).
    #[schemars(range(min = 1, max = 365), example = 90)]
    pub days: Option<u32>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct GetReturnHistory;

impl Operation for GetReturnHistory {
    const META: OpMeta = OpMeta {
        address: "customer.person.get_return_history",
        tool: "get_return_history",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/{customer_id}/returns",
        summary: "A customer's return requests in the last days, with a summary that flags frequent returners (for example 4 returns in 90 days).",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::many("claim_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetReturnHistoryInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetReturnHistory {
    async fn read(&self, cx: &Cx, input: GetReturnHistoryInput) -> Result<Value, ToolError> {
        staff_only(cx)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let flag_days = policy_number(
            cx,
            "fulfillment.return_suggestions.flag_return_history.days",
        )?;
        let threshold = policy_number(
            cx,
            "fulfillment.return_suggestions.flag_return_history.returns",
        )?;
        let days = input.days.map_or(flag_days, i64::from);
        if !(1..=365).contains(&days) {
            return Err(ToolError::invalid_input("`days` must be 1–365.")
                .detail(json!({"field": "days", "reason": "range"}))
                .next("get_return_history"));
        }
        let reader = providers::return_claims().ok_or_else(|| {
            ToolError::new(
                ErrorCode::UPSTREAM_UNAVAILABLE,
                "Return history is not connected to fulfillment yet.",
            )
            .detail(json!({"upstream": "fulfillment", "retryable": false}))
        })?;
        let since = cx.clock.now() - Duration::days(days);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if repo::person(&mut tx, &input.customer_id).await?.is_none() {
            return Err(customer_not_found(&input.customer_id));
        }
        let claims: Vec<_> = reader(&mut tx, &input.customer_id, since)
            .await?
            .into_iter()
            .filter(|c| c.status != "withdrawn")
            .collect();
        tx.commit().await.map_err(ToolError::from)?;
        let total = i64::try_from(claims.len()).unwrap_or(i64::MAX);
        let start = usize::try_from(page.offset).unwrap_or(usize::MAX);
        let take = usize::try_from(page.limit).unwrap_or(0);
        let items: Vec<Value> = claims
            .iter()
            .skip(start)
            .take(take)
            .map(|c| {
                json!({"id": c.id, "claim_type": c.claim_type, "order_id": c.order_id,
                       "order_number": (!c.order_number.is_empty()).then_some(&c.order_number),
                       "item": (!c.item.is_empty()).then_some(&c.item),
                       "amount": c.amount, "reason": c.reason, "status": c.status,
                       "requested_at": cx.time.format(c.requested_at)})
            })
            .collect();
        let mut out = json!({"items": items, "total": total,
            "summary": {"count": total, "days": days, "threshold": threshold,
                        "flagged": total >= threshold}});
        if let Some(c) = page.next_cursor(items.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

// ── search_members ──────────────────────────────────────────────────────────────────────

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

/// Checks an unmask request and writes it to the action log (privacy.access_reason_logging).
async fn record_unmask(
    cx: &Cx,
    tool: &'static str,
    unmask: bool,
    reason: Option<&str>,
    object: Option<&str>,
) -> Result<bool, ToolError> {
    if !unmask {
        return Ok(false);
    }
    let reason = reason
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| {
            ToolError::new(
                ACCESS_REASON_REQUIRED,
                "Say why you need the unmasked data; the reason goes to the action log.",
            )
            .detail(json!({"field": "access_reason", "reason": "missing"}))
            .next(tool)
        })?;
    use acom_core::enums::ActorKind;
    if !matches!(
        cx.principal.kind,
        ActorKind::Operator | ActorKind::AgentOnBehalf
    ) {
        return Err(ToolError::new(
            UNMASK_FORBIDDEN,
            "Only an operator may unmask customer data.",
        )
        .detail(json!({"reason": "role"}))
        .next("list_roles"));
    }
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    acom_changeset::log_action(
        &mut tx,
        acom_changeset::LogEntry {
            object: object.map(|id| ("person", id)),
            result: Some(json!({"unmasked": true, "access_reason": reason})),
            ..acom_changeset::LogEntry::new(ActionType::SEARCH.0, ActionStatus::Completed, tool)
        },
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(true)
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

// ── get_member ──────────────────────────────────────────────────────────────────────────

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
    fn inputs_are_closed_and_described() {
        for s in [
            closed_schema_for::<GetMyProfileInput>(),
            closed_schema_for::<GetReturnHistoryInput>(),
            closed_schema_for::<SearchMembersInput>(),
            closed_schema_for::<GetMemberInput>(),
        ] {
            assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        }
        let search = closed_schema_for::<SearchMembersInput>();
        let words = search["properties"]["type"]["enum"].as_array().unwrap();
        assert!(words.contains(&json!("individual")) && words.contains(&json!("business")));
    }

    #[test]
    fn metas_follow_the_spec_paths() {
        assert_eq!(GetMyProfile::META.path, "/v1/customer/people/me");
        assert_eq!(GetMember::META.path, "/v1/customer/people/{customer_id}");
        assert_eq!(
            GetReturnHistory::META.path,
            "/v1/customer/people/{customer_id}/returns"
        );
        assert_eq!(SearchMembers::META.audience, Audience::Operator);
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

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reads_people_from_the_database() {
        let mut tx = crate::testdb::tx("cus_jamie").await;
        let jamie = repo::person(&mut tx, "cus_jamie").await.unwrap().unwrap();
        assert_eq!(jamie.card_last4.as_deref(), Some("4821"));
        let (total, rows) = repo::search_members(
            &mut tx,
            &repo::MemberFilter {
                query: Some("alex".into()),
                ..Default::default()
            },
            20,
            0,
        )
        .await
        .unwrap();
        assert!(total >= 1);
        assert!(rows.iter().any(|r| r.id == "cus_alex"));
    }
}
