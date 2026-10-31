//! `get_member_tier`.

use super::requirement_text;
use crate::access::{customer_for, policy};
use crate::person::customer_not_found;
use crate::progress::{
    cents, next_evaluation, progress_entry, requirement_combine, requirement_list, retention,
    tally, threshold_money, window_start,
};
use crate::service::providers::{self, MemberOrder};
use crate::{CUSTOMER_NOT_FOUND, TIER_NOT_FOUND, repo};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

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
    fn input_is_closed_and_shoppers_may_use_it() {
        let s = closed_schema_for::<GetMemberTierInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(GetMemberTier::META.audience, Audience::Both);
    }
}
