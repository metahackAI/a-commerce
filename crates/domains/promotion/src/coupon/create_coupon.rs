//! `create_coupon` (docs/api/promotion.md#coupon).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::{CouponApplication, CouponStacking};
use acom_customer::service::for_promotion as customers;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{CODE_TAKEN, COUPON_TIER_MISMATCH, STACKING_CONFLICT, policy_number, words};
use crate::model::{check_member_benefits, count_text};
use crate::repo::{self, CouponFilter, CouponRow};
use crate::{TIER_NOT_FOUND, audience, check_id, known_tier, not_found, time_or_null};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCouponInput {
    /// The code, unique in the store: 4–20 capital letters or digits.
    #[schemars(regex(pattern = r"^[A-Z0-9]{4,20}$"), example = &"VIPWEEKEND20")]
    pub code: String,
    /// The promotion whose benefit the coupon records (`set_sale_prices`,
    /// `list_active_promotions`).
    #[schemars(regex(pattern = r"^promo_[a-z0-9_]+$"), example = &"promo_autumn_knit")]
    pub promotion_id: String,
    /// Tier allowed to use it: a tier id (`list_tiers`) or `everyone`. Must be the promotion's
    /// tier.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub valid_for_member_tier: String,
    /// How the coupon reaches an order. automatic_at_checkout: applies to every eligible
    /// member's checkout, nothing to issue; code_entry: the shopper types the code; download:
    /// the shopper keeps it in their wallet first.
    #[schemars(example = &"automatic_at_checkout")]
    pub application: CouponApplication,
    /// Which other coupons may be used in the same order. no_tier_coupons: not with tier
    /// coupons; with_tier_coupons: together with them; no_other_coupons: with no other coupon.
    /// Default: the policy's `coupon.stacking.default`.
    #[schemars(example = &"no_tier_coupons")]
    pub stacking: Option<CouponStacking>,
}

/// What a run of `create_coupon` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CouponRun {
    /// `cpn_` + the code in lower case.
    pub coupon_id: String,
    pub code: String,
    /// The promotion id, or a `$item[n].promotion_id` placeholder until the runner binds it.
    pub promotion_id: String,
    /// `None` for everyone.
    pub valid_for_member_tier: Option<String>,
    pub application: String,
    pub stacking: String,
    pub records_benefit_only: bool,
}

pub struct CreateCoupon;

impl Operation for CreateCoupon {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.create",
        tool: "create_coupon",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/promotion/coupons",
        summary: "Create a coupon tied to a promotion. It records the promotion's benefit on the order (one discount line in the cart), never a second discount; its period follows the promotion. Always needs a person's approval.",
        consumes: &[Slot::one("promotion_id"), Slot::one("tier_id")],
        produces: &[Slot::one("coupon_id")],
        checks: &["member_benefits_per_order"],
        errors: &[
            ErrorCode::NOT_FOUND,
            CODE_TAKEN,
            COUPON_TIER_MISMATCH,
            STACKING_CONFLICT,
            TIER_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateCouponInput;
    type Output = Value;
}

/// The approval sentence of a new coupon: how it reaches whom.
pub(crate) fn coupon_reason(
    application: CouponApplication,
    code: &str,
    tier: Option<&customers::Tier>,
) -> String {
    let who = match tier {
        Some(t) => format!("{} {} members", count_text(t.member_count), t.name),
        None => "everyone".to_owned(),
    };
    match application {
        CouponApplication::AutomaticAtCheckout => {
            format!("Applies automatically at checkout for {who}")
        }
        CouponApplication::CodeEntry => format!("Code {code} works for {who}"),
        CouponApplication::Download => format!("Can be downloaded by {who}"),
    }
}

fn tier_mismatch(coupon: Option<&str>, promotion: Option<&str>) -> ToolError {
    ToolError::new(
        COUPON_TIER_MISMATCH,
        format!(
            "The coupon is for {} but the promotion is for {}; they must match.",
            audience(coupon),
            audience(promotion)
        ),
    )
    .detail(
        json!({"rule": "coupon.tier_match", "coupon": audience(coupon),
                   "promotion": audience(promotion)}),
    )
    .next("get_promotion")
}

impl WriteOp<Cx> for CreateCoupon {
    type Plan = CouponRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateCouponInput,
    ) -> Result<Planned<CouponRun>, ToolError> {
        let code = input.code.trim().to_owned();
        let code_ok = (4..=20).contains(&code.len())
            && code
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if !code_ok {
            return Err(ToolError::invalid_input(
                "`code` must be 4–20 capital letters or digits, for example VIPWEEKEND20.",
            )
            .detail(json!({"field": "code", "reason": "format"})));
        }
        let placeholder = check_id("promotion_id", "promo", &input.promotion_id, true)?;
        let application = input.application.as_str();
        if !words(cx, "promotion.coupon.applications")
            .iter()
            .any(|a| a == application)
        {
            return Err(ToolError::invalid_input(format!(
                "This store does not use `{application}` coupons."
            ))
            .detail(json!({"field": "application", "reason": "enum",
                           "allowed": words(cx, "promotion.coupon.applications")})));
        }
        let stacking = match input.stacking {
            Some(s) => s.as_str().to_owned(),
            None => cx
                .policy
                .lookup("promotion.coupon.stacking.default")
                .and_then(Value::as_str)
                .unwrap_or(CouponStacking::NoTierCoupons.as_str())
                .to_owned(),
        };
        let allowed = words(cx, "promotion.coupon.stacking.allowed");
        if !allowed.contains(&stacking) {
            return Err(ToolError::invalid_input(format!(
                "Stacking `{stacking}` is not allowed in this store."
            ))
            .detail(json!({"field": "stacking", "reason": "enum", "allowed": allowed})));
        }
        let per_order = policy_number(cx, "promotion.member_benefits_per_order")?;
        let records_benefit_only = cx
            .policy
            .lookup("promotion.coupon.records_benefit_only")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if let Some(existing) = repo::coupon_with_code(&mut tx, &code).await? {
            return Err(
                ToolError::new(CODE_TAKEN, format!("The code {code} is already used."))
                    .detail(json!({"entity": "coupon", "field": "code", "existing_id": existing}))
                    .next("search_coupons"),
            );
        }
        let tier = known_tier(&mut tx, &input.valid_for_member_tier).await?;
        let tier_id = tier.as_ref().map(|t| t.id.clone());
        let eligible = match &tier {
            Some(t) => t.member_count,
            None => customers::tiers(&mut tx)
                .await?
                .iter()
                .map(|t| t.member_count)
                .sum(),
        };
        let mut basis = Vec::new();
        let mut period = (Value::Null, Value::Null);
        if !placeholder {
            let p = repo::promotion(&mut tx, &input.promotion_id)
                .await?
                .ok_or_else(|| {
                    not_found("promotion", &input.promotion_id, "list_active_promotions")
                })?;
            if p.valid_for_member_tier != tier_id {
                return Err(tier_mismatch(
                    tier_id.as_deref(),
                    p.valid_for_member_tier.as_deref(),
                ));
            }
            if application == CouponApplication::AutomaticAtCheckout.as_str() {
                let (_, existing) = repo::coupons(
                    &mut tx,
                    &CouponFilter {
                        promotion_id: Some(p.id.clone()),
                        application: Some(application.to_owned()),
                        ..CouponFilter::default()
                    },
                    None,
                )
                .await?;
                if let Some(other) = existing.first() {
                    return Err(ToolError::new(
                        STACKING_CONFLICT,
                        format!(
                            "{} already records {} automatically; a second automatic coupon would record it twice.",
                            other.discount_code, p.name
                        ),
                    )
                    .detail(json!({"rule": "coupon.stacking", "items": [other.id]}))
                    .next("search_coupons"));
                }
            }
            period = (
                json!(cx.time.format(p.valid_from)),
                time_or_null(&cx.time, p.valid_through),
            );
            basis.push(Basis {
                entity: "promotion.promotion".into(),
                id: p.id,
                version: i64::from(p.version),
            });
        }
        tx.commit().await.map_err(ToolError::from)?;

        let reason = coupon_reason(input.application, &code, tier.as_ref());
        let check: Check = check_member_benefits(per_order, &stacking);
        let change = Change {
            op: ChangeOp::Create,
            entity: "coupon".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({
                "code": code,
                "promotion_id": input.promotion_id,
                "valid_for_member_tier": audience(tier_id.as_deref()),
                "application": application,
                "stacking": stacking,
                "valid_from": period.0,
                "valid_through": period.1,
                "eligible_members": eligible,
            }),
            note: placeholder.then(|| {
                "Valid from and through follow the promotion the earlier item creates".into()
            }),
        };
        Ok(Planned {
            plan: CouponRun {
                coupon_id: format!("cpn_{}", code.to_lowercase()),
                code,
                promotion_id: input.promotion_id,
                valid_for_member_tier: tier_id,
                application: application.to_owned(),
                stacking,
                records_benefit_only,
            },
            changes: vec![change],
            checks: vec![check],
            basis,
            approval: ApprovalNeed::Needed { reason },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &CouponRun) -> Result<Committed, ToolError> {
        // The promotion may have been a placeholder at preview; it is real now.
        let p = repo::promotion(tx, &run.promotion_id)
            .await?
            .ok_or_else(|| not_found("promotion", &run.promotion_id, "list_active_promotions"))?;
        if p.valid_for_member_tier != run.valid_for_member_tier {
            return Err(tier_mismatch(
                run.valid_for_member_tier.as_deref(),
                p.valid_for_member_tier.as_deref(),
            ));
        }
        repo::insert_coupon(
            tx,
            &repo::NewCoupon {
                id: &run.coupon_id,
                code: &run.code,
                promotion_id: &run.promotion_id,
                tier: run.valid_for_member_tier.as_deref(),
                application: &run.application,
                stacking: &run.stacking,
                records_benefit_only: run.records_benefit_only,
            },
        )
        .await?;
        Ok(Committed {
            id: run.coupon_id.clone(),
            produced: serde_json::Map::from_iter([("coupon_id".to_owned(), json!(run.coupon_id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &CouponRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let stored = repo::coupon(&mut tx, &run.coupon_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(compare_coupon(run, stored.as_ref()))
    }

    async fn undo(&self, tx: &mut TxCx, run: &CouponRun, _: &Committed) -> Result<(), ToolError> {
        // A coupon nobody holds is removed; otherwise the unused copies are revoked.
        if !repo::delete_coupon_if_unheld(tx, &run.coupon_id).await? {
            repo::revoke_unused(tx, &run.coupon_id, None).await?;
        }
        Ok(())
    }
}

/// Read-back check of a created coupon.
pub(crate) fn compare_coupon(run: &CouponRun, stored: Option<&CouponRow>) -> Verification {
    let mut mismatches = Vec::new();
    match stored {
        None => mismatches.push(format!("coupon {} is missing", run.coupon_id)),
        Some(c) => {
            let pairs = [
                ("code", c.discount_code.as_str(), run.code.as_str()),
                (
                    "promotion",
                    c.promotion_id.as_str(),
                    run.promotion_id.as_str(),
                ),
                (
                    "application",
                    c.application.as_str(),
                    run.application.as_str(),
                ),
                ("stacking", c.stacking.as_str(), run.stacking.as_str()),
                (
                    "tier",
                    audience(c.valid_for_member_tier.as_deref()),
                    audience(run.valid_for_member_tier.as_deref()),
                ),
            ];
            for (what, got, want) in pairs {
                if got != want {
                    mismatches.push(format!(
                        "{} {what} is {got} instead of {want}",
                        run.coupon_id
                    ));
                }
            }
        }
    }
    Verification {
        matched: mismatches.is_empty(),
        mismatches,
    }
}
