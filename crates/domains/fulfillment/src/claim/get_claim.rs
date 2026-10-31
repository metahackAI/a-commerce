//! `get_claim`: one claim with its items, evidence, inspection, refund and history.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError, fields,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::visible_claim;
use crate::errors::CLAIM_NOT_FOUND;
use crate::repo;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetClaimInput {
    /// The cancel, return or exchange claim.
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_20402")]
    pub claim_id: String,
    /// Top-level fields to return, for example only the status and its history.
    #[schemars(example = ["status", "status_history"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetClaim;

impl Operation for GetClaim {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.get",
        tool: "get_claim",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/claims/{claim_id}",
        summary: "One claim (cancel, return or exchange): items, reason, fault, evidence, inspection, linked refund, and its status history.",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::one("refund_id")],
        checks: &[],
        errors: &[CLAIM_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetClaimInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetClaim {
    async fn read(&self, cx: &Cx, input: GetClaimInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = visible_claim(&mut tx, &input.claim_id, "search_claims").await?;
        let items = repo::claim_items(&mut tx, std::slice::from_ref(&c.id)).await?;
        let inspection = repo::inspection(&mut tx, &c.id).await?;
        let refund = repo::refund_of_claim(&mut tx, &c.id).await?;
        let events = repo::claim_events(&mut tx, &c.id).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let evidence: Vec<Value> = c
            .evidence
            .as_array()
            .map(|l| {
                l.iter()
                    .map(|e| json!({"id": e.get("id"), "evidence_type": e.get("evidence_type")}))
                    .collect()
            })
            .unwrap_or_default();
        let history: Vec<Value> = events
            .iter()
            .filter_map(|e| {
                e.to_state
                    .as_ref()
                    .map(|s| json!({"status": s, "at": cx.time.format(e.start_time)}))
            })
            .collect();
        let refund_estimate = match (c.refund_estimate_value, &c.refund_estimate_currency) {
            (Some(v), Some(cur)) => Some(repo::money(v, cur)?),
            _ => None,
        };
        let out = json!({
            "id": c.id,
            "claim_type": c.claim_type,
            "status": c.claim_status,
            "order_id": c.order_id,
            "customer_id": c.customer_id,
            "items": items.iter().map(|i| {
                let mut v = json!({"order_item_id": i.order_item_id, "quantity": i.order_quantity});
                if let Some(r) = &i.replacement_product_id {
                    v["replacement_variant_id"] = json!(r);
                }
                v
            }).collect::<Vec<_>>(),
            "reason": c.reason(),
            "reason_text": c.reason_text,
            "liability": c.liability,
            "claim_path": c.claim_path,
            "return_fees": c.return_fees,
            "amount": c.amount()?,
            "refund_estimate": refund_estimate,
            "send_back_method": c.send_back_method,
            "pickup_window": c.pickup_window,
            "evidence": evidence,
            "inspection": inspection.map(|i| json!({
                "id": i.id, "condition": i.inspection_condition, "liability": i.liability,
                "note": i.description, "at": cx.time.format(i.end_time)})),
            "refund_id": refund.map(|r| r.id),
            "charge": c.charge()?,
            "charge_paid_at": c.charge_paid_at.map(|t| cx.time.format(t)),
            "requested_at": cx.time.format(c.start_time),
            "closed_at": c.end_time.map(|t| cx.time.format(t)),
            "status_history": history,
        });
        fields::keep(out, &input.fields.unwrap_or_default())
    }
}
