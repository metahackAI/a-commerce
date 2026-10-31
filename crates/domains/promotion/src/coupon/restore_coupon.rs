//! `restore_coupon` (docs/api/promotion.md#coupon): gives a coupon used on an order back to its
//! member when the order is cancelled or returned (`used → restored`). A whole cancel gives back
//! every coupon; a partial one only product coupons (`coupon.restore_on_cancel`). A coupon that
//! expired meanwhile gets `coupon.expiry_grace_days` more days.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Days, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{move_member_coupon, on_order, policy_number};
use crate::model::AppliesTo;
use crate::{check_id, not_found, repo, rules, time_or_null};

/// What was cancelled or returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestoreScope {
    /// The whole order: every coupon comes back.
    Full,
    /// Part of the order: only product coupons come back (`coupon.restore_on_cancel.partial`).
    Partial,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RestoreCouponInput {
    /// The coupon the order used.
    #[schemars(regex(pattern = r"^cpn_[a-z0-9_]+$"), example = &"cpn_birthday")]
    pub coupon_id: String,
    /// The order cancelled or returned.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20352")]
    pub order_id: String,
    /// full: the whole order was cancelled or returned; partial: part of it.
    #[schemars(example = &"full")]
    pub scope: RestoreScope,
}

/// What a run of `restore_coupon` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RestoreRun {
    pub coupon_id: String,
    pub member_coupon_id: String,
    /// Nothing to write: already restored, or an order coupon on a partial cancel.
    pub skip: bool,
    pub owned_through_before: Option<DateTime<Utc>>,
    pub owned_through_after: Option<DateTime<Utc>>,
}

pub struct RestoreCoupon;

impl Operation for RestoreCoupon {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.restore",
        tool: "restore_coupon",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/coupons/{coupon_id}/restore",
        summary: "Give a member the coupon an order used back after a cancel or return (used → restored). A whole cancel restores it; a partial one only restores product coupons. An expired coupon gets a few grace days.",
        consumes: &[Slot::one("coupon_id"), Slot::one("order_id")],
        produces: &[Slot::one("member_coupon_id")],
        checks: &[],
        errors: &[ErrorCode::INVALID_STATE, ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = RestoreCouponInput;
    type Output = Value;
}

/// Whether a coupon's promotion discounts products (an item price, or products or categories in
/// scope) rather than the order.
pub(crate) fn product_coupon(layer: &str, applies_to: &Value) -> bool {
    layer == "item_price"
        || matches!(
            AppliesTo::read(applies_to).scope.as_str(),
            "products" | "categories"
        )
}

impl WriteOp<Cx> for RestoreCoupon {
    type Plan = RestoreRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RestoreCouponInput,
    ) -> Result<Planned<RestoreRun>, ToolError> {
        check_id("coupon_id", "cpn", &input.coupon_id, false)?;
        check_id("order_id", "ord", &input.order_id, false)?;
        let grace = policy_number(cx, "promotion.coupon.expiry_grace_days")?;
        let partial_rule = rules::word(&cx.policy, "promotion.coupon.restore_on_cancel.partial")?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::coupon(&mut tx, &input.coupon_id)
            .await?
            .ok_or_else(|| not_found("coupon", &input.coupon_id, "search_coupons"))?;
        let p = repo::promotion(&mut tx, &c.promotion_id).await?;
        let rows = on_order(&mut tx, &c.id, &input.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let m = rows
            .iter()
            .find(|m| matches!(m.coupon_status.as_str(), "used" | "restored"))
            .or(rows.first())
            .ok_or_else(|| {
                not_found(
                    "member_coupon",
                    &format!("{} on {}", c.id, input.order_id),
                    "list_member_coupons",
                )
            })?;
        let basis = vec![Basis {
            entity: "promotion.member_coupon".into(),
            id: m.id.clone(),
            version: i64::from(m.version),
        }];
        // Nothing to write: already restored, or an order coupon on a partial cancel.
        let skip = RestoreRun {
            coupon_id: c.id.clone(),
            member_coupon_id: m.id.clone(),
            skip: true,
            owned_through_before: m.owned_through,
            owned_through_after: m.owned_through,
        };
        if m.coupon_status == "restored" {
            return Ok(Planned {
                plan: skip,
                changes: Vec::new(),
                checks: Vec::new(),
                basis,
                approval: ApprovalNeed::NotNeeded,
            });
        }
        if m.coupon_status != "used" {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("{} is {}, not used, so there is nothing to give back.", m.id, m.coupon_status),
            )
            .detail(json!({"entity": "member_coupon", "from": m.coupon_status, "allowed_from": ["used"]}))
            .next("get_coupon"));
        }
        let is_product = p
            .as_ref()
            .is_some_and(|p| product_coupon(&p.discount_layer, &p.applies_to));
        if input.scope == RestoreScope::Partial
            && partial_rule == "product_coupon_only"
            && !is_product
        {
            return Ok(Planned {
                plan: skip,
                changes: vec![Change {
                    op: ChangeOp::Exclude,
                    entity: "member_coupon".into(),
                    id: m.id.clone(),
                    from: Value::Null,
                    to: json!({"why": "coupon.restore_on_cancel"}),
                    note: Some(format!(
                        "{} is an order coupon; on a partial cancel it stays used",
                        c.discount_code
                    )),
                }],
                checks: Vec::new(),
                basis,
                approval: ApprovalNeed::NotNeeded,
            });
        }
        // Expired meanwhile: the grace days from today, to 23:59 store time.
        let after = match m.owned_through {
            Some(t) if t < now => {
                let day = now.with_timezone(&cx.time.zone).date_naive()
                    + Days::new(u64::try_from(grace).unwrap_or(0));
                Some(
                    cx.time
                        .parse(&format!("{day}T23:59"))
                        .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?,
                )
            }
            other => other,
        };
        let note = (after != m.owned_through).then(|| {
            format!(
                "It had expired; usable again until {}",
                time_or_null(&cx.time, after).as_str().unwrap_or("")
            )
        });
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "member_coupon".into(),
                id: m.id.clone(),
                from: json!("used"),
                to: json!("restored"),
                note,
            }],
            plan: RestoreRun {
                coupon_id: c.id.clone(),
                member_coupon_id: m.id.clone(),
                skip: false,
                owned_through_before: m.owned_through,
                owned_through_after: after,
            },
            checks: Vec::new(),
            basis,
            approval: ApprovalNeed::NotNeeded,
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &RestoreRun) -> Result<Committed, ToolError> {
        if !run.skip
            && !move_member_coupon(
                tx,
                &run.member_coupon_id,
                "used",
                "restored",
                run.owned_through_after,
            )
            .await?
        {
            let now = super::member_coupon(tx, &run.coupon_id, &run.member_coupon_id).await?;
            if now.as_ref().map(|m| m.coupon_status.as_str()) != Some("restored") {
                return Err(ToolError::new(
                    ErrorCode::INVALID_STATE,
                    "The member coupon changed since the preview; preview again.",
                )
                .detail(json!({"entity": "member_coupon", "id": run.member_coupon_id}))
                .next("get_coupon"));
            }
        }
        Ok(Committed {
            id: run.member_coupon_id.clone(),
            produced: serde_json::Map::from_iter([(
                "member_coupon_id".to_owned(),
                json!(run.member_coupon_id),
            )]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &RestoreRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let m = super::member_coupon(&mut tx, &run.coupon_id, &run.member_coupon_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let want = if run.skip { None } else { Some("restored") };
        let matched = want.is_none_or(|w| m.as_ref().map(|m| m.coupon_status.as_str()) == Some(w));
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not restored", run.member_coupon_id)]
            },
        })
    }

    async fn undo(&self, tx: &mut TxCx, run: &RestoreRun, _: &Committed) -> Result<(), ToolError> {
        if !run.skip {
            move_member_coupon(
                tx,
                &run.member_coupon_id,
                "restored",
                "used",
                run.owned_through_before,
            )
            .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn partial_cancels_restore_product_coupons_only() {
        assert!(product_coupon("item_price", &json!({"scope": "products"})));
        assert!(product_coupon(
            "order_discount",
            &json!({"scope": "categories"})
        ));
        assert!(!product_coupon(
            "order_discount",
            &json!({"scope": "order"})
        ));
        let s = closed_schema_for::<RestoreCouponInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(s["properties"]["scope"]["enum"], json!(["full", "partial"]));
    }
}
