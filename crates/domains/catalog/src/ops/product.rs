//! `get_product`, `get_product_measurements`, `get_return_policy`, `get_delivery_policy`,
//! `discontinue_product` (docs/api/catalog.md#product).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, ReadOp, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_core::enums::MeasurementName;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{not_found, product_not_found};
use crate::model::{plain, size_key};
use crate::repo::{self, Measurement};
use crate::rules;
use crate::viewer::Viewer;
use crate::{HAS_OPEN_SUBSCRIPTIONS, PRODUCT_NOT_FOUND, SIZE_NOT_FOUND, TIER_NOT_FOUND, cards};
use crate::{is_operator, project};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetProductInput {
    /// The product, as `search_products` returns it.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_windbreaker")]
    pub product_id: String,
    /// Operators only. The member tier whose price to show (`list_tiers`), or `everyone`.
    /// Default everyone.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub valid_for_member_tier: Option<String>,
    /// Operators only. The moment to price at, in store time. Default now.
    #[schemars(example = &"2026-11-07T00:00:00-05:00")]
    pub as_of: Option<String>,
    /// Only these top-level fields.
    #[schemars(example = ["id", "price", "has_variant"])]
    pub fields: Option<Vec<String>>,
}

/// One product for selling: list price, the caller's member price, stock per variant, and the
/// attributes used for comparing.
#[derive(Debug)]
pub struct GetProduct;

impl Operation for GetProduct {
    const META: OpMeta = OpMeta {
        address: "catalog.product.get",
        tool: "get_product",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/products/{product_id}",
        summary: "One product as it sells: list price, the member price that applies to the caller, total and per-variant stock, the variant ids to add to a cart, and the attributes used to compare. Operators also see cost.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("variant_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND, TIER_NOT_FOUND, ErrorCode::FORBIDDEN],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetProductInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetProduct {
    async fn read(&self, cx: &Cx, input: GetProductInput) -> Result<Value, ToolError> {
        let tier = input.valid_for_member_tier.as_deref();
        let as_of = input.as_of.as_deref();
        Viewer::check_inputs(cx, tier, as_of)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let viewer = Viewer::resolve(cx, &mut tx, tier, as_of).await?;
        let card = cards::load(&mut tx, std::slice::from_ref(&input.product_id), &viewer)
            .await?
            .into_iter()
            .next();
        tx.commit().await.map_err(ToolError::from)?;
        let card =
            card.ok_or_else(|| not_found("product", &input.product_id, "search_products"))?;
        Ok(project(
            card.product_json(&viewer, &cx.time),
            &input.fields.unwrap_or_default(),
        ))
    }
}

/// A product the caller may see: shoppers only see active ones.
pub(crate) async fn visible_group(
    cx: &Cx,
    tx: &mut TxCx,
    product_id: &str,
) -> Result<Option<repo::Group>, ToolError> {
    Ok(repo::group(tx, product_id)
        .await?
        .filter(|g| is_operator(&cx.principal) || g.approval_status == "active"))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetProductMeasurementsInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_wool_overcoat")]
    pub product_id: String,
    /// Only these sizes, values of the product's size axis. Default all.
    #[schemars(example = ["M", "L"])]
    pub sizes: Option<Vec<String>>,
}

/// Garment measurements per size, for fit questions.
#[derive(Debug)]
pub struct GetProductMeasurements;

impl Operation for GetProductMeasurements {
    const META: OpMeta = OpMeta {
        address: "catalog.product.get_measurements",
        tool: "get_product_measurements",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/products/{product_id}/measurements",
        summary: "Garment measurements per size in cm (chest, shoulder, sleeve, length, …), with the same names on every product so sizes can be compared with what the shopper already owns.",
        consumes: &[Slot::one("product_id")],
        produces: &[],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND, SIZE_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetProductMeasurementsInput;
    type Output = Value;
}

/// The size chart: measurement names in their usual order and one row per size.
fn measurement_chart(rows: &[Measurement], only: &[String]) -> Result<Value, ToolError> {
    let mut sizes: Vec<&str> = rows.iter().map(|r| r.size.as_str()).collect();
    sizes.sort_by_key(|s| size_key(Some(s)));
    sizes.dedup();
    for s in only {
        if !sizes.iter().any(|x| x.eq_ignore_ascii_case(s)) {
            return Err(
                ToolError::new(SIZE_NOT_FOUND, format!("The size chart has no size {s}."))
                    .detail(json!({"field": "sizes", "size": s, "allowed": sizes}))
                    .next("get_product"),
            );
        }
    }
    let names: Vec<&str> = MeasurementName::WORDS
        .iter()
        .copied()
        .filter(|n| rows.iter().any(|r| r.name == *n))
        .collect();
    let table: Vec<Value> = sizes
        .iter()
        .filter(|s| only.is_empty() || only.iter().any(|o| o.eq_ignore_ascii_case(s)))
        .map(|s| {
            let mut row = Map::new();
            row.insert("size".into(), json!(s));
            for n in &names {
                if let Some(m) = rows.iter().find(|r| r.size == *s && r.name == *n) {
                    row.insert((*n).into(), plain(m.value));
                }
            }
            Value::Object(row)
        })
        .collect();
    Ok(json!({
        "unit": rows.first().map_or("cm", |r| r.unit_text.as_str()),
        "measurements": names,
        "sizes": table,
        "how_measured": rows.iter().find_map(|r| r.measurement_technique.clone()),
    }))
}

impl ReadOp<Cx> for GetProductMeasurements {
    async fn read(&self, cx: &Cx, input: GetProductMeasurementsInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = visible_group(cx, &mut tx, &input.product_id).await?;
        let rows = match group {
            Some(_) => repo::measurements(&mut tx, &input.product_id).await?,
            None => Vec::new(),
        };
        tx.commit().await.map_err(ToolError::from)?;
        if group.is_none() {
            return Err(not_found("product", &input.product_id, "search_products"));
        }
        if rows.is_empty() {
            return Err(not_found(
                "measurement_chart",
                &input.product_id,
                "get_review_summary",
            ));
        }
        let mut out = json!({"product_id": input.product_id});
        let chart = measurement_chart(&rows, &input.sizes.unwrap_or_default())?;
        if let (Value::Object(o), Value::Object(c)) = (&mut out, chart) {
            o.extend(c);
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductIdInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_wool_overcoat")]
    pub product_id: String,
}

/// The return terms of a product for its page.
#[derive(Debug)]
pub struct GetReturnPolicy;

impl Operation for GetReturnPolicy {
    const META: OpMeta = OpMeta {
        address: "catalog.product.get_return_policy",
        tool: "get_return_policy",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/products/{product_id}/return-policy",
        summary: "The return policy that applies to one product: return days, who pays return shipping, when it cannot be returned, and the free size-exchange window.",
        consumes: &[Slot::one("product_id")],
        produces: &[],
        checks: &[],
        errors: &[PRODUCT_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ProductIdInput;
    type Output = Value;
}

/// The store's return conditions (`fulfillment.yml return.non_returnable_when`) that hold for a
/// whole product: today only being a gift card is a property of the product itself.
fn non_returnable_when(conditions: &[String], sale_types: &[String]) -> Vec<String> {
    conditions
        .iter()
        .filter(|c| sale_types.iter().any(|s| s == *c))
        .cloned()
        .collect()
}

fn words(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

impl ReadOp<Cx> for GetReturnPolicy {
    async fn read(&self, cx: &Cx, input: ProductIdInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = visible_group(cx, &mut tx, &input.product_id)
            .await?
            .ok_or_else(|| product_not_found("product", &input.product_id, "search_products"))?;
        let terms = match &group.has_merchant_return_policy_id {
            Some(id) => repo::return_policy(&mut tx, id).await?,
            None => None,
        };
        let sale_types: Vec<String> = repo::variants(&mut tx, std::slice::from_ref(&group.id))
            .await?
            .into_iter()
            .map(|v| v.sale_type)
            .collect();
        tx.commit().await.map_err(ToolError::from)?;
        let conditions = words(cx.policy.lookup("fulfillment.return.non_returnable_when"));
        let mut out = match terms {
            Some(t) => json!({
                "product_id": group.id, "merchant_return_policy_id": t.id, "name": t.name,
                "merchant_return_days": t.merchant_return_days, "return_fees": t.return_fees,
                "return_method": t.return_method, "refund_type": t.refund_type,
                "acom:size_exchange_days": t.size_exchange_days}),
            // No policy of its own: the store defaults in fulfillment.yml.
            None => json!({
                "product_id": group.id, "merchant_return_policy_id": null,
                "name": "Store default",
                "merchant_return_days": cx.policy.lookup("fulfillment.return_window_days"),
                "return_fees": cx.policy.lookup("fulfillment.changed_mind_return_shipping"),
                "return_method": words(cx.policy.lookup("fulfillment.return.send_back_methods")),
                "refund_type": "full_refund",
                "acom:size_exchange_days": null}),
        };
        out["acom:non_returnable_when"] = json!(non_returnable_when(&conditions, &sale_types));
        Ok(out)
    }
}

/// The delivery terms of a product.
#[derive(Debug)]
pub struct GetDeliveryPolicy;

impl Operation for GetDeliveryPolicy {
    const META: OpMeta = OpMeta {
        address: "catalog.product.get_delivery_policy",
        tool: "get_delivery_policy",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/products/{product_id}/delivery-policy",
        summary: "The shipping terms of one product: fee type, standard rate, the subtotal that ships free, and handling days.",
        consumes: &[Slot::one("product_id")],
        produces: &[],
        checks: &[],
        errors: &[PRODUCT_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ProductIdInput;
    type Output = Value;
}

/// Shipping terms from the policies: nothing to ship for products sold only as types that are
/// not shipped (`delivery.no_ship_for`), else free over the threshold, else the standard rate.
fn delivery_terms(policy: &acom_policy::PolicySet, sale_types: &[String]) -> Value {
    let rate = policy
        .lookup("fulfillment.delivery.standard_rate")
        .and_then(rules::money);
    let free_over = policy
        .lookup("order.free_shipping_min_subtotal.amount")
        .and_then(rules::money);
    let handling = policy.lookup("fulfillment.delivery.handling_days").cloned();
    let no_ship = words(policy.lookup("fulfillment.delivery.no_ship_for"));
    let not_shipped = !sale_types.is_empty() && sale_types.iter().all(|s| no_ship.contains(s));
    if not_shipped {
        return json!({"shipping_fee_type": "free",
                      "shipping_rate": rate.map(|r| acom_core::Money::zero(r.currency)),
                      "free_shipping_min_subtotal": null, "handling_days": 0});
    }
    let fee_type = match (&rate, &free_over) {
        (_, Some(_)) => "free_over_min_subtotal",
        (Some(r), None) if r.value.is_zero() => "free",
        _ => "paid",
    };
    json!({"shipping_fee_type": fee_type, "shipping_rate": rate,
           "free_shipping_min_subtotal": free_over, "handling_days": handling})
}

impl ReadOp<Cx> for GetDeliveryPolicy {
    async fn read(&self, cx: &Cx, input: ProductIdInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = visible_group(cx, &mut tx, &input.product_id)
            .await?
            .ok_or_else(|| product_not_found("product", &input.product_id, "search_products"))?;
        let sale_types: Vec<String> = repo::variants(&mut tx, std::slice::from_ref(&group.id))
            .await?
            .into_iter()
            .map(|v| v.sale_type)
            .collect();
        tx.commit().await.map_err(ToolError::from)?;
        let mut out = json!({"product_id": group.id});
        if let (Value::Object(o), Value::Object(t)) =
            (&mut out, delivery_terms(&cx.policy, &sale_types))
        {
            o.extend(t);
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscontinueProductInput {
    /// The variant to stop selling for good.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_linen_shirt_xs_white")]
    pub variant_id: String,
    /// Why, in a sentence; shown in the change set.
    #[schemars(length(max = 500), example = &"The mill stopped making this linen.")]
    pub reason: Option<String>,
}

/// What `discontinue_product` runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscontinuePlan {
    pub variant_id: String,
    pub offer_id: String,
    /// Availability before; `discontinued` when there was nothing to do.
    pub from_availability: String,
    pub reason: Option<String>,
}

/// Stop selling a variant for good.
#[derive(Debug)]
pub struct DiscontinueProduct;

impl Operation for DiscontinueProduct {
    const META: OpMeta = OpMeta {
        address: "catalog.product.discontinue",
        tool: "discontinue_product",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/variants/{variant_id}/discontinue",
        summary: "Discontinue a variant (availability becomes discontinued; cannot be undone). A product whose variants are all discontinued shows as discontinued.",
        consumes: &[Slot::one("variant_id")],
        produces: &[Slot::one("variant_id")],
        checks: &["product.discontinue"],
        errors: &[
            PRODUCT_NOT_FOUND,
            ErrorCode::INVALID_STATE,
            HAS_OPEN_SUBSCRIPTIONS,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = DiscontinueProductInput;
    type Output = Value;
}

/// The `product.discontinue` check. Subscriptions are not stored yet (no sales table holds
/// them), so none can be open.
fn discontinue_check(policy: &acom_policy::PolicySet) -> Check {
    let blocks = policy
        .lookup("catalog.product.discontinue.block_if_open_subscriptions")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    Check {
        rule: "product.discontinue".into(),
        passed: true,
        detail: Some(if blocks {
            "No open subscriptions; discontinuing cannot be undone".into()
        } else {
            "Subscriptions are not checked; discontinuing cannot be undone".into()
        }),
        fix_tool: None,
    }
}

impl WriteOp<Cx> for DiscontinueProduct {
    type Plan = DiscontinuePlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: DiscontinueProductInput,
    ) -> Result<Planned<DiscontinuePlan>, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let state = repo::offer_state(&mut tx, &input.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let s = state
            .ok_or_else(|| product_not_found("variant", &input.variant_id, "search_products"))?;
        let reason = input
            .reason
            .map(|r| r.trim().to_owned())
            .filter(|r| !r.is_empty());
        let plan = DiscontinuePlan {
            variant_id: s.variant_id.clone(),
            offer_id: s.offer_id.clone(),
            from_availability: s.availability.clone(),
            reason: reason.clone(),
        };
        let basis = vec![Basis {
            entity: "catalog.offer".into(),
            id: s.offer_id.clone(),
            version: i64::from(s.version),
        }];
        if s.availability == "discontinued" {
            return Ok(Planned {
                plan,
                changes: Vec::new(),
                checks: vec![Check {
                    rule: "product.discontinue".into(),
                    passed: true,
                    detail: Some("Already discontinued; nothing changes".into()),
                    fix_tool: None,
                }],
                basis,
                approval: ApprovalNeed::NotNeeded,
            });
        }
        let approval = cx.policy.approval_for(
            Self::META.tool,
            &json!({"change_type": "structural", "changed_offers": 1}),
        );
        Ok(Planned {
            plan,
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "variant".into(),
                id: s.variant_id,
                from: json!({"availability": s.availability}),
                to: json!({"availability": "discontinued"}),
                note: reason,
            }],
            checks: vec![discontinue_check(&cx.policy)],
            basis,
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: "Discontinues 1 variant; this cannot be undone".into(),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &DiscontinuePlan) -> Result<Committed, ToolError> {
        repo::discontinue_offer(tx, &plan.offer_id).await?;
        let mut produced = Map::new();
        produced.insert("variant_id".into(), json!(plan.variant_id));
        Ok(Committed {
            id: plan.variant_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &DiscontinuePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let state = repo::offer_state(&mut tx, &plan.variant_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let now = state.map(|s| s.availability);
        let matched = now.as_deref() == Some("discontinued");
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} is {} instead of discontinued",
                    plan.variant_id,
                    now.as_deref().unwrap_or("missing")
                )]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &DiscontinuePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!("Discontinuing {} cannot be taken back.", plan.variant_id),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn m(size: &str, name: &str, value: &str) -> Measurement {
        Measurement {
            size: size.into(),
            name: name.into(),
            value: Decimal::from_str(value).unwrap_or_default(),
            unit_text: "cm".into(),
            measurement_technique: Some("Garment laid flat.".into()),
        }
    }

    /// F3: the wool overcoat M 56/46/64/98 and L 59/48/65.5/100.
    fn overcoat() -> Vec<Measurement> {
        vec![
            m("L", "length", "100.0"),
            m("M", "chest", "56.0"),
            m("L", "chest", "59.0"),
            m("M", "shoulder", "46.0"),
            m("L", "shoulder", "48.0"),
            m("M", "sleeve", "64.0"),
            m("L", "sleeve", "65.5"),
            m("M", "length", "98.0"),
        ]
    }

    #[test]
    fn size_chart_matches_the_fit_check() {
        let chart = measurement_chart(&overcoat(), &[]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            chart["measurements"],
            json!(["chest", "shoulder", "sleeve", "length"])
        );
        assert_eq!(
            chart["sizes"],
            json!([
                {"size": "M", "chest": 56, "shoulder": 46, "sleeve": 64, "length": 98},
                {"size": "L", "chest": 59, "shoulder": 48, "sleeve": 65.5, "length": 100}])
        );
        let only_l =
            measurement_chart(&overcoat(), &["l".into()]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(only_l["sizes"].as_array().map(Vec::len), Some(1));
        let e = measurement_chart(&overcoat(), &["XS".into()]).unwrap_err();
        assert_eq!(e.code, "size_not_found");
        assert_eq!(
            e.detail.map(|d| d["allowed"].clone()),
            Some(json!(["M", "L"]))
        );
    }

    #[test]
    fn only_gift_cards_are_unreturnable_as_a_product() {
        let conditions = ["used".to_owned(), "final_sale".into(), "gift_card".into()];
        assert!(non_returnable_when(&conditions, &["always".into()]).is_empty());
        assert_eq!(
            non_returnable_when(&conditions, &["gift_card".into()]),
            vec!["gift_card".to_owned()]
        );
    }

    #[test]
    fn delivery_terms_come_from_the_policies() {
        let p = crate::testing::policies();
        let t = delivery_terms(&p, &["always".into()]);
        assert_eq!(t["shipping_fee_type"], "free_over_min_subtotal");
        assert_eq!(t["shipping_rate"]["value"].to_string(), "6.00");
        assert_eq!(t["shipping_rate"]["currency"], "USD");
        assert_eq!(
            t["free_shipping_min_subtotal"]["value"].to_string(),
            "80.00"
        );
        assert_eq!(t["handling_days"], json!(1));
        let gift = delivery_terms(&p, &["gift_card".into()]);
        assert_eq!(gift["shipping_fee_type"], "free");
    }

    #[test]
    fn discontinuing_is_checked_and_needs_approval_by_policy() {
        let p = crate::testing::policies();
        assert!(discontinue_check(&p).passed);
        let a = p.approval_for(
            DiscontinueProduct::META.tool,
            &json!({"change_type": "structural", "changed_offers": 1}),
        );
        assert!(a.needed);
    }

    #[tokio::test]
    async fn shoppers_cannot_ask_for_another_tier() {
        let e = GetProduct
            .read(
                &testing::shopper(),
                GetProductInput {
                    product_id: "prd_windbreaker".into(),
                    valid_for_member_tier: Some("vip".into()),
                    as_of: None,
                    fields: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        assert_eq!(e.detail.map(|d| d["reason"].clone()), Some(json!("scope")));
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn windbreaker_sells_at_list_price_on_friday() {
        let cx = testing::operator_db().await;
        let out = GetProduct
            .read(
                &cx,
                GetProductInput {
                    product_id: "prd_windbreaker".into(),
                    valid_for_member_tier: Some("vip".into()),
                    as_of: None,
                    fields: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["price"].to_string(), "89.00");
        assert_eq!(out["cost"].to_string(), "36.00");
        assert_eq!(out["inventory_level"], json!(214));
        assert_eq!(out["member_price"], Value::Null);
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn overcoat_chart_and_policies_read_back() {
        let cx = testing::shopper_db().await;
        let chart = GetProductMeasurements
            .read(
                &cx,
                GetProductMeasurementsInput {
                    product_id: "prd_wool_overcoat".into(),
                    sizes: Some(vec!["M".into(), "L".into()]),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(chart["sizes"][1]["sleeve"].to_string(), "65.5");
        let returns = GetReturnPolicy
            .read(
                &cx,
                ProductIdInput {
                    product_id: "prd_wool_overcoat".into(),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(returns["return_fees"], "free_return");
        assert_eq!(returns["acom:size_exchange_days"], json!(30));
        let delivery = GetDeliveryPolicy
            .read(
                &cx,
                ProductIdInput {
                    product_id: "prd_windbreaker".into(),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(delivery["shipping_fee_type"], "free_over_min_subtotal");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn discontinuing_needs_approval_and_changes_nothing_in_preview() {
        let cx = testing::operator_db().await;
        let p = DiscontinueProduct
            .preview(
                &cx,
                DiscontinueProductInput {
                    variant_id: "var_linen_shirt_xs_white".into(),
                    reason: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(p.approval, ApprovalNeed::Needed { .. }));
        assert_eq!(p.changes[0].to, json!({"availability": "discontinued"}));
    }
}
