//! `update_schedule` (`sales.subscription.update_schedule`): how often a standing order comes,
//! its next charge date, how many it orders, or where it goes, from the next order on. A new
//! quantity is priced again at today's prices with the subscription discounts.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::BillingCycle;
use acom_customer::service::for_sales as customer;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::prepare_subscription::{card_check, future_day, offered};
use super::{
    basis, invalid_state, lines_of, not_found, reprice, schedule_renewal, visible_subscription,
};
use crate::delivery;
use crate::pricing;
use crate::repo::{self, Schedule};
use crate::rules;
use crate::{
    ADDRESS_NOT_DELIVERABLE, ADDRESS_NOT_FOUND, NOT_CARD_PAYMENT, NOT_SIGNED_IN, approval, today,
};

const TOOL: &str = "update_schedule";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateScheduleInput {
    /// The subscription (`list_subscriptions`).
    #[schemars(regex(pattern = r"^sub_[a-z0-9_]+$"), example = &"sub_3301")]
    pub subscription_id: String,
    /// How often from now on, as the store offers (`subscription.billing_cycles`).
    #[schemars(example = &"every_2_weeks")]
    pub billing_cycle: Option<BillingCycle>,
    /// The next charge date, a store-time day after today.
    #[schemars(regex(pattern = r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"), example = &"2026-12-15")]
    pub next_charge_date: Option<String>,
    /// Units every order brings, at least 1 (a subscription of one line).
    #[schemars(range(min = 1), example = 2)]
    pub order_quantity: Option<u32>,
    /// One of the shopper's saved addresses (`list_addresses`).
    #[schemars(regex(pattern = r"^adr_[a-z0-9_]+$"), example = &"adr_home")]
    pub delivery_address_id: Option<String>,
}

/// What `update_schedule` changes: the schedule before and after.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SchedulePlan {
    subscription_id: String,
    from: Schedule,
    to: Schedule,
}

pub(crate) struct UpdateSchedule;

impl Operation for UpdateSchedule {
    const META: OpMeta = OpMeta {
        address: "sales.subscription.update_schedule",
        tool: "update_schedule",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/sales/subscriptions/{subscription_id}/schedule",
        summary: "Change a standing order from its next order on: how often, the next charge date, how many, or which saved address. A new quantity is priced again.",
        consumes: &[Slot::one("subscription_id")],
        produces: &[Slot::one("subscription_id")],
        checks: &[
            "subscription.billing_cycles",
            "payment.subscription_card_only",
        ],
        errors: &[
            ErrorCode::NOT_FOUND,
            ErrorCode::INVALID_STATE,
            ADDRESS_NOT_FOUND,
            ADDRESS_NOT_DELIVERABLE,
            NOT_CARD_PAYMENT,
            NOT_SIGNED_IN,
        ],
        audience: Audience::Both,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateScheduleInput;
    type Output = Value;
}

/// The fields that differ, as `{field: value}` before and after.
fn differences(from: &Schedule, to: &Schedule) -> (Value, Value) {
    let (mut a, mut b) = (Map::new(), Map::new());
    let mut put = |k: &str, x: Value, y: Value| {
        if x != y {
            a.insert(k.into(), x);
            b.insert(k.into(), y);
        }
    };
    put(
        "billing_cycle",
        json!(from.billing_cycle),
        json!(to.billing_cycle),
    );
    put(
        "next_charge_date",
        json!(from.next_charge_date.format("%Y-%m-%d").to_string()),
        json!(to.next_charge_date.format("%Y-%m-%d").to_string()),
    );
    let quantities = |s: &Schedule| -> Value {
        json!(
            s.ordered_item
                .as_array()
                .map(|l| l.iter().map(|x| x["quantity"].clone()).collect::<Vec<_>>())
        )
    };
    put("quantity", quantities(from), quantities(to));
    put("amount", json!(from.total), json!(to.total));
    put(
        "delivery_address",
        from.order_delivery["address_label"].clone(),
        to.order_delivery["address_label"].clone(),
    );
    (Value::Object(a), Value::Object(b))
}

impl WriteOp<Cx> for UpdateSchedule {
    type Plan = SchedulePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateScheduleInput,
    ) -> Result<Planned<SchedulePlan>, ToolError> {
        if input.billing_cycle.is_none()
            && input.next_charge_date.is_none()
            && input.order_quantity.is_none()
            && input.delivery_address_id.is_none()
        {
            return Err(ToolError::invalid_input(
                "Give what to change: `billing_cycle`, `next_charge_date`, `order_quantity` or `delivery_address_id`.",
            )
            .detail(json!({"field": "billing_cycle", "reason": "missing"}))
            .next(TOOL));
        }
        let rules = rules::subscription_rules(&cx.policy)?;
        let mut checks = Vec::new();
        if let Some(cycle) = input.billing_cycle {
            offered(&rules, cycle, TOOL)?;
            checks.push(Check {
                rule: "subscription.billing_cycles".into(),
                passed: true,
                detail: Some(format!("{} is offered", cycle.as_str().replace('_', " "))),
                fix_tool: None,
            });
        }
        let today = today(cx.clock.as_ref(), &cx.time);
        let next = input
            .next_charge_date
            .as_deref()
            .map(|d| future_day(d, today, TOOL))
            .transpose()?;

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = visible_subscription(cx, &mut tx, &input.subscription_id).await?;
        if r.subscription_status == "cancelled" {
            return Err(invalid_state(&r, "changed"));
        }
        checks.push(card_check(
            (r.payment_method == "credit_card")
                .then_some(r.card_last4.as_deref())
                .flatten(),
            rules.card_only,
        )?);
        let from = Schedule::of(&r)?;
        let mut to = from.clone();
        if let Some(cycle) = input.billing_cycle {
            to.billing_cycle = cycle.as_str().to_owned();
        }
        if let Some(day) = next {
            to.next_charge_date = day;
        }
        if let Some(quantity) = input.order_quantity {
            let mut lines = lines_of(&r)?;
            let [line] = lines.as_mut_slice() else {
                return Err(ToolError::invalid_input(
                    "This subscription has several lines; a quantity names one of them only.",
                )
                .detail(json!({"field": "order_quantity", "reason": "several_items"}))
                .next(TOOL));
            };
            let max = rules::cart_limits(&cx.policy)?.max_quantity_per_line;
            if quantity == 0 || quantity > max {
                return Err(ToolError::invalid_input(format!(
                    "A line takes 1 to {max} units (`cart.max_quantity_per_line`)."
                ))
                .detail(json!({"field": "order_quantity", "reason": "range",
                               "policy_key": "cart.max_quantity_per_line"}))
                .next(TOOL));
            }
            line.quantity = quantity;
            let (priced, total) = reprice(&mut tx, &r.customer_id, &lines).await?;
            to.ordered_item = serde_json::to_value(&priced)
                .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
            to.total = total;
        }
        if let Some(id) = &input.delivery_address_id {
            let a = customer::address(&mut tx, &r.customer_id, id)
                .await?
                .ok_or_else(|| {
                    ToolError::new(
                        ADDRESS_NOT_FOUND,
                        format!("The subscriber has no saved address {id}."),
                    )
                    .detail(json!({"entity": "postal_address", "id": id}))
                    .next("list_addresses")
                })?;
            let ships_to = rules::ships_to(&cx.policy);
            if !delivery::deliverable(&a.address_country, &ships_to) {
                return Err(ToolError::new(
                    ADDRESS_NOT_DELIVERABLE,
                    format!("The store does not ship to {}.", a.address_country),
                )
                .detail(json!({"country": a.address_country, "ships_to": ships_to,
                               "policy_key": "shipping.ships_to"}))
                .next("list_addresses"));
            }
            let address = pricing::address_of(&a);
            to.order_delivery = json!({
                "address_id": a.id,
                "delivery_address": address,
                "address_label": delivery::full_label(a.label.as_deref(), &address),
            });
        }
        tx.commit().await.map_err(ToolError::from)?;

        let (was, will) = differences(&from, &to);
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "subscription".into(),
                id: r.id.clone(),
                from: was,
                to: will,
                note: (from == to).then(|| "Already so; nothing changes".to_owned()),
            }],
            checks,
            basis: basis(&r),
            approval: approval(cx, Self::META.tool, &json!({"amount": to.total})),
            plan: SchedulePlan {
                subscription_id: r.id,
                from,
                to,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SchedulePlan) -> Result<Committed, ToolError> {
        let r = repo::subscription_for_update(tx, &plan.subscription_id)
            .await?
            .ok_or_else(|| not_found(&plan.subscription_id))?;
        if Schedule::of(&r)? != plan.to {
            repo::set_schedule(tx, &r.id, &plan.to).await?;
            if r.subscription_status == "active" && r.next_charge_date != plan.to.next_charge_date {
                schedule_renewal(tx, &r.id, plan.to.next_charge_date).await?;
            }
        }
        let mut produced = Map::new();
        produced.insert("subscription_id".into(), json!(r.id));
        produced.insert(
            "next_charge_date".into(),
            json!(plan.to.next_charge_date.format("%Y-%m-%d").to_string()),
        );
        produced.insert("amount".into(), json!(plan.to.total));
        Ok(Committed { id: r.id, produced })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SchedulePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::subscription(&mut tx, &plan.subscription_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let now = r.as_ref().map(Schedule::of).transpose()?;
        let matched = now.as_ref() == Some(&plan.to);
        Ok(Verification {
            matched,
            mismatches: if matched {
                vec![]
            } else {
                vec![format!(
                    "subscription {} does not have the planned schedule",
                    plan.subscription_id
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SchedulePlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        let r = repo::subscription_for_update(tx, &plan.subscription_id)
            .await?
            .ok_or_else(|| not_found(&plan.subscription_id))?;
        repo::set_schedule(tx, &r.id, &plan.from).await?;
        if r.subscription_status == "active" && r.next_charge_date != plan.from.next_charge_date {
            schedule_renewal(tx, &r.id, plan.from.next_charge_date).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed_with_cycles_as_words() {
        let s = closed_schema_for::<UpdateScheduleInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["subscription_id"]));
        assert_eq!(
            s["properties"]["billing_cycle"]["enum"],
            json!(["every_week", "every_2_weeks", "every_month"])
        );
    }

    #[tokio::test]
    async fn something_must_change_and_dates_lie_ahead() {
        let cx = crate::testdb::offline("cus_jamie");
        let input = |v: Value| serde_json::from_value::<UpdateScheduleInput>(v).unwrap();
        let e = UpdateSchedule
            .preview(&cx, input(json!({"subscription_id": "sub_3301"})))
            .await
            .unwrap_err();
        assert_eq!(e.detail.unwrap()["reason"], "missing");
        let e = UpdateSchedule
            .preview(
                &cx,
                input(json!({"subscription_id": "sub_3301", "next_charge_date": "2026-11-01"})),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.detail.unwrap()["reason"].clone(), e.next_tool.as_deref()),
            (json!("past"), Some("update_schedule"))
        );
    }

    /// Integration (`DATABASE_URL`, demo seed): Jamie gets two merino tees every two weeks from
    /// Dec 15. Two tees are $116.00, over the $80.00 free-shipping subtotal. Rolled back.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_changes_her_schedule() {
        let jamie = crate::testdb::cx("cus_jamie").await;
        let planned = UpdateSchedule
            .preview(
                &jamie,
                serde_json::from_value(json!({"subscription_id": "sub_3301",
                    "billing_cycle": "every_2_weeks", "next_charge_date": "2026-12-15",
                    "order_quantity": 2}))
                .unwrap(),
            )
            .await
            .unwrap();
        let c = &planned.changes[0];
        assert_eq!(c.from["billing_cycle"], "every_month");
        assert_eq!(c.to["billing_cycle"], "every_2_weeks");
        assert_eq!(c.to["amount"]["value"].to_string(), "116.00");
        let mut tx = jamie.begin().await.unwrap();
        let done = UpdateSchedule.commit(&mut tx, &planned.plan).await.unwrap();
        assert_eq!(done.produced["next_charge_date"], "2026-12-15");
        drop(tx); // rolled back
    }
}
