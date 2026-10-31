//! `get_return_policy` (docs/api/catalog.md#product).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use serde_json::{Value, json};

use super::{ProductIdInput, words};
use crate::repo::{self};
use crate::visible_group;
use crate::{PRODUCT_NOT_FOUND, product_not_found};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_gift_cards_are_unreturnable_as_a_product() {
        let conditions = ["used".to_owned(), "final_sale".into(), "gift_card".into()];
        assert!(non_returnable_when(&conditions, &["always".into()]).is_empty());
        assert_eq!(
            non_returnable_when(&conditions, &["gift_card".into()]),
            vec!["gift_card".to_owned()]
        );
    }
}
