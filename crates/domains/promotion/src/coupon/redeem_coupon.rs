//! `redeem_coupon` (docs/api/promotion.md#coupon): marks a member's coupon used on an order and
//! records the order's benefit on it. An internal write commerce runs inside placing an order;
//! the agent never sees it.
//!
//! The order (who placed it, its subtotal and the discount it recorded) is read through sales'
//! order reader (`service::for_sales::orders`), so the tool takes the spec's two inputs only.
//! Before recording, the tier, the period, the order minimum and the per-member and per-order
//! limits are checked again.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::Money;
use acom_core::enums::{ActorKind, CouponApplication};
use acom_customer::service::for_promotion as customers;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{COUPON_EXPIRED, MIN_ORDER_NOT_MET, on_order, policy_number, used_by};
use crate::benefit::{NOT_ELIGIBLE, in_period, promotion_active, repo as benefit_repo};
use crate::model::money_text;
use crate::service::for_sales::orders;
use crate::{audience, check_id, not_found, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RedeemCouponInput {
    /// The coupon the order used.
    #[schemars(regex(pattern = r"^cpn_[a-z0-9_]+$"), example = &"cpn_vipweekend20")]
    pub coupon_id: String,
    /// The order being placed.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20518")]
    pub order_id: String,
}

/// What a run of `redeem_coupon` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RedeemRun {
    pub coupon_id: String,
    pub order_id: String,
    pub customer_id: String,
    /// The member coupon that becomes `used`: one the member held, or a new one.
    pub member_coupon_id: String,
    /// Whether the member held it (it moves to `used`) or it is recorded now.
    pub held: bool,
    /// Already recorded on this order: nothing to write.
    pub already: bool,
    pub benefit: Money,
}

pub struct RedeemCoupon;

impl Operation for RedeemCoupon {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.redeem",
        tool: "redeem_coupon",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/coupons/{coupon_id}/redeem",
        summary: "Internal: while an order is placed, mark the member's coupon used on it and record the order's benefit, after checking tier, period, minimum order and limits again.",
        consumes: &[Slot::one("coupon_id"), Slot::one("order_id")],
        produces: &[Slot::one("member_coupon_id")],
        checks: &["coupon.tier_match", "member_benefits_per_order"],
        errors: &[
            NOT_ELIGIBLE,
            COUPON_EXPIRED,
            MIN_ORDER_NOT_MET,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Internal,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = RedeemCouponInput;
    type Output = Value;
}

fn refuse(code: ErrorCode, rule: &str, message: String) -> ToolError {
    ToolError::new(code, message).detail(json!({"rule": rule}))
}

impl WriteOp<Cx> for RedeemCoupon {
    type Plan = RedeemRun;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RedeemCouponInput,
    ) -> Result<Planned<RedeemRun>, ToolError> {
        check_id("coupon_id", "cpn", &input.coupon_id, false)?;
        check_id("order_id", "ord", &input.order_id, false)?;
        let per_order = policy_number(cx, "promotion.member_benefits_per_order")?;
        let per_member = policy_number(cx, "promotion.coupon.per_member_limit")?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::coupon(&mut tx, &input.coupon_id)
            .await?
            .ok_or_else(|| not_found("coupon", &input.coupon_id, "search_coupons"))?;
        let order = orders(&mut tx, std::slice::from_ref(&input.order_id))
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| not_found("order", &input.order_id, "search_orders"))?;
        let customer = order.customer_id.clone().ok_or_else(|| {
            refuse(
                NOT_ELIGIBLE,
                "coupon.tier_match",
                format!(
                    "Order {} was placed by a guest; coupons are for members.",
                    order.order_id
                ),
            )
        })?;
        let shopper = cx.principal.kind == ActorKind::Shopper;
        if shopper && cx.principal.customer_id() != Some(customer.as_str()) {
            return Err(
                ToolError::new(ErrorCode::FORBIDDEN, "This is not your order.")
                    .detail(json!({"reason": "owner"})),
            );
        }
        let basis = vec![Basis {
            entity: "promotion.coupon".into(),
            id: c.id.clone(),
            version: i64::from(c.version),
        }];
        let benefit = if order.coupon_id.as_deref().is_none_or(|id| id == c.id) {
            order.discount
        } else {
            Money::zero(order.discount.currency)
        };
        // Already recorded on this order: the same result again.
        if let Some(done) = on_order(&mut tx, &c.id, &order.order_id)
            .await?
            .into_iter()
            .find(|m| m.coupon_status == "used")
        {
            tx.commit().await.map_err(ToolError::from)?;
            return Ok(Planned {
                plan: RedeemRun {
                    coupon_id: c.id,
                    order_id: order.order_id,
                    customer_id: customer,
                    member_coupon_id: done.id,
                    held: true,
                    already: true,
                    benefit,
                },
                changes: Vec::new(),
                checks: Vec::new(),
                basis,
                approval: ApprovalNeed::NotNeeded,
            });
        }
        let promotion = repo::promotion(&mut tx, &c.promotion_id).await?;
        let extras = repo::promotion_extras(&mut tx, &c.promotion_id).await?;
        let tier = customers::tier_of(&mut tx, &customer).await?;
        let used = used_by(&mut tx, &customer).await?;
        let held = benefit_repo::held_coupons(&mut tx, &customer).await?;
        tx.commit().await.map_err(ToolError::from)?;

        // The coupon's period (its own, else its promotion's) holds the order, and the promotion
        // is not paused or ended.
        let open = c
            .valid_from
            .is_some_and(|f| in_period(f, c.valid_through, order.order_date))
            && promotion.as_ref().is_some_and(|p| {
                promotion_active(
                    &p.promotion_status,
                    p.valid_from,
                    p.valid_through,
                    order.order_date,
                )
            });
        if !open {
            return Err(refuse(
                COUPON_EXPIRED,
                "coupon.validity",
                format!("{} is not valid for an order placed now.", c.discount_code),
            ));
        }
        if let Some(t) = &c.valid_for_member_tier
            && tier.as_deref() != Some(t.as_str())
        {
            return Err(refuse(
                NOT_ELIGIBLE,
                "coupon.tier_match",
                format!(
                    "{} is for {} members; the member is {}.",
                    c.discount_code,
                    t,
                    audience(tier.as_deref())
                ),
            ));
        }
        if let Some(x) = &extras
            && let Some(min) = x.eligible_transaction_volume_value
            && order.subtotal.value < min
        {
            return Err(refuse(
                MIN_ORDER_NOT_MET,
                "coupon.min_order",
                format!(
                    "{} needs an order of {}; this one is {}.",
                    c.discount_code,
                    money_text(min, order.subtotal.currency.as_str()),
                    money_text(order.subtotal.value, order.subtotal.currency.as_str())
                ),
            ));
        }
        let on_this_order = used
            .iter()
            .filter(|m| m.order_id.as_deref() == Some(order.order_id.as_str()))
            .count() as i64;
        if per_order > 0 && on_this_order >= per_order {
            return Err(refuse(
                NOT_ELIGIBLE,
                "member_benefits_per_order",
                format!(
                    "The order already records {on_this_order} member benefit; the limit is {per_order}."
                ),
            ));
        }
        let used_this = used.iter().filter(|m| m.coupon_id == c.id).count() as i64;
        if per_member > 0 && used_this >= per_member {
            return Err(refuse(
                NOT_ELIGIBLE,
                "coupon.per_member_limit",
                format!(
                    "The member already used {} {used_this} times.",
                    c.discount_code
                ),
            ));
        }
        let holding = held.iter().find(|h| {
            h.coupon_id == c.id && in_period(h.owned_from, h.owned_through, order.order_date)
        });
        let needs_holding = c.application == CouponApplication::Download.as_str();
        if needs_holding && holding.is_none() {
            return Err(refuse(
                NOT_ELIGIBLE,
                "coupon.applications",
                format!("The member does not hold {} to use it.", c.discount_code),
            ));
        }
        let (member_coupon_id, held) = match holding {
            Some(h) => (h.id.clone(), true),
            None => (
                acom_core::ids::MemberCouponId::generate().to_string(),
                false,
            ),
        };
        let checks = vec![
            Check {
                rule: "coupon.tier_match".into(),
                passed: true,
                detail: Some(format!(
                    "For {}",
                    audience(c.valid_for_member_tier.as_deref())
                )),
                fix_tool: None,
            },
            Check {
                rule: "member_benefits_per_order".into(),
                passed: true,
                detail: Some(format!(
                    "{} of {per_order} member benefits on the order",
                    on_this_order + 1
                )),
                fix_tool: None,
            },
        ];
        Ok(Planned {
            changes: vec![Change {
                op: if held {
                    ChangeOp::Update
                } else {
                    ChangeOp::Create
                },
                entity: "member_coupon".into(),
                id: if held {
                    member_coupon_id.clone()
                } else {
                    String::new()
                },
                from: if held {
                    json!("available")
                } else {
                    Value::Null
                },
                to: json!({"status": "used", "order_id": order.order_id,
                           "benefit_recorded": crate::money(benefit.value, benefit.currency.as_str())}),
                note: None,
            }],
            plan: RedeemRun {
                coupon_id: c.id,
                order_id: order.order_id,
                customer_id: customer,
                member_coupon_id,
                held,
                already: false,
                benefit,
            },
            checks,
            basis,
            approval: ApprovalNeed::NotNeeded,
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &RedeemRun) -> Result<Committed, ToolError> {
        let benefit = (run.benefit.value, run.benefit.currency.as_str());
        if !run.already {
            if let Some(id) = benefit_repo::used_on_order(tx, &run.coupon_id, &run.order_id).await?
            {
                if id != run.member_coupon_id {
                    return Err(refuse(
                        NOT_ELIGIBLE,
                        "coupon.per_member_limit",
                        "The coupon was recorded on this order in the meantime.".into(),
                    ));
                }
            } else if run.held {
                if !benefit_repo::use_held(tx, &run.member_coupon_id, &run.order_id, benefit)
                    .await?
                {
                    return Err(refuse(
                        NOT_ELIGIBLE,
                        "coupon.applications",
                        "The coupon was used or revoked since the preview.".into(),
                    ));
                }
            } else {
                benefit_repo::insert_used(
                    tx,
                    &run.member_coupon_id,
                    &run.coupon_id,
                    &run.customer_id,
                    &run.order_id,
                    benefit,
                )
                .await?;
            }
        }
        Ok(Committed {
            id: run.member_coupon_id.clone(),
            produced: serde_json::Map::from_iter([
                ("member_coupon_id".to_owned(), json!(run.member_coupon_id)),
                (
                    "benefit_recorded".to_owned(),
                    crate::money(run.benefit.value, run.benefit.currency.as_str()),
                ),
            ]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &RedeemRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let rows = on_order(&mut tx, &run.coupon_id, &run.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = rows
            .iter()
            .any(|m| m.id == run.member_coupon_id && m.coupon_status == "used");
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} is not used on {}",
                    run.member_coupon_id, run.order_id
                )]
            },
        })
    }

    async fn undo(&self, tx: &mut TxCx, run: &RedeemRun, _: &Committed) -> Result<(), ToolError> {
        // Only the change-set runner calls this, to roll back a set that failed after this item;
        // a placed order's coupon comes back through restore_coupon instead.
        if run.already {
            return Ok(());
        }
        let back = if run.held { "available" } else { "revoked" };
        sqlx::query(
            "UPDATE promotion.member_coupon SET coupon_status = $4,
               order_id = CASE WHEN $4 = 'available' THEN NULL ELSE order_id END,
               used_at = CASE WHEN $4 = 'available' THEN NULL ELSE used_at END,
               updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2 AND order_id = $3 AND coupon_status = 'used'",
        )
        .bind(tx.tenant.as_str())
        .bind(&run.member_coupon_id)
        .bind(&run.order_id)
        .bind(back)
        .execute(&mut *tx.tx)
        .await
        .map_err(repo::db)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};
    use rust_decimal::Decimal;

    #[test]
    fn is_internal_and_takes_the_spec_inputs() {
        let s = closed_schema_for::<RedeemCouponInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(s["required"], json!(["coupon_id", "order_id"]));
        assert_eq!(RedeemCoupon::META.audience, Audience::Internal);
        assert_eq!(RedeemCoupon::UNDO, UndoSupport::NotSupported);
        let run = RedeemRun {
            coupon_id: "cpn_vipweekend20".into(),
            order_id: "ord_20518".into(),
            customer_id: "cus_jamie".into(),
            member_coupon_id: "mcp_x".into(),
            held: false,
            already: false,
            benefit: Money::new(Decimal::new(1780, 2), acom_core::Currency::USD),
        };
        let back: RedeemRun = serde_json::from_value(serde_json::to_value(&run).unwrap()).unwrap();
        assert_eq!(back, run);
    }
}
