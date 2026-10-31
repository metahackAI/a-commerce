//! `prepare_subscription` (`sales.subscription.prepare`): a standing order from the caller's own
//! cart, for the shopper's Start button. Preview only (`approval.preview_only`); the shopper
//! starts it with the person-only `POST /v1/sales/subscriptions` (`start_subscription`).

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::BillingCycle;
use acom_store::{Cx, TxCx};
use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{SubscriptionLine, priced};
use crate::checkout;
use crate::delivery;
use crate::order::{
    DeliveryMode, PlannedLine, PostalAddressInput, checkout_view, delivery_choice, planned_lines,
};
use crate::pricing::{CartView, DeliveryChoice};
use crate::rules::{self, SubscriptionRules};
use crate::{
    ADDRESS_NOT_DELIVERABLE, ADDRESS_NOT_FOUND, CART_EMPTY, MIXED_ORDER_TYPE, NOT_CARD_PAYMENT,
    NOT_SIGNED_IN, approval, parse_day, today,
};

pub(crate) const PREPARE_TOOL: &str = "prepare_subscription";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrepareSubscriptionInput {
    /// How often an order is placed: `every_week`, `every_2_weeks` or `every_month`, as the store
    /// offers (`subscription.billing_cycles`).
    #[schemars(example = &"every_month")]
    pub billing_cycle: BillingCycle,
    /// The first charge date, a store-time day after today.
    #[schemars(regex(pattern = r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"), example = &"2026-12-01")]
    pub next_charge_date: String,
    /// `saved`: a saved address (`delivery_address_id`); `one_time`: an address for this
    /// subscription only (`delivery_address`, not saved).
    #[schemars(example = &"saved")]
    pub delivery_mode: DeliveryMode,
    /// A saved address (`get_my_profile`, `list_addresses`). Needed with `saved`.
    #[schemars(regex(pattern = r"^adr_[a-z0-9_]+$"), example = &"adr_home")]
    pub delivery_address_id: Option<String>,
    /// The address for this subscription. Needed with `one_time`.
    #[schemars(example = json!({"name": "Jamie", "street_address": "12 Harbor St, Apt 3",
        "address_locality": "Brooklyn", "address_region": "NY", "postal_code": "11201",
        "address_country": "US"}))]
    pub delivery_address: Option<PostalAddressInput>,
    /// What one order comes to as the shopper saw it; the server recalculates and compares.
    #[schemars(example = json!({"value": 58.00, "currency": "USD"}))]
    pub client_total: Money,
}

/// What `prepare_subscription` hands to the shopper's Start button.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PreparedSubscription {
    pub subscription_id: String,
    pub customer_id: String,
    pub billing_cycle: BillingCycle,
    pub next_charge_date: NaiveDate,
    /// The cart lines it takes, to see they did not change.
    pub cart_lines: Vec<PlannedLine>,
    pub delivery: DeliveryChoice,
    /// What each order comes to.
    pub total: Money,
    /// Checks that failed on the card; such a subscription cannot start.
    pub failed_checks: Vec<String>,
}

pub(crate) struct PrepareSubscription;

impl Operation for PrepareSubscription {
    const META: OpMeta = OpMeta {
        address: "sales.subscription.prepare",
        tool: "prepare_subscription",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/sales/subscriptions/prepare",
        summary: "Prepare a standing order from the caller's own cart: how often, the first charge date, the saved card, the address, and what each order comes to with the subscription discounts. Preview only; only the shopper's Start button starts it.",
        consumes: &[Slot::one("cart_id")],
        produces: &[Slot::one("preview_id")],
        checks: &[
            "payment.subscription_card_only",
            "subscription.billing_cycles",
            "subscription.discount_layers",
            "checkout_validation.stock_available",
            "checkout_validation.server_total_matches_client",
        ],
        errors: &[
            NOT_CARD_PAYMENT,
            CART_EMPTY,
            MIXED_ORDER_TYPE,
            ADDRESS_NOT_FOUND,
            ADDRESS_NOT_DELIVERABLE,
            NOT_SIGNED_IN,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::ORDER,
        person_scope: "",
    };
    type Input = PrepareSubscriptionInput;
    type Output = Value;
}

fn bad(message: String, field: &str, reason: &str) -> ToolError {
    ToolError::invalid_input(message)
        .detail(json!({"field": field, "reason": reason}))
        .next(PREPARE_TOOL)
}

/// The billing cycle the store offers, or `invalid_input` naming the ones it does.
pub(crate) fn offered(
    rules: &SubscriptionRules,
    cycle: BillingCycle,
    next: &'static str,
) -> Result<(), ToolError> {
    match rules.cycle(cycle.as_str()) {
        Some(_) => Ok(()),
        None => Err(ToolError::invalid_input(format!(
            "This store does not offer {} subscriptions.",
            cycle.as_str().replace('_', " ")
        ))
        .detail(json!({"field": "billing_cycle", "reason": "enum",
                       "allowed": rules.billing_cycles,
                       "policy_key": "subscription.billing_cycles"}))
        .next(next)),
    }
}

/// A first or next charge date: a store-time day after today.
pub(crate) fn future_day(
    s: &str,
    today: NaiveDate,
    next: &'static str,
) -> Result<NaiveDate, ToolError> {
    let day = parse_day("next_charge_date", s).map_err(|e| e.next(next))?;
    if day <= today {
        return Err(ToolError::invalid_input(format!(
            "The charge date {day} is not after today ({today})."
        ))
        .detail(json!({"field": "next_charge_date", "reason": "past"}))
        .next(next));
    }
    Ok(day)
}

/// The card a subscription charges, or `not_card_payment` (`payment.subscription_card_only`).
pub(crate) fn card_check(card_last4: Option<&str>, card_only: bool) -> Result<Check, ToolError> {
    match card_last4 {
        Some(last4) => Ok(Check {
            rule: "payment.subscription_card_only".into(),
            passed: true,
            detail: Some(checkout::payment_label("credit_card", Some(last4))),
            fix_tool: None,
        }),
        None => Err(ToolError::new(
            NOT_CARD_PAYMENT,
            if card_only {
                "Subscriptions are charged to a saved card, and there is none."
            } else {
                "Subscriptions here are charged to a saved card, and there is none."
            },
        )
        .detail(json!({"policy_key": "payment.subscription_card_only"}))
        .next("get_payment_methods")),
    }
}

/// The subscription a cart would make: its lines, what each order comes to, and its checks.
pub(crate) fn plan_from_cart(
    view: &CartView,
    rules: &SubscriptionRules,
    client_total: Money,
) -> Result<(Vec<SubscriptionLine>, Money, Vec<Check>), ToolError> {
    let card = card_check(view.card_last4.as_deref(), rules.card_only)?;
    let (lines, total) = priced(&view.lines, &view.breakdown, &rules.discount_layers);
    let mut checks = vec![card];
    checks.extend(checkout::checkout_checks(
        view,
        &["stock_available".to_owned()],
        client_total,
        None,
    ));
    let ok = total.currency == client_total.currency && total.value == client_total.value;
    checks.push(Check {
        rule: "checkout_validation.server_total_matches_client".into(),
        passed: ok,
        detail: Some(if ok {
            format!("{} as shown", total.value)
        } else {
            format!(
                "Recalculated with the subscription discounts: {}, but {} was shown",
                total.value, client_total.value
            )
        }),
        fix_tool: (!ok).then(|| "get_cart".to_owned()),
    });
    Ok((lines, total, checks))
}

/// The `create subscription` change the preview shows.
pub(crate) fn create_change(
    view: &CartView,
    cycle: BillingCycle,
    next: NaiveDate,
    lines: &[SubscriptionLine],
    total: Money,
) -> Change {
    Change {
        op: ChangeOp::Create,
        entity: "subscription".into(),
        id: String::new(),
        from: Value::Null,
        to: json!({
            "billing_cycle": cycle,
            "next_charge_date": next.format("%Y-%m-%d").to_string(),
            "amount": total,
            "lines": lines.iter().map(|l| json!({
                "variant_id": l.variant_id, "name": l.name, "size": l.size, "color": l.color,
                "quantity": l.quantity, "price": l.price, "discount": l.discount,
            })).collect::<Vec<_>>(),
            "delivery": view.destination.as_ref().map(|d| json!({
                "address_id": d.address_id,
                "address_label": delivery::full_label(d.label.as_deref(), &d.address),
            })),
            "payment": {"payment_method": "credit_card",
                        "label": checkout::payment_label("credit_card", view.card_last4.as_deref())},
        }),
        note: None,
    }
}

impl WriteOp<Cx> for PrepareSubscription {
    type Plan = PreparedSubscription;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: PrepareSubscriptionInput,
    ) -> Result<Planned<PreparedSubscription>, ToolError> {
        let rules = rules::subscription_rules(&cx.policy)?;
        offered(&rules, input.billing_cycle, PREPARE_TOOL)?;
        let today = today(cx.clock.as_ref(), &cx.time);
        let next = future_day(&input.next_charge_date, today, PREPARE_TOOL)?;
        let delivery = delivery_choice(
            input.delivery_mode,
            input.delivery_address_id.as_ref(),
            input.delivery_address.as_ref(),
        )
        .map_err(|e| e.next(PREPARE_TOOL))?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (carts, view) = checkout_view(&mut tx, &delivery, Some("credit_card")).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let (lines, total, checks) = plan_from_cart(&view, &rules, input.client_total)?;
        if lines.is_empty() {
            return Err(bad(
                "The cart has nothing to subscribe to.".into(),
                "cart",
                "empty",
            ));
        }
        let plan = PreparedSubscription {
            subscription_id: format!("sub_{}", acom_core::id::new_suffix()),
            customer_id: view.customer_id.clone().unwrap_or_default(),
            billing_cycle: input.billing_cycle,
            next_charge_date: next,
            cart_lines: planned_lines(&view),
            delivery,
            total,
            failed_checks: checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| c.rule.clone())
                .collect(),
        };
        Ok(Planned {
            changes: vec![create_change(
                &view,
                input.billing_cycle,
                next,
                &lines,
                total,
            )],
            checks,
            basis: carts.basis(),
            approval: approval(cx, Self::META.tool, &json!({"amount": total})),
            plan,
        })
    }

    /// Never run by the preview engine (`approval.preview_only`): the shopper's Start button
    /// starts the subscription through `start_subscription`.
    async fn commit(&self, _: &mut TxCx, _: &PreparedSubscription) -> Result<Committed, ToolError> {
        Err(preview_only())
    }

    async fn verify(
        &self,
        _: &Cx,
        _: &PreparedSubscription,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        Err(preview_only())
    }

    async fn undo(
        &self,
        _: &mut TxCx,
        _: &PreparedSubscription,
        _: &Committed,
    ) -> Result<(), ToolError> {
        Err(preview_only())
    }
}

fn preview_only() -> ToolError {
    ToolError::invalid_input(
        "`prepare_subscription` only prepares; the shopper starts it with POST /v1/sales/subscriptions.",
    )
    .detail(json!({"field": "preview_id", "reason": "preview_only"}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed_with_cycles_as_words() {
        let s = closed_schema_for::<PrepareSubscriptionInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["required"],
            json!([
                "billing_cycle",
                "next_charge_date",
                "delivery_mode",
                "client_total"
            ])
        );
        assert_eq!(
            s["properties"]["billing_cycle"]["enum"],
            json!(["every_week", "every_2_weeks", "every_month"])
        );
    }

    fn input(v: Value) -> PrepareSubscriptionInput {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn the_first_charge_is_after_today() {
        let cx = crate::testdb::offline("cus_jamie");
        let e = PrepareSubscription
            .preview(
                &cx,
                input(
                    json!({"billing_cycle": "every_month", "next_charge_date": "2026-11-07",
                             "delivery_mode": "saved", "delivery_address_id": "adr_home",
                             "client_total": {"value": 58.00, "currency": "USD"}}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.detail.unwrap()["reason"].clone(), e.next_tool.as_deref()),
            (json!("past"), Some("prepare_subscription"))
        );
        let e = PrepareSubscription
            .preview(
                &cx,
                input(
                    json!({"billing_cycle": "every_month", "next_charge_date": "12/01/2026",
                             "delivery_mode": "saved", "delivery_address_id": "adr_home",
                             "client_total": {"value": 58.00, "currency": "USD"}}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["reason"], "format");
    }

    #[test]
    fn a_subscription_needs_a_saved_card() {
        let e = card_check(None, true).unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("not_card_payment", Some("get_payment_methods"))
        );
        assert_eq!(
            card_check(Some("4821"), true).unwrap().detail.as_deref(),
            Some("Card ending 4821")
        );
    }
}
