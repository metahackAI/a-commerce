//! Job `subscription.renew` (`platform.job`): on a subscription's charge date, place its order
//! and charge the saved card, then move the charge date one cycle on
//! (`order.subscription.cycle_length`) and schedule the next one. Scheduled by
//! `start_subscription`, `resume_subscription` and `update_schedule` for the date they set; a
//! job whose subscription is no longer active, or whose date moved, places nothing.
//!
//! The order is what the shopper agreed to when subscribing (its lines, discounts and amount),
//! takes stock like any order (`inventory.reserve_on`), and is paid by the card on file. The card
//! provider is outside this system: the charge is recorded as approved, and the order says so.
//! A line that is out of stock fails the job, which the runner retries later with nothing placed.

use acom_catalog::service::for_sales as catalog;
use acom_contract::{DomainEvent, ErrorCode, Job, JobMeta, JobOutcome, ToolError};
use acom_core::enums::{OrderStatus, PaymentStatus};
use acom_core::ids::{OrderId, OrderItemId, PaymentId};
use acom_promotion::service::for_sales as promotion;
use acom_store::{Cx, TxCx};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::{RENEW_JOB, lines_of, schedule_renewal};
use crate::delivery;
use crate::repo::{self, SubscriptionState};
use crate::rules;
use crate::{OUT_OF_STOCK, outside_system};

#[derive(Debug)]
pub struct RenewSubscription;

fn skipped(id: &str, why: &str) -> JobOutcome {
    JobOutcome::done(json!({"subscription_id": id, "order_id": null, "skipped": why}))
}

impl Job<Cx> for RenewSubscription {
    const META: JobMeta = JobMeta {
        kind: RENEW_JOB,
        summary: "Places a subscription's order on its charge date and schedules the next one.",
        watcher: None,
    };

    async fn run(&self, tx: &mut TxCx, payload: &Value) -> Result<JobOutcome, ToolError> {
        let id = payload
            .get("subscription_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::invalid_input("The job names no subscription_id."))?
            .to_owned();
        let date = payload
            .get("charge_date")
            .and_then(Value::as_str)
            .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .ok_or_else(|| ToolError::invalid_input("The job names no charge_date."))?;
        let Some(r) = repo::subscription_for_update(tx, &id).await? else {
            return Ok(skipped(&id, "no such subscription"));
        };
        if r.subscription_status != "active" {
            return Ok(skipped(&id, &format!("it is {}", r.subscription_status)));
        }
        if r.next_charge_date != date {
            return Ok(skipped(&id, "its charge date moved"));
        }
        let length = rules::subscription_rules(&tx.policy)?
            .cycle_length
            .get(&r.billing_cycle)
            .copied()
            .ok_or_else(|| {
                ToolError::new(
                    ErrorCode::INTERNAL,
                    format!(
                        "policy order.subscription.cycle_length has no {}",
                        r.billing_cycle
                    ),
                )
            })?;

        let lines = lines_of(&r)?;
        let total = repo::money(r.total_payment_due_value, &r.total_payment_due_currency)?;
        let currency = total.currency;
        let subtotal: Decimal = lines
            .iter()
            .map(|l| l.price.value * Decimal::from(l.quantity))
            .sum();
        let discount: Decimal = lines.iter().map(|l| l.discount.value).sum();
        let shipping = (total.value - subtotal + discount).max(Decimal::ZERO);
        let now = tx.clock.now();
        let order_id = OrderId::generate().to_string();
        let order_number = repo::next_order_number(tx).await?;
        let processing = OrderStatus::Processing.as_str();
        repo::insert_order(
            tx,
            &repo::NewOrder {
                id: &order_id,
                order_number: &order_number,
                order_date: now,
                order_status: processing,
                customer_id: &r.customer_id,
                sales_channel: "web",
                discount_code: None,
                coupon_id: None,
                subtotal: promotion::money(subtotal, currency),
                discount: promotion::money(discount, currency),
                shipping: promotion::money(shipping, currency),
                total,
                preview_id: r.preview_id.as_deref(),
                is_gift: false,
                gift_options: None,
                gift_wrap: None,
            },
        )
        .await?;
        for (n, l) in lines.iter().enumerate() {
            let item_id = OrderItemId::generate().to_string();
            repo::insert_item(
                tx,
                &repo::NewItem {
                    id: &item_id,
                    order_id: &order_id,
                    position: i32::try_from(n + 1).unwrap_or(i32::MAX),
                    variant_id: &l.variant_id,
                    name: &l.name,
                    quantity: i32::try_from(l.quantity).unwrap_or(i32::MAX),
                    status: processing,
                    price: l.price,
                    discount: l.discount,
                    promotion_id: None,
                },
            )
            .await?;
            // `inventory.reserve_on: place_order`: the order takes its stock now.
            if !catalog::take_stock(tx, &l.variant_id, i64::from(l.quantity)).await? {
                return Err(ToolError::new(
                    OUT_OF_STOCK,
                    format!("{} is out of stock for subscription {id}.", l.name),
                )
                .detail(json!({"variant_id": l.variant_id, "subscription_id": id})));
            }
        }
        let card = outside_system(
            "card_provider",
            "The card provider is not connected: the charge to the saved card is recorded as approved without calling it.",
        );
        let payment_id = PaymentId::generate().to_string();
        repo::insert_payment(
            tx,
            &repo::NewPayment {
                id: &payment_id,
                order_id: &order_id,
                method: &r.payment_method,
                status: PaymentStatus::PaymentComplete.as_str(),
                amount: total,
                card_last4: r.card_last4.as_deref(),
                paid_at: Some(now),
            },
        )
        .await?;
        let calendar = rules::ship_calendar(&tx.policy)?;
        let arrives = delivery::arrival(now.with_timezone(&tx.time.zone).naive_local(), &calendar);
        let mut order_delivery = r.order_delivery.clone();
        order_delivery["arrives"] = json!(arrives.format("%Y-%m-%d").to_string());
        repo::append_event(
            tx,
            &order_id,
            "OrderAction",
            RENEW_JOB,
            None,
            Some(processing),
            &json!({"subscription_id": id, "charge_date": date.to_string(),
                    "delivery": order_delivery, "payment_id": payment_id,
                    "outside_system": card}),
        )
        .await?;

        let next = length.after(date);
        repo::set_subscription_state(
            tx,
            &id,
            &SubscriptionState {
                next_charge_date: next,
                ..SubscriptionState::from(&r)
            },
        )
        .await?;
        schedule_renewal(tx, &id, next).await?;
        let result = json!({"subscription_id": id, "order_id": order_id,
                            "order_number": order_number, "total": total,
                            "next_charge_date": next.to_string()});
        Ok(JobOutcome::Done {
            events: vec![DomainEvent {
                topic: "subscription.renewed".into(),
                object_id: id.clone(),
                payload: result.clone(),
            }],
            result,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_job_is_named_for_its_resource() {
        assert_eq!(RenewSubscription::META.kind, "subscription.renew");
        assert!(RenewSubscription::META.watcher.is_none());
    }

    /// Integration (`DATABASE_URL`, demo seed): Jamie's monthly merino tee comes due on Dec 1;
    /// the job places a $64.00 order paid by card 4821 and moves the date to Jan 1. A job for
    /// a date that moved places nothing. Rolled back.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamies_tee_is_ordered_on_its_charge_date() {
        let cx = crate::testdb::cx_at("sys_scheduler", "2026-12-01T00:00").await;
        let mut tx = cx.begin().await.unwrap();
        let stale = RenewSubscription
            .run(
                &mut tx,
                &json!({"subscription_id": "sub_3301", "charge_date": "2026-11-01"}),
            )
            .await
            .unwrap();
        let JobOutcome::Done { result, .. } = stale else {
            panic!("a renewal is a plain job");
        };
        assert_eq!(result["skipped"], "its charge date moved");
        let done = RenewSubscription
            .run(
                &mut tx,
                &json!({"subscription_id": "sub_3301", "charge_date": "2026-12-01"}),
            )
            .await
            .unwrap();
        let JobOutcome::Done { result, events } = done else {
            panic!("a renewal is a plain job");
        };
        let order_id = result["order_id"].as_str().unwrap().to_owned();
        let row: (String, rust_decimal::Decimal, Option<String>) = sqlx::query_as(
            "SELECT o.order_status, o.total_payment_due_value, p.card_last4
             FROM sales.sales_order o JOIN sales.payment p
               ON p.tenant_id = o.tenant_id AND p.order_id = o.id
             WHERE o.tenant_id = $1 AND o.id = $2",
        )
        .bind("store_northwind")
        .bind(&order_id)
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        let next: NaiveDate = sqlx::query_scalar(
            "SELECT next_charge_date FROM sales.subscription WHERE tenant_id = $1 AND id = $2",
        )
        .bind("store_northwind")
        .bind("sub_3301")
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        drop(tx); // rolled back
        assert_eq!(row.0, "processing");
        assert_eq!(row.1.to_string(), "64.00");
        assert_eq!(row.2.as_deref(), Some("4821"));
        assert_eq!(next.to_string(), "2027-01-01");
        assert_eq!(events[0].topic, "subscription.renewed");
    }
}
