//! `submit_for_review`, `approve_product`, `reject_product` (docs/api/catalog.md#approval): a
//! draft goes to review, and a person publishes it in a change set or sends it back.
//!
//! ```text
//! draft ─submit─▶ pending_review ─approve (W$)─▶ active
//!   ▲                   └─reject─▶ rejected ─submit─┘
//! ```

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorClass,
    ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_policy::PolicySet;
use acom_store::{Cx, TxCx};
use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::product_not_found;
use crate::model::{Variant, money_text};
use crate::repo::{self, Group as GroupRow};
use crate::rules::{PriceFloorRule, ProductRules};
use crate::write::{self, DraftItem};
use crate::{BELOW_PRICE_FLOOR, PRODUCT_NOT_FOUND, neighbors, parse_time};

/// A product with nothing that could be reviewed: no variants yet.
pub const NOTHING_TO_REVIEW: ErrorCode = ErrorCode::new("nothing_to_review", ErrorClass::Domain);

/// Everything the review checks look at.
pub(crate) struct Facts {
    pub group: GroupRow,
    pub variants: Vec<Variant>,
    pub item: Option<DraftItem>,
    pub category_id: Option<String>,
}

pub(crate) async fn load(tx: &mut TxCx, product_id: &str) -> Result<Facts, ToolError> {
    let group = repo::group(tx, product_id)
        .await?
        .ok_or_else(|| product_not_found("product", product_id, "search_product_groups"))?;
    let ids = vec![group.id.clone()];
    let variants = repo::variants(tx, &ids).await?;
    let item = write::items_of(tx, &ids).await?.pop();
    let category_id = neighbors::categories(tx, Some(&ids))
        .await?
        .into_iter()
        .filter(|c| c.direct)
        .max_by_key(|c| c.is_primary)
        .map(|c| c.category_id);
    Ok(Facts {
        group,
        variants,
        item,
        category_id,
    })
}

fn joined(list: &[String]) -> String {
    list.join(", ")
}

/// `product.publish_requires`: what a product must have before it goes live.
pub(crate) fn publish_check(rules: &ProductRules, f: &Facts) -> Check {
    let selling: Vec<&Variant> = f
        .variants
        .iter()
        .filter(|v| v.availability.as_deref() != Some("discontinued"))
        .collect();
    let mut missing: Vec<String> = Vec::new();
    let mut fix = None;
    for r in &rules.publish_requires {
        match r.as_str() {
            "at_least_one_variant" if selling.is_empty() => {
                missing.push("a variant".into());
                fix = fix.or(Some("set_variants"));
            }
            "price_on_every_variant" => {
                let unpriced: Vec<String> = selling
                    .iter()
                    .filter(|v| v.price.is_none())
                    .map(|v| v.sku.clone())
                    .collect();
                if !unpriced.is_empty() {
                    missing.push(format!("a list price on {}", joined(&unpriced)));
                    fix = fix.or(Some("set_price"));
                }
            }
            "cost_on_every_variant" => {
                let costless: Vec<String> = selling
                    .iter()
                    .filter(|v| v.cost_value.is_none())
                    .map(|v| v.sku.clone())
                    .collect();
                if !costless.is_empty() {
                    missing.push(format!("a cost on {}", joined(&costless)));
                    fix = fix.or(Some("set_price"));
                }
            }
            "no_open_fields" => {
                if let Some(i) = f.item.as_ref().filter(|i| !i.open_fields.is_empty()) {
                    missing.push(format!(
                        "{} (open from the supplier sheet)",
                        joined(&i.open_fields)
                    ));
                    fix = fix.or(Some("update_product_group"));
                }
            }
            "description"
                if f.group
                    .description
                    .as_deref()
                    .is_none_or(|d| d.trim().is_empty()) =>
            {
                missing.push("a description in the store's tone".into());
                fix = fix.or(Some("update_product_group"));
            }
            _ => {}
        }
    }
    Check {
        rule: "product.publish_requires".into(),
        passed: missing.is_empty(),
        detail: Some(if missing.is_empty() {
            format!(
                "{} variant{} with prices and costs, a description, nothing open",
                selling.len(),
                if selling.len() == 1 { "" } else { "s" }
            )
        } else {
            format!("Still needs {}", missing.join("; "))
        }),
        fix_tool: fix.map(str::to_owned),
    }
}

/// `product.required_on_create`: a name, a category, and a seller (the store).
pub(crate) fn required_check(rules: &ProductRules, f: &Facts) -> Check {
    let mut missing = Vec::new();
    for r in &rules.required_on_create {
        match r.as_str() {
            "name" if f.group.name.trim().is_empty() => missing.push("a name"),
            "category" if f.category_id.is_none() => missing.push("a category"),
            _ => {}
        }
    }
    Check {
        rule: "product.required_on_create".into(),
        passed: missing.is_empty(),
        detail: Some(if missing.is_empty() {
            "It has a name and a category; the store is the seller".into()
        } else {
            format!(
                "It has no {}",
                missing.join(" and ").trim_start_matches("a ")
            )
        }),
        fix_tool: (!missing.is_empty()).then(|| "update_product_group".into()),
    }
}

/// `price_floor` on every priced variant.
pub(crate) fn floor_check(rule: &PriceFloorRule, f: &Facts) -> Check {
    let below: Vec<String> = f
        .variants
        .iter()
        .filter_map(|v| {
            let (price, cost) = (v.list_price()?, v.cost()?);
            (!rule.allows(price, cost)).then(|| {
                format!(
                    "{} at {} is under the floor {}",
                    v.sku,
                    money_text(&price),
                    money_text(&rule.floor(cost))
                )
            })
        })
        .collect();
    Check {
        rule: "price_floor".into(),
        passed: below.is_empty(),
        detail: Some(if below.is_empty() {
            format!("Every price at or above the floor ({})", rule.markup_text())
        } else {
            below.join("; ")
        }),
        fix_tool: (!below.is_empty()).then(|| "set_price".into()),
    }
}

/// `gift_card.disallow`: a gift card is not sold with the ways of selling the policy excludes.
pub(crate) fn gift_card_check(p: &PolicySet, f: &Facts) -> Check {
    let disallowed: Vec<String> = p
        .lookup("catalog.gift_card.disallow")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let gift = f.variants.iter().any(|v| v.sale_type == "gift_card");
    let clash: Vec<&str> = f
        .variants
        .iter()
        .map(|v| v.sale_type.as_str())
        .filter(|t| disallowed.iter().any(|d| d == t))
        .collect();
    let ok = !gift || clash.is_empty();
    Check {
        rule: "gift_card.disallow".into(),
        passed: ok,
        detail: Some(if !gift {
            "No gift card variants".into()
        } else if ok {
            "Gift cards are sold on their own".into()
        } else {
            format!("A gift card cannot be sold as {}", clash.join(", "))
        }),
        fix_tool: (!ok).then(|| "update_product".into()),
    }
}

fn invalid_state(product_id: &str, from: &str, allowed: &[&str], next: &'static str) -> ToolError {
    ToolError::new(
        ErrorCode::INVALID_STATE,
        format!(
            "{product_id} is {}; this needs it to be {}.",
            from.replace('_', " "),
            allowed
                .iter()
                .map(|a| a.replace('_', " "))
                .collect::<Vec<_>>()
                .join(" or ")
        ),
    )
    .detail(json!({"entity": "product", "from": from, "allowed_from": allowed}))
    .next(next)
}

/// What a review step runs: one product from one state to another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusPlan {
    pub product_id: String,
    pub from: String,
    pub to: String,
    /// `approve_product`: the release date to set, and the one before.
    #[serde(default)]
    pub release_date: Option<NaiveDate>,
    #[serde(default)]
    pub from_release_date: Option<NaiveDate>,
    #[serde(default)]
    pub note: Option<String>,
}

async fn move_status(tx: &mut TxCx, p: &StatusPlan, release: bool) -> Result<Committed, ToolError> {
    let n = write::set_approval_status(
        tx,
        &p.product_id,
        &p.from,
        &p.to,
        release.then_some(p.release_date),
    )
    .await?;
    if n == 0 {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!(
                "{} is no longer {}.",
                p.product_id,
                p.from.replace('_', " ")
            ),
        )
        .detail(json!({"entity": "product", "from": p.from}))
        .next("get_product_group"));
    }
    let mut produced = Map::new();
    produced.insert("product_id".into(), json!(p.product_id));
    Ok(Committed {
        id: p.product_id.clone(),
        produced,
    })
}

async fn verify_status(cx: &Cx, p: &StatusPlan) -> Result<Verification, ToolError> {
    let mut tx = cx.read().await.map_err(ToolError::from)?;
    let g = repo::group(&mut tx, &p.product_id).await?;
    tx.commit().await.map_err(ToolError::from)?;
    let now = g.map(|g| g.approval_status);
    let matched = now.as_deref() == Some(p.to.as_str());
    Ok(Verification {
        matched,
        mismatches: if matched {
            Vec::new()
        } else {
            vec![format!(
                "{} is {} instead of {}",
                p.product_id,
                now.as_deref().unwrap_or("missing"),
                p.to
            )]
        },
    })
}

async fn undo_status(tx: &mut TxCx, p: &StatusPlan, release: bool) -> Result<(), ToolError> {
    let back = StatusPlan {
        product_id: p.product_id.clone(),
        from: p.to.clone(),
        to: p.from.clone(),
        release_date: p.from_release_date,
        from_release_date: p.release_date,
        note: None,
    };
    move_status(tx, &back, release).await.map(|_| ())
}

fn status_change(p: &StatusPlan) -> Change {
    Change {
        op: ChangeOp::Update,
        entity: "product".into(),
        id: p.product_id.clone(),
        from: json!({"approval_status": p.from}),
        to: json!({"approval_status": p.to}),
        note: p.note.clone(),
    }
}

fn basis(g: &GroupRow) -> Vec<Basis> {
    vec![Basis {
        entity: "catalog.product_group".into(),
        id: g.id.clone(),
        version: i64::from(g.version),
    }]
}

fn note_of(s: Option<String>) -> Option<String> {
    s.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty())
}

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmitForReviewInput {
    /// The draft (or rejected) product to send to review.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_fw26_ot_014_chr")]
    pub product_id: String,
    /// A line for the reviewer.
    #[schemars(length(max = 500), example = &"Description written; ready to publish.")]
    pub note: Option<String>,
}

/// Send a product to review.
#[derive(Debug)]
pub struct SubmitForReview;

impl Operation for SubmitForReview {
    const META: OpMeta = OpMeta {
        address: "catalog.approval.submit",
        tool: "submit_for_review",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/products/{product_id}/submit",
        summary: "Send a draft (or rejected) product to review, once it has everything publishing needs. Publishing it is approve_product.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("product_id")],
        checks: &["product.publish_requires", "price_floor"],
        errors: &[
            ErrorCode::INVALID_STATE,
            NOTHING_TO_REVIEW,
            PRODUCT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SubmitForReviewInput;
    type Output = Value;
}

impl WriteOp<Cx> for SubmitForReview {
    type Plan = StatusPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SubmitForReviewInput,
    ) -> Result<Planned<StatusPlan>, ToolError> {
        let rules = ProductRules::from_policy(&cx.policy)?;
        let floor = PriceFloorRule::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let f = load(&mut tx, &input.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let from = f.group.approval_status.clone();
        if from != "draft" && from != "rejected" {
            return Err(invalid_state(
                &f.group.id,
                &from,
                &["draft", "rejected"],
                "get_product_group",
            ));
        }
        if f.variants.is_empty() {
            return Err(ToolError::new(
                NOTHING_TO_REVIEW,
                format!(
                    "{} has no variants yet, so there is nothing to review.",
                    f.group.id
                ),
            )
            .detail(json!({"entity": "product", "id": f.group.id}))
            .next("set_variants"));
        }
        let plan = StatusPlan {
            product_id: f.group.id.clone(),
            from,
            to: "pending_review".into(),
            release_date: None,
            from_release_date: None,
            note: note_of(input.note),
        };
        let approval = cx
            .policy
            .approval_for(Self::META.tool, &json!({"change_type": "content"}));
        Ok(Planned {
            changes: vec![status_change(&plan)],
            checks: vec![publish_check(&rules, &f), floor_check(&floor, &f)],
            basis: basis(&f.group),
            approval: match approval.reason {
                Some(reason) if approval.needed => ApprovalNeed::Needed { reason },
                _ => ApprovalNeed::NotNeeded,
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StatusPlan) -> Result<Committed, ToolError> {
        move_status(tx, plan, false).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &StatusPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        verify_status(cx, plan).await
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &StatusPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        undo_status(tx, plan, false).await
    }
}

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApproveProductInput {
    /// The product under review to publish.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_fw26_ot_014_chr")]
    pub product_id: String,
    /// The release date shoppers sort "newest" by, in store time. Default today. The product is
    /// shown as soon as the change set runs.
    #[schemars(example = &"2026-11-09T00:00")]
    pub valid_from: Option<String>,
    /// A line for the change history.
    #[schemars(length(max = 500), example = &"FW26 drop")]
    pub note: Option<String>,
}

/// Publish a reviewed product.
#[derive(Debug)]
pub struct ApproveProduct;

impl Operation for ApproveProduct {
    const META: OpMeta = OpMeta {
        address: "catalog.approval.approve",
        tool: "approve_product",
        group: Group::Offer,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/catalog/products/{product_id}/approve",
        summary: "Publish a product under review so shoppers can find and buy it. It reaches customers, so it always runs in a change set a person approves.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("product_id")],
        checks: &[
            "price_floor",
            "product.required_on_create",
            "product.publish_requires",
            "gift_card.disallow",
        ],
        errors: &[
            ErrorCode::INVALID_STATE,
            BELOW_PRICE_FLOOR,
            PRODUCT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::AUTHORIZE,
        person_scope: "",
    };
    type Input = ApproveProductInput;
    type Output = Value;
}

impl WriteOp<Cx> for ApproveProduct {
    type Plan = StatusPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ApproveProductInput,
    ) -> Result<Planned<StatusPlan>, ToolError> {
        let rules = ProductRules::from_policy(&cx.policy)?;
        let floor = PriceFloorRule::from_policy(&cx.policy)?;
        let release = parse_time(cx, "valid_from", input.valid_from.as_deref())?
            .unwrap_or_else(|| cx.clock.now());
        let release_date = release.with_timezone(&cx.time.zone).date_naive();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let f = load(&mut tx, &input.product_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if f.group.approval_status != "pending_review" {
            return Err(invalid_state(
                &f.group.id,
                &f.group.approval_status,
                &["pending_review"],
                "submit_for_review",
            ));
        }
        let plan = StatusPlan {
            product_id: f.group.id.clone(),
            from: "pending_review".into(),
            to: "active".into(),
            release_date: Some(release_date),
            from_release_date: f.group.release_date,
            note: note_of(input.note),
        };
        let mut change = status_change(&plan);
        change.to["release_date"] = json!(release_date.to_string());
        change.to["variants"] = json!(f.variants.len());
        Ok(Planned {
            changes: vec![change],
            checks: vec![
                floor_check(&floor, &f),
                required_check(&rules, &f),
                publish_check(&rules, &f),
                gift_card_check(&cx.policy, &f),
            ],
            basis: basis(&f.group),
            approval: ApprovalNeed::Needed {
                reason: format!("Publishes 1 product: {}", f.group.name),
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StatusPlan) -> Result<Committed, ToolError> {
        move_status(tx, plan, true).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &StatusPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        verify_status(cx, plan).await
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &StatusPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        undo_status(tx, plan, true).await
    }
}

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RejectProductInput {
    /// The product under review to send back.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_fw26_pt_011_snd")]
    pub product_id: String,
    /// Why, for whoever edits it next.
    #[schemars(length(min = 1, max = 500), example = &"The photo shows two trousers; shoot this color alone.")]
    pub reason: String,
}

/// Send a product under review back for editing.
#[derive(Debug)]
pub struct RejectProduct;

impl Operation for RejectProduct {
    const META: OpMeta = OpMeta {
        address: "catalog.approval.reject",
        tool: "reject_product",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/products/{product_id}/reject",
        summary: "Send a product under review back with a reason; it can be edited and submitted again.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("product_id")],
        checks: &[],
        errors: &[ErrorCode::INVALID_STATE, PRODUCT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::REJECT,
        person_scope: "",
    };
    type Input = RejectProductInput;
    type Output = Value;
}

impl WriteOp<Cx> for RejectProduct {
    type Plan = StatusPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RejectProductInput,
    ) -> Result<Planned<StatusPlan>, ToolError> {
        let Some(reason) = note_of(Some(input.reason)) else {
            return Err(ToolError::invalid_input("Say why the product goes back.")
                .detail(json!({"field": "reason", "reason": "missing"})));
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let g = repo::group(&mut tx, &input.product_id)
            .await?
            .ok_or_else(|| {
                product_not_found("product", &input.product_id, "search_product_groups")
            })?;
        tx.commit().await.map_err(ToolError::from)?;
        if g.approval_status != "pending_review" {
            return Err(invalid_state(
                &g.id,
                &g.approval_status,
                &["pending_review"],
                "get_change_history",
            ));
        }
        let plan = StatusPlan {
            product_id: g.id.clone(),
            from: "pending_review".into(),
            to: "rejected".into(),
            release_date: None,
            from_release_date: None,
            note: Some(reason),
        };
        Ok(Planned {
            changes: vec![status_change(&plan)],
            checks: Vec::new(),
            basis: basis(&g),
            approval: crate::policy_approval(
                cx,
                Self::META.tool,
                &json!({"change_type": "content"}),
            ),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &StatusPlan) -> Result<Committed, ToolError> {
        move_status(tx, plan, false).await
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &StatusPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        verify_status(cx, plan).await
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &StatusPlan,
        _d: &Committed,
    ) -> Result<(), ToolError> {
        undo_status(tx, plan, false).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::policies;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn variant(sku: &str, price: Option<&str>, cost: Option<&str>) -> Variant {
        Variant {
            id: format!("var_{sku}"),
            product_group_id: "prd_coat".into(),
            sku: sku.into(),
            name: "Coat".into(),
            color: None,
            size: Some("M".into()),
            sale_type: "always".into(),
            is_managed: true,
            offer_id: price.map(|_| format!("off_{sku}")),
            price: price.map(d),
            price_currency: price.map(|_| "USD".into()),
            availability: Some("in_stock".into()),
            inventory_level: Some(5),
            safety_stock: Some(0),
            cost_value: cost.map(d),
            cost_currency: cost.map(|_| "USD".into()),
        }
    }

    fn facts(description: Option<&str>, open: &[&str], variants: Vec<Variant>) -> Facts {
        Facts {
            group: GroupRow {
                id: "prd_coat".into(),
                name: "Wool blend overcoat, charcoal".into(),
                description: description.map(str::to_owned),
                brand_id: None,
                approval_status: "draft".into(),
                varies_by: vec!["size".into()],
                material: None,
                keywords: Vec::new(),
                additional_property: json!([]),
                image: None,
                has_merchant_return_policy_id: None,
                release_date: None,
                version: 1,
                updated_at: chrono::DateTime::UNIX_EPOCH,
            },
            variants,
            item: Some(DraftItem {
                product_import_id: "imp_x".into(),
                position: 1,
                product_group_id: "prd_coat".into(),
                supplier_code: "FW26-OT-014".into(),
                supplier_text: None,
                draft_status: if open.is_empty() {
                    "ready"
                } else {
                    "needs_you"
                }
                .into(),
                open_fields: open.iter().map(|s| (*s).to_owned()).collect(),
                draft_note: None,
            }),
            category_id: Some("cat_outerwear".into()),
        }
    }

    #[test]
    fn publishing_needs_a_description_and_no_open_fields() {
        let rules = ProductRules::from_policy(&policies()).unwrap();
        let f = facts(
            None,
            &["material"],
            vec![variant("c-m", Some("239.00"), Some("96.00"))],
        );
        let c = publish_check(&rules, &f);
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some(
                "Still needs material (open from the supplier sheet); a description in the store's tone"
            )
        );
        assert_eq!(c.fix_tool.as_deref(), Some("update_product_group"));
        let f = facts(
            Some("A coat."),
            &[],
            vec![variant("c-m", Some("239.00"), Some("96.00"))],
        );
        assert!(publish_check(&rules, &f).passed);
        let f = facts(Some("A coat."), &[], vec![variant("c-m", None, None)]);
        let c = publish_check(&rules, &f);
        assert_eq!(c.fix_tool.as_deref(), Some("set_price"));
    }

    #[test]
    fn the_floor_and_gift_cards_are_checked_before_publishing() {
        let floor = PriceFloorRule::from_policy(&policies()).unwrap();
        let f = facts(
            Some("x"),
            &[],
            vec![variant("c-m", Some("100.00"), Some("96.00"))],
        );
        let c = floor_check(&floor, &f);
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("c-m at $100.00 is under the floor $110.40")
        );
        let mut gift = variant("gc", Some("50.00"), Some("0.00"));
        gift.sale_type = "gift_card".into();
        let mut pre = variant("pre", Some("50.00"), Some("10.00"));
        pre.sale_type = "pre_order".into();
        let c = gift_card_check(&policies(), &facts(Some("x"), &[], vec![gift, pre]));
        assert!(!c.passed);
        assert_eq!(c.rule, "gift_card.disallow");
    }

    #[test]
    fn approving_always_waits_for_a_person_and_rejecting_needs_a_reason() {
        assert_eq!(ApproveProduct::META.kind, Kind::WriteApproval);
        assert!(
            serde_json::from_value::<RejectProductInput>(json!({"product_id": "prd_x"})).is_err()
        );
        let undo = StatusPlan {
            product_id: "prd_x".into(),
            from: "pending_review".into(),
            to: "active".into(),
            release_date: NaiveDate::from_ymd_opt(2026, 11, 6),
            from_release_date: None,
            note: None,
        };
        assert_eq!(status_change(&undo).to["approval_status"], "active");
    }
}
