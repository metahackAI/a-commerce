//! `get_contract`: a seller's contract (commission by category, settlement cycle and method,
//! payout account masked for operators, period) and its version history.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{contract_not_found, current, version_wire};
use crate::rules::Rules;
use crate::viewer;
use crate::{CONTRACT_NOT_FOUND, SELLER_NOT_FOUND, model, repo, seller_not_found, store};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetContractInput {
    /// The seller, from `search_sellers`.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_fieldstone")]
    pub seller_id: String,
    /// The version to show, 1 and up; the version in force now when left out.
    #[schemars(range(min = 1), example = 1)]
    pub version: Option<i32>,
    /// Top-level fields to return, for example `["acom:commission_rate", "valid_through"]`.
    #[schemars(example = ["acom:commission_rate", "acom:settlement_cycle", "valid_through"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetContract;

impl Operation for GetContract {
    const META: OpMeta = OpMeta {
        address: "partner.contract.get_contract",
        tool: "get_contract",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/partner/sellers/{seller_id}/contract",
        summary: "A seller's contract: commission rates by category, settlement cycle and method, payout account (masked), period, status, and the list of its versions.",
        consumes: &[Slot::one("seller_id")],
        produces: &[Slot::one("contract_id")],
        checks: &[],
        errors: &[CONTRACT_NOT_FOUND, SELLER_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetContractInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetContract {
    async fn read(&self, cx: &Cx, input: GetContractInput) -> Result<Value, ToolError> {
        viewer::staff(cx)?;
        if input.version.is_some_and(|v| v < 1) {
            return Err(ToolError::invalid_input("`version` starts at 1.")
                .detail(json!({"field": "version", "reason": "range"})));
        }
        let rules = Rules::load(&cx.policy)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(store)?;
        let viewer = viewer::viewer(cx, &mut tx).await?;
        viewer.may_see(&input.seller_id)?;
        let s = repo::seller(&mut tx, &input.seller_id)
            .await?
            .ok_or_else(|| seller_not_found(&input.seller_id))?;
        let (contract_id, _) = repo::contract_of(&mut tx, &s.id)
            .await?
            .ok_or_else(|| contract_not_found(&s.id, None))?;
        let versions = repo::versions(&mut tx, &contract_id).await?;
        tx.commit().await.map_err(store)?;
        let v = match input.version {
            Some(n) => versions.iter().find(|v| v.version_number == n),
            None => current(&versions, now),
        }
        .ok_or_else(|| contract_not_found(&s.id, input.version))?;
        let mut out = version_wire(&contract_id, &s.id, v, now, &cx.time, &viewer, &rules);
        out["acom:versions"] = json!(
            versions
                .iter()
                .map(|v| json!({
                    "version": v.version_number,
                    "valid_from": cx.time.format(v.valid_from),
                    "valid_through": cx.time.format(v.valid_through),
                    "acom:contract_status": model::contract_status(v.valid_from, v.valid_through, now),
                }))
                .collect::<Vec<_>>()
        );
        acom_contract::fields::keep(out, &input.fields.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    #[tokio::test]
    async fn versions_start_at_one() {
        let e = GetContract
            .read(
                &offline("adm_sj"),
                input(json!({"seller_id": "slr_fieldstone", "version": 0})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }
}
