//! `list_tiers`, `get_tier`, `get_member_tier`. Tiers are data the store defines
//! (`customer.member_program_tier`), never code.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::access::{customer_for, only_fields, policy};
use crate::person::customer_not_found;
use crate::progress::{
    cents, next_evaluation, progress_entry, requirement_combine, requirement_list, retention,
    tally, threshold_money, window_start,
};
use crate::service::providers::{self, MemberOrder};
use crate::{CUSTOMER_NOT_FOUND, PROGRAM_NOT_FOUND, TIER_NOT_FOUND, repo};

fn tier_not_found(id: &str) -> ToolError {
    ToolError::new(TIER_NOT_FOUND, format!("There is no tier {id}."))
        .detail(json!({"entity": "tier", "id": id}))
        .next("list_tiers")
}

/// `{value, unit_text}` of a tier's points multiplier, or null.
fn points_json(t: &repo::TierRow) -> Value {
    match (
        &t.membership_points_earned_value,
        &t.membership_points_earned_unit_text,
    ) {
        (Some(v), unit) => json!({"value": v, "unit_text": unit}),
        _ => Value::Null,
    }
}

async fn member_count(tx: &mut TxCx, program: &str, tier: &str) -> Result<i64, ToolError> {
    Ok(repo::member_counts(tx, Some(program))
        .await?
        .into_iter()
        .find(|(t, _)| t == tier)
        .map_or(0, |(_, n)| n))
}

// ── list_tiers ──────────────────────────────────────────────────────────────────────────

/// A field of a tier row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TierField {
    /// The tier id.
    Id,
    /// The name people see.
    Name,
    /// Higher is better.
    Rank,
    /// The program the tier belongs to.
    ProgramId,
    /// Members in the tier now.
    MemberCount,
}

impl TierField {
    fn word(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Name => "name",
            Self::Rank => "rank",
            Self::ProgramId => "program_id",
            Self::MemberCount => "member_count",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTiersInput {
    /// Only the tiers of this program. All programs when left out.
    #[schemars(regex(pattern = r"^prg_[a-z0-9_]+$"), example = &"prg_rewards")]
    pub program_id: Option<String>,
    /// Return only these fields of each tier. All when left out.
    #[schemars(example = ["id", "member_count"])]
    pub fields: Option<Vec<TierField>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListTiers;

impl Operation for ListTiers {
    const META: OpMeta = OpMeta {
        address: "customer.membership.list_tiers",
        tool: "list_tiers",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/tiers",
        summary: "The store's member tiers with how many members each has, highest first. Turns words like \"VIP members\" into a tier id for valid_for_member_tier.",
        consumes: &[],
        produces: &[Slot::many("tier_id")],
        checks: &[],
        errors: &[PROGRAM_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListTiersInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListTiers {
    async fn read(&self, cx: &Cx, input: ListTiersInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if let Some(p) = &input.program_id
            && repo::program(&mut tx, p).await?.is_none()
        {
            return Err(
                ToolError::new(PROGRAM_NOT_FOUND, format!("There is no program {p}."))
                    .detail(json!({"entity": "program", "id": p}))
                    .next("list_tiers"),
            );
        }
        let tiers = repo::tiers(&mut tx, input.program_id.as_deref()).await?;
        let counts = repo::member_counts(&mut tx, input.program_id.as_deref()).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let fields: Vec<String> = input
            .fields
            .unwrap_or_default()
            .iter()
            .map(|f| f.word().to_owned())
            .collect();
        let total = i64::try_from(tiers.len()).unwrap_or(i64::MAX);
        let items: Vec<Value> = tiers
            .iter()
            .skip(usize::try_from(page.offset).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit).unwrap_or(0))
            .map(|t| {
                let n = counts.iter().find(|(id, _)| *id == t.id).map_or(0, |c| c.1);
                only_fields(
                    json!({"id": t.id, "name": t.name, "rank": t.rank,
                           "program_id": t.member_program_id, "member_count": n}),
                    &fields,
                )
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(items.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

// ── get_tier ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetTierInput {
    /// The tier (`list_tiers`).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub tier_id: String,
    /// Top-level fields to return. All when left out.
    #[schemars(example = ["has_tier_benefit", "member_count"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetTier;

impl Operation for GetTier {
    const META: OpMeta = OpMeta {
        address: "customer.membership.get_tier",
        tool: "get_tier",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/tiers/{tier_id}",
        summary: "One tier in full: requirements, benefits, points, its program's evaluation and movement rules, member count, and what running promotions and coupons refer to it.",
        consumes: &[Slot::one("tier_id")],
        produces: &[],
        checks: &[],
        errors: &[TIER_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetTierInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetTier {
    async fn read(&self, cx: &Cx, input: GetTierInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let t = repo::tier(&mut tx, &input.tier_id)
            .await?
            .ok_or_else(|| tier_not_found(&input.tier_id))?;
        let program = repo::program(&mut tx, &t.member_program_id).await?;
        let members = member_count(&mut tx, &t.member_program_id, &t.id).await?;
        let references = match providers::tier_references() {
            Some(read) => Some(read(&mut tx, &t.id).await?),
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;
        let mut out = json!({
            "id": t.id, "name": t.name, "rank": t.rank,
            "program": program.map(|p| json!({"id": p.id, "name": p.name, "version": p.version,
                                              "evaluation": p.evaluation, "movement": p.movement})),
            "requirement_combine": requirement_combine(&t.has_tier_requirement),
            "has_tier_requirement": requirement_list(&t.has_tier_requirement),
            "has_tier_benefit": t.has_tier_benefit,
            "membership_points_earned": points_json(&t),
            "member_count": members,
        });
        // Display targeting lives in merchandising, which customer cannot read.
        if let Some(r) = references {
            out["referenced_by"] = json!({"active_promotions": r.active_promotions,
                                          "active_coupons": r.active_coupons,
                                          "display_targeting": null});
        }
        Ok(only_fields(out, &input.fields.unwrap_or_default()))
    }
}

// ── get_member_tier ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetMemberTierInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
    /// The program. The member's first program when left out.
    #[schemars(regex(pattern = r"^prg_[a-z0-9_]+$"), example = &"prg_rewards")]
    pub program_id: Option<String>,
}

pub struct GetMemberTier;

impl Operation for GetMemberTier {
    const META: OpMeta = OpMeta {
        address: "customer.membership.get_member_tier",
        tool: "get_member_tier",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/{customer_id}/tier",
        summary: "A member's current tier, why they have it, how far they are from the next one, whether they keep it, and the next evaluation date.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::one("tier_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND, TIER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetMemberTierInput;
    type Output = Value;
}

/// `Spend at least 300.00 USD in 6 months`, one line per requirement.
fn requirement_text(req: &Value) -> String {
    let metric = req
        .get("metric")
        .and_then(Value::as_str)
        .unwrap_or("custom");
    let window = req.get("window").map(|w| {
        let length = w.get("length").and_then(Value::as_u64).unwrap_or(0);
        let unit = w.get("unit").and_then(Value::as_str).unwrap_or("month");
        match w.get("type").and_then(Value::as_str) {
            Some("lifetime") => " in total".to_owned(),
            _ => format!(" in {length} {unit}{}", if length == 1 { "" } else { "s" }),
        }
    });
    let window = window.unwrap_or_default();
    match metric {
        "spend" => match threshold_money(req, "threshold") {
            Some(m) => {
                let m = cents(m.value, m.currency);
                format!("Spend at least {} {}{window}", m.value, m.currency)
            }
            None => format!("Spend{window}"),
        },
        "order_count" => format!(
            "At least {} delivered orders{window}",
            req.get("threshold").and_then(Value::as_i64).unwrap_or(0)
        ),
        "invite" => "Invited".to_owned(),
        "manual" => "Set by an operator".to_owned(),
        other => other.replace('_', " "),
    }
}

impl ReadOp<Cx> for GetMemberTier {
    async fn read(&self, cx: &Cx, input: GetMemberTierInput) -> Result<Value, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let person = repo::person(&mut tx, &customer_id)
            .await?
            .ok_or_else(|| customer_not_found(&customer_id))?;
        let no_tier = || {
            ToolError::new(TIER_NOT_FOUND, format!("{customer_id} has no tier."))
                .detail(json!({"entity": "tier", "customer_id": customer_id}))
        };
        if person.member_state == "withdrawn" {
            return Err(no_tier());
        }
        let membership = repo::memberships(&mut tx, &customer_id)
            .await?
            .into_iter()
            .find(|m| {
                input
                    .program_id
                    .as_deref()
                    .is_none_or(|p| p == m.member_program_id)
            })
            .ok_or_else(no_tier)?;
        let program = repo::program(&mut tx, &membership.member_program_id).await?;
        let tiers = repo::tiers(&mut tx, Some(&membership.member_program_id)).await?;
        let orders: Option<Vec<MemberOrder>> = match providers::member_orders() {
            Some(read) => {
                // The widest window any tier of the program uses.
                let default_window = policy(cx, "customer.tier.requirement.window")
                    .cloned()
                    .unwrap_or(Value::Null);
                let since = tiers
                    .iter()
                    .flat_map(|t| requirement_list(&t.has_tier_requirement))
                    .map(|r| window_start(r.get("window").unwrap_or(&default_window), now, cx.time))
                    .min()
                    .unwrap_or(now);
                Some(read(&mut tx, &customer_id, since).await?)
            }
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;

        let current = tiers
            .iter()
            .find(|t| t.id == membership.tier_id)
            .ok_or_else(no_tier)?;
        let evaluation = program
            .as_ref()
            .map(|p| p.evaluation.clone())
            .unwrap_or(Value::Null);
        let settle_days = evaluation
            .get("settle_days")
            .and_then(Value::as_i64)
            .or_else(|| policy(cx, "customer.tier.evaluation.settle_days").and_then(Value::as_i64))
            .unwrap_or(0);
        let schedule = evaluation
            .get("schedule")
            .cloned()
            .or_else(|| policy(cx, "customer.tier.evaluation.schedule").cloned())
            .unwrap_or(Value::Null);
        let default_window = policy(cx, "customer.tier.requirement.window")
            .cloned()
            .unwrap_or(Value::Null);
        let default_channels = policy(cx, "customer.tier.requirement.counts.channels")
            .cloned()
            .unwrap_or(Value::Null);
        let tally_for = |req: &Value| {
            let currency =
                threshold_money(req, "threshold").map_or(acom_core::Currency::USD, |m| m.currency);
            let channels: Vec<String> = req
                .pointer("/counts/channels")
                .unwrap_or(&default_channels)
                .as_array()
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let start = window_start(req.get("window").unwrap_or(&default_window), now, cx.time);
            tally(
                orders.as_deref().unwrap_or_default(),
                currency,
                &channels,
                start,
                now,
                settle_days,
            )
        };

        let requirements = requirement_list(&current.has_tier_requirement);
        let combine_any =
            requirement_combine(&current.has_tier_requirement).as_deref() != Some("all");
        let source = if membership.tier_override.is_some() {
            "manual_override"
        } else if requirements.is_empty() {
            "entry"
        } else {
            "evaluation"
        };
        let spend_req = requirements
            .iter()
            .find(|r| r.get("metric").and_then(Value::as_str) == Some("spend"));
        let why_value = match (spend_req, &orders) {
            (Some(r), Some(_)) => {
                let t = tally_for(r);
                threshold_money(r, "threshold")
                    .map(|m| json!(cents(t.settled, m.currency)))
                    .unwrap_or(Value::Null)
            }
            _ => Value::Null,
        };
        let why_rule = match (&membership.tier_override, requirements.is_empty()) {
            (Some(o), _) => format!(
                "Set by an operator: {}",
                o.get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("no reason given")
            ),
            (None, true) => "Entry tier, no requirement".to_owned(),
            (None, false) => requirements
                .iter()
                .map(requirement_text)
                .collect::<Vec<_>>()
                .join(if combine_any { " or " } else { " and " }),
        };

        let mut out = json!({
            "customer_id": customer_id, "program_id": membership.member_program_id,
            "tier_id": current.id, "rank": current.rank,
            "since": membership.tier_since.with_timezone(&cx.time.zone).format("%Y-%m-%d").to_string(),
            "source": source, "override": membership.tier_override,
            "why": {"rule": why_rule, "value": why_value, "evaluation_id": null},
            "next_evaluation": next_evaluation(&schedule, now, cx.time).map(|t| cx.time.format(t)),
            "has_tier_benefit": current.has_tier_benefit,
        });
        if orders.is_some() {
            // The next tier up is the lowest rank above the current one.
            let next = tiers
                .iter()
                .filter(|t| t.rank > current.rank)
                .min_by_key(|t| t.rank);
            let progress: Vec<Value> = next
                .map(|n| {
                    requirement_list(&n.has_tier_requirement)
                        .iter()
                        .filter_map(|r| progress_entry(&n.id, r, &tally_for(r), cx.time))
                        .collect()
                })
                .unwrap_or_default();
            let (status, keep) = retention(&requirements, combine_any, tally_for);
            out["progress"] = json!(progress);
            out["retention"] = json!({"status": status, "retain_threshold": keep});
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn inputs_are_closed_and_described() {
        for s in [
            closed_schema_for::<ListTiersInput>(),
            closed_schema_for::<GetTierInput>(),
            closed_schema_for::<GetMemberTierInput>(),
        ] {
            assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        }
        assert_eq!(GetMemberTier::META.audience, Audience::Both);
    }

    #[test]
    fn describes_requirements_in_a_line() {
        let gold = json!({"metric": "spend", "threshold": {"value": 300.00, "currency": "USD"},
                          "window": {"type": "rolling", "length": 6, "unit": "month"}});
        assert_eq!(
            requirement_text(&gold),
            "Spend at least 300.00 USD in 6 months"
        );
        assert_eq!(requirement_text(&json!({"metric": "invite"})), "Invited");
    }

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reads_tiers_and_memberships() {
        let mut tx = crate::testdb::tx("adm_sj").await;
        let tiers = repo::tiers(&mut tx, Some("prg_rewards")).await.unwrap();
        assert_eq!(
            tiers.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["vip", "gold", "member"]
        );
        let alex = repo::memberships(&mut tx, "cus_alex").await.unwrap();
        assert_eq!(alex[0].tier_id, "member");
        let cx = crate::testdb::cx("cus_alex").await;
        let out = GetMemberTier
            .read(
                &cx,
                GetMemberTierInput {
                    customer_id: "me".into(),
                    program_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(out["tier_id"], "member");
        assert_eq!(out["source"], "entry");
    }
}
