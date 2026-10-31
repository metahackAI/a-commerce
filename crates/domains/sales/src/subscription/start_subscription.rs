//! The person-only `POST /v1/sales/subscriptions` (`start_subscription`): the shopper's Start
//! button starts the standing order a `prepare_subscription` preview describes. The server prices
//! it again and refuses a different amount; the subscribed lines leave the cart, and the first
//! order is scheduled for the first charge date (`subscription.renew`). Nothing is charged now.

use acom_changeset::LogEntry;
use acom_contract::{
    ActionType, Audience, Committed, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp,
    Slot, ToolError,
};
use acom_core::enums::ActionStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::prepare_subscription::{
    PREPARE_TOOL, PreparedSubscription, create_change, plan_from_cart,
};
use super::schedule_renewal;
use crate::delivery;
use crate::order::{checkout_view, planned_lines};
use crate::repo;
use crate::rules;
use crate::{
    ADDRESS_NOT_FOUND, CART_EMPTY, MIXED_ORDER_TYPE, NOT_CARD_PAYMENT, NOT_SIGNED_IN,
    PAYMENT_DECLINED,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StartSubscriptionInput {
    /// The `prepare_subscription` preview the shopper confirmed on the card.
    #[schemars(regex(pattern = r"^pv_[a-z0-9_]+$"), example = &"pv_2210")]
    pub preview_id: String,
}

/// Person-only: the shopper's Start button.
pub(crate) struct StartSubscription;

impl Operation for StartSubscription {
    const META: OpMeta = OpMeta {
        address: "sales.subscription.start",
        tool: "start_subscription",
        group: Group::Transact,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/sales/subscriptions",
        summary: "Start the standing order a prepare_subscription preview describes: priced again, the lines leave the cart, and an order is placed and charged to the saved card on every charge date.",
        consumes: &[Slot::one("preview_id")],
        produces: &[Slot::one("subscription_id")],
        checks: &["payment.subscription_card_only"],
        errors: &[
            ErrorCode::PREVIEW_EXPIRED,
            ErrorCode::ESTIMATE_CHANGED,
            PAYMENT_DECLINED,
            NOT_CARD_PAYMENT,
            CART_EMPTY,
            MIXED_ORDER_TYPE,
            ADDRESS_NOT_FOUND,
            NOT_SIGNED_IN,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::ORDER,
        person_scope: "human:order",
    };
    type Input = StartSubscriptionInput;
    type Output = Value;
}

fn started(done: &Committed) -> Value {
    json!({"ok": true, "id": done.id,
           "next_charge_date": done.produced.get("next_charge_date"),
           "undo": null})
}

impl PersonOp<Cx> for StartSubscription {
    async fn run(&self, tx: &mut TxCx, input: StartSubscriptionInput) -> Result<Value, ToolError> {
        let claimed = acom_changeset::claim_preview(tx, &input.preview_id, PREPARE_TOOL).await?;
        if let Some(done) = &claimed.already {
            return Ok(started(done));
        }
        let plan: PreparedSubscription =
            serde_json::from_value(claimed.plan.clone()).map_err(|e| {
                ToolError::new(ErrorCode::INTERNAL, format!("prepared subscription: {e}"))
            })?;
        if tx.principal.customer_id() != Some(plan.customer_id.as_str()) {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                "This subscription was prepared for someone else.",
            )
            .detail(json!({"reason": "owner"})));
        }
        if let Some(rule) = plan.failed_checks.first() {
            return Err(ToolError::new(
                ErrorCode::POLICY_VIOLATION,
                "The card showed a failed check; prepare the subscription again.",
            )
            .detail(json!({"rule": rule})));
        }

        // Price it again: what each order charges is what the shopper saw, or nothing starts.
        let rules = rules::subscription_rules(&tx.policy)?;
        let (carts, view) = checkout_view(tx, &plan.delivery, Some("credit_card")).await?;
        let (lines, total, checks) = plan_from_cart(&view, &rules, plan.total).map_err(|e| {
            if e.code == NOT_CARD_PAYMENT.code {
                ToolError::new(PAYMENT_DECLINED, "There is no saved card to charge.")
                    .detail(json!({"payment_method": "credit_card"}))
            } else {
                e
            }
        })?;
        if planned_lines(&view) != plan.cart_lines || total != plan.total {
            return Err(ToolError::new(
                ErrorCode::ESTIMATE_CHANGED,
                format!(
                    "Each order now comes to {total} instead of {}; prepare it again.",
                    plan.total
                ),
            )
            .detail(json!({"new_preview": {
                "changes": [create_change(&view, plan.billing_cycle, plan.next_charge_date, &lines, total)],
                "checks": checks}})));
        }
        if let Some(c) = checks.iter().find(|c| !c.passed) {
            return Err(ToolError::new(
                ErrorCode::POLICY_VIOLATION,
                c.detail.clone().unwrap_or_else(|| "A check failed.".into()),
            )
            .detail(json!({"rule": c.rule})));
        }
        let destination = view.destination.as_ref().ok_or_else(|| {
            ToolError::new(ADDRESS_NOT_FOUND, "There is no delivery address.")
                .detail(json!({"entity": "postal_address"}))
        })?;
        let order_delivery = json!({
            "address_id": destination.address_id,
            "delivery_address": destination.address,
            "address_label": delivery::full_label(destination.label.as_deref(), &destination.address),
        });
        let ordered_item = serde_json::to_value(&lines)
            .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
        repo::insert_subscription(
            tx,
            &repo::NewSubscription {
                id: &plan.subscription_id,
                customer_id: &plan.customer_id,
                billing_cycle: plan.billing_cycle.as_str(),
                next_charge_date: plan.next_charge_date,
                ordered_item: &ordered_item,
                order_delivery: &order_delivery,
                card_last4: view.card_last4.as_deref(),
                total,
                preview_id: &claimed.id,
            },
        )
        .await?;
        schedule_renewal(tx, &plan.subscription_id, plan.next_charge_date).await?;

        // The subscribed lines leave the cart (with guest lines folded into them).
        let mut taken: Vec<String> = Vec::new();
        for l in &plan.cart_lines {
            if let Some(m) = carts.line(&l.cart_line_id) {
                taken.push(m.id.clone());
                taken.extend(m.absorbs.clone());
            }
        }
        repo::delete_lines(tx, &taken).await?;
        for c in [&carts.main, &carts.guest].into_iter().flatten() {
            repo::touch_cart(tx, &c.id).await?;
        }

        let next = plan.next_charge_date.format("%Y-%m-%d").to_string();
        let mut produced = Map::new();
        produced.insert("subscription_id".into(), json!(plan.subscription_id));
        produced.insert("next_charge_date".into(), json!(next));
        produced.insert("amount".into(), json!(total));
        let done = Committed {
            id: plan.subscription_id.clone(),
            produced,
        };
        acom_changeset::mark_committed(tx, &claimed.id, &claimed.plan, &done).await?;
        acom_changeset::log_action(
            tx,
            LogEntry {
                object: Some(("subscription", &plan.subscription_id)),
                result: Some(
                    json!({"billing_cycle": plan.billing_cycle, "next_charge_date": next,
                                    "amount": total}),
                ),
                preview_id: Some(&claimed.id),
                ..LogEntry::new(
                    ActionType::ORDER.0,
                    ActionStatus::Completed,
                    Self::META.tool,
                )
            },
        )
        .await?;
        Ok(started(&done))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed_and_the_button_is_the_shoppers() {
        let s = closed_schema_for::<StartSubscriptionInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(StartSubscription::META.kind, Kind::Person);
        assert_eq!(StartSubscription::META.person_scope, "human:order");
    }

    /// Integration (`DATABASE_URL`, demo seed): Jamie puts the merino tee in her cart, prepares
    /// a monthly subscription for exactly what it comes to, and starts it; the subscription is
    /// rolled back and the cart line removed again.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_subscribes_to_the_merino_tee() {
        let cx = crate::testdb::cx("cus_jamie").await;
        let registry = crate::testdb::registry();
        let added = acom_changeset::preview(
            &registry,
            &cx,
            "add_to_cart",
            json!({"mode": "product_options", "product_id": "prd_merino_tee", "size": "M",
                   "color": "Charcoal", "order_quantity": 1}),
            None,
        )
        .await
        .unwrap();
        let line = acom_changeset::commit(&registry, &cx, "add_to_cart", &added.preview_id)
            .await
            .unwrap();
        let input = |total: Value| {
            json!({"billing_cycle": "every_month", "next_charge_date": "2026-12-01",
                   "delivery_mode": "saved", "delivery_address_id": "adr_home",
                   "client_total": total})
        };
        let first = acom_changeset::preview(
            &registry,
            &cx,
            "prepare_subscription",
            input(json!({"value": 1.00, "currency": "USD"})),
            None,
        )
        .await
        .unwrap();
        let amount = first.changes[0].to["amount"].clone();
        let shown = first
            .checks
            .iter()
            .find(|c| c.rule == "checkout_validation.server_total_matches_client")
            .unwrap()
            .passed;
        let p = acom_changeset::preview(
            &registry,
            &cx,
            "prepare_subscription",
            input(amount.clone()),
            None,
        )
        .await
        .unwrap();
        let mut tx = cx.begin().await.unwrap();
        let out = StartSubscription
            .run(
                &mut tx,
                StartSubscriptionInput {
                    preview_id: p.preview_id.clone(),
                },
            )
            .await;
        let row: Option<(String, String)> = match &out {
            Ok(v) => sqlx::query_as(
                "SELECT subscription_status, billing_cycle FROM sales.subscription
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind("store_northwind")
            .bind(v["id"].as_str().unwrap_or_default())
            .fetch_optional(&mut *tx.tx)
            .await
            .unwrap(),
            Err(_) => None,
        };
        let jobs: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.job WHERE tenant_id = $1 AND kind = 'subscription.renew'
               AND payload ->> 'charge_date' = '2026-12-01'",
        )
        .bind("store_northwind")
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        drop(tx); // rolled back
        let cart_line_id = line.produced["cart_line_id"].as_str().unwrap().to_owned();
        let removed = acom_changeset::preview(
            &registry,
            &cx,
            "remove_cart_item",
            json!({"cart_line_id": cart_line_id}),
            None,
        )
        .await
        .unwrap();
        acom_changeset::commit(&registry, &cx, "remove_cart_item", &removed.preview_id)
            .await
            .unwrap();

        assert!(!shown, "a wrong amount fails the check");
        // $58.00 under the $80.00 free-shipping subtotal: $6.00 shipping on every order.
        assert_eq!(amount["value"].to_string(), "64.00", "{amount}");
        let out = out.unwrap();
        assert_eq!(out["next_charge_date"], "2026-12-01");
        assert_eq!(row, Some(("active".into(), "every_month".into())));
        assert!(jobs >= 1, "the first order is scheduled");
    }
}
