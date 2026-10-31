//! `get_delivery_policy` (docs/api/catalog.md#product).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use serde_json::{Value, json};

use super::{ProductIdInput, words};
use crate::repo::{self};
use crate::visible_group;
use crate::{PRODUCT_NOT_FOUND, product_not_found, rules};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
