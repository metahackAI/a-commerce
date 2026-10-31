//! `revoke_coupon` (docs/api/promotion.md#coupon): takes unused member coupons back
//! (`issued | available → revoked`; a coupon given back after a cancel, `restored`, is unused
//! too). Used coupons stay: they are part of an order.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{ALREADY_USED, in_statuses, member_coupon, move_member_coupon};
use crate::model::count_text;
use crate::{check_id, not_found, repo};

/// The statuses of a member coupon nobody used.
const UNUSED: [&str; 3] = ["issued", "available", "restored"];

/// Which member coupons to take back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevokeMode {
    /// The one in `member_coupon_id`.
    MemberCoupon,
    /// Every unused copy of the coupon.
    AllUnused,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeCouponInput {
    /// The coupon.
    #[schemars(regex(pattern = r"^cpn_[a-z0-9_]+$"), example = &"cpn_birthday")]
    pub coupon_id: String,
    /// member_coupon: the one in `member_coupon_id`; all_unused: every unused copy.
    #[schemars(example = &"all_unused")]
    pub revoke_mode: RevokeMode,
    /// The member's copy (`list_member_coupons`) when `revoke_mode` is `member_coupon`.
    #[schemars(regex(pattern = r"^mcp_[a-z0-9_]+$"), example = &"mcp_alex_birthday")]
    pub member_coupon_id: Option<String>,
    /// Why, in a sentence, for the change set and the action log.
    #[schemars(length(min = 1, max = 500), example = &"Issued to the wrong segment.")]
    pub reason: String,
}

/// What a run of `revoke_coupon` writes: each member coupon and the status it leaves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RevokeRun {
    pub coupon_id: String,
    pub items: Vec<(String, String)>,
    pub reason: String,
}

pub struct RevokeCoupon;

impl Operation for RevokeCoupon {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.revoke",
        tool: "revoke_coupon",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/coupons/{coupon_id}/revoke",
        summary: "Take back member coupons nobody used: one member's copy, or every unused copy of the coupon. Used coupons stay with their orders.",
        consumes: &[Slot::one("coupon_id")],
        produces: &[Slot::many("member_coupon_id")],
        checks: &[],
        errors: &[ALREADY_USED, ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = RevokeCouponInput;
    type Output = Value;
}

impl WriteOp<Cx> for RevokeCoupon {
    type Plan = RevokeRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RevokeCouponInput,
    ) -> Result<Planned<RevokeRun>, ToolError> {
        check_id("coupon_id", "cpn", &input.coupon_id, false)?;
        let reason = input.reason.trim().to_owned();
        if reason.is_empty() {
            return Err(ToolError::invalid_input("`reason` must not be empty.")
                .detail(json!({"field": "reason", "reason": "missing"})));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::coupon(&mut tx, &input.coupon_id)
            .await?
            .ok_or_else(|| not_found("coupon", &input.coupon_id, "search_coupons"))?;
        let picked = match input.revoke_mode {
            RevokeMode::MemberCoupon => {
                let id = input.member_coupon_id.clone().ok_or_else(|| {
                    ToolError::invalid_input(
                        "`revoke_mode: member_coupon` needs `member_coupon_id`.",
                    )
                    .detail(json!({"field": "member_coupon_id", "reason": "missing"}))
                })?;
                check_id("member_coupon_id", "mcp", &id, false)?;
                let m = member_coupon(&mut tx, &c.id, &id)
                    .await?
                    .ok_or_else(|| not_found("member_coupon", &id, "list_member_coupons"))?;
                if m.coupon_status == "used" {
                    return Err(ToolError::new(
                        ALREADY_USED,
                        format!(
                            "{id} was used on {}; a used coupon stays with its order.",
                            m.order_id.as_deref().unwrap_or("an order")
                        ),
                    )
                    .detail(json!({"entity": "member_coupon", "from": "used"}))
                    .next("get_coupon"));
                }
                vec![m]
            }
            RevokeMode::AllUnused => in_statuses(&mut tx, &c.id, &UNUSED).await?,
        };
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<(String, String)> = picked
            .iter()
            .filter(|m| UNUSED.contains(&m.coupon_status.as_str()))
            .map(|m| (m.id.clone(), m.coupon_status.clone()))
            .collect();
        let basis = picked
            .iter()
            .map(|m| Basis {
                entity: "promotion.member_coupon".into(),
                id: m.id.clone(),
                version: i64::from(m.version),
            })
            .collect();
        let changes = if items.is_empty() {
            Vec::new()
        } else {
            let single = input.revoke_mode == RevokeMode::MemberCoupon;
            let mut from: Vec<&str> = items.iter().map(|(_, s)| s.as_str()).collect();
            from.sort_unstable();
            from.dedup();
            vec![Change {
                op: ChangeOp::Revoke,
                entity: "member_coupon".into(),
                id: if single {
                    items[0].0.clone()
                } else {
                    String::new()
                },
                from: if from.len() == 1 {
                    json!(from[0])
                } else {
                    json!(from)
                },
                to: json!({"revoked": items.len()}),
                note: Some(format!(
                    "{} unused {} of {}: {reason}",
                    count_text(items.len() as i64),
                    if items.len() == 1 { "copy" } else { "copies" },
                    c.discount_code
                )),
            }]
        };
        Ok(Planned {
            plan: RevokeRun {
                coupon_id: c.id,
                items,
                reason,
            },
            changes,
            checks: Vec::new(),
            basis,
            approval: ApprovalNeed::NotNeeded,
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &RevokeRun) -> Result<Committed, ToolError> {
        for (id, from) in &run.items {
            let owned = member_coupon(tx, &run.coupon_id, id)
                .await?
                .and_then(|m| m.owned_through);
            move_member_coupon(tx, id, from, "revoked", owned).await?;
        }
        let ids: Vec<&String> = run.items.iter().map(|(id, _)| id).collect();
        Ok(Committed {
            id: run.coupon_id.clone(),
            produced: serde_json::Map::from_iter([
                ("member_coupon_id".to_owned(), json!(ids)),
                ("revoked".to_owned(), json!(ids.len())),
            ]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &RevokeRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        for (id, _) in &run.items {
            let m = member_coupon(&mut tx, &run.coupon_id, id).await?;
            if m.as_ref().map(|m| m.coupon_status.as_str()) != Some("revoked") {
                mismatches.push(format!("{id} is not revoked"));
            }
        }
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, tx: &mut TxCx, run: &RevokeRun, _: &Committed) -> Result<(), ToolError> {
        for (id, from) in &run.items {
            let owned = member_coupon(tx, &run.coupon_id, id)
                .await?
                .and_then(|m| m.owned_through);
            move_member_coupon(tx, id, "revoked", from, owned).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn takes_the_spec_inputs() {
        let s = closed_schema_for::<RevokeCouponInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(s["required"], json!(["coupon_id", "revoke_mode", "reason"]));
        let i: RevokeCouponInput = serde_json::from_value(json!({"coupon_id": "cpn_birthday",
            "revoke_mode": "all_unused", "reason": "Issued to the wrong segment."}))
        .unwrap();
        assert_eq!(i.revoke_mode, RevokeMode::AllUnused);
    }
}
