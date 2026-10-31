//! `get_tier`.

use super::{member_count, points_json, tier_not_found};
use crate::progress::{requirement_combine, requirement_list};
use crate::service::providers::{self};
use crate::{TIER_NOT_FOUND, repo};
use acom_contract::fields;
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

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
        fields::keep(out, &input.fields.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<GetTierInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }
}
