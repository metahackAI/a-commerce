//! Database tests for the operations built on top of scenes B2 and F5: exchanges and their
//! charges, store cancels, parcels, inspections, conversions, refund retries, evidence and
//! shipping zones (integration, `#[ignore]`).
//!
//! Reads and previews see committed rows only, so the orders, parcels and claims these tests
//! need beyond the demo seed are written as committed fixtures (ids `*_t9…`, order numbers
//! 990…) through the owner role, and removed again at the end (and before, in case an earlier
//! run stopped half-way). Every commit under test runs in a transaction that is rolled back.

use acom_contract::{ApprovalNeed, PersonOp, ReadOp, WriteOp};
use acom_store::{Cx, TxCx};
use serde_json::{Value, json};

use super::{cx_on, db, input};
use crate::claim::{
    CancelOrderLines, ConvertExchangeToReturn, GetExchangeOptions, PayClaimCharge,
    SendExchangeRequest, UploadEvidence, WithdrawClaim,
};
use crate::inspection::AssessReturnCondition;
use crate::parcel_delivery::{
    ConfirmShipment, GetDeliveryDates, RegisterShortage, ScheduleShipment,
};
use crate::refund::RetryRefund;
use crate::shipping_rate::{ListShippingZones, ManageShippingZone, SetShippingRate};

const STORE: &str = "store_northwind";

/// Fixture orders beyond the seed: #99001 (two lines waiting to ship), #99002 (delivered tee
/// in an exchange that came back, and a returned windbreaker whose refund failed), #99004 (a
/// windbreaker in a parcel being picked).
const FIXTURE: &[&str] = &[
    "INSERT INTO sales.sales_order (tenant_id, id, order_number, order_date, order_status, customer_id,
       subtotal_value, subtotal_currency, discount_value, discount_currency, shipping_value,
       shipping_currency, total_payment_due_value, total_payment_due_currency)
     VALUES ('store_northwind', 'ord_t9001', '99001', '2026-11-05T10:00:00-05:00', 'processing', 'cus_dana',
             147.00, 'USD', 0, 'USD', 0, 'USD', 147.00, 'USD'),
            ('store_northwind', 'ord_t9002', '99002', '2026-10-30T10:00:00-04:00', 'delivered', 'cus_dana',
             147.00, 'USD', 0, 'USD', 0, 'USD', 147.00, 'USD'),
            ('store_northwind', 'ord_t9004', '99004', '2026-11-05T11:00:00-05:00', 'processing', 'cus_dana',
             89.00, 'USD', 0, 'USD', 0, 'USD', 89.00, 'USD')",
    "INSERT INTO sales.order_item (tenant_id, id, order_id, position, product_id, name, order_quantity,
       order_item_status, fulfillment_stage, price, price_currency, discount_value, discount_currency)
     VALUES ('store_northwind', 'oln_t9001_1', 'ord_t9001', 1, 'var_windbreaker_m_slate', 'Packable windbreaker, Slate, M', 1, 'processing', NULL, 89.00, 'USD', 0, 'USD'),
            ('store_northwind', 'oln_t9001_2', 'ord_t9001', 2, 'var_merino_tee_m_charcoal', 'Merino trail tee, Charcoal, M', 1, 'processing', NULL, 58.00, 'USD', 0, 'USD'),
            ('store_northwind', 'oln_t9002_1', 'ord_t9002', 1, 'var_merino_tee_m_charcoal', 'Merino trail tee, Charcoal, M', 1, 'delivered', 'shipped', 58.00, 'USD', 0, 'USD'),
            ('store_northwind', 'oln_t9002_2', 'ord_t9002', 2, 'var_windbreaker_l_slate', 'Packable windbreaker, Slate, L', 1, 'delivered', 'shipped', 89.00, 'USD', 0, 'USD'),
            ('store_northwind', 'oln_t9004_1', 'ord_t9004', 1, 'var_windbreaker_m_slate', 'Packable windbreaker, Slate, M', 1, 'processing', 'ship_instructed', 89.00, 'USD', 0, 'USD')",
    "INSERT INTO sales.payment (tenant_id, id, order_id, payment_method, payment_status, amount_value,
       amount_currency, card_last4, paid_at)
     VALUES ('store_northwind', 'pay_t9001', 'ord_t9001', 'credit_card', 'payment_complete', 147.00, 'USD', '4242', '2026-11-05T10:01:00-05:00'),
            ('store_northwind', 'pay_t9002', 'ord_t9002', 'credit_card', 'payment_complete', 147.00, 'USD', '4242', '2026-10-30T10:01:00-04:00'),
            ('store_northwind', 'pay_t9004', 'ord_t9004', 'credit_card', 'payment_complete', 89.00, 'USD', '4242', '2026-11-05T11:01:00-05:00')",
    "INSERT INTO fulfillment.parcel_delivery (tenant_id, id, order_id, item_shipped, delivery_state,
       delivery_address, carrier, delivered_at)
     VALUES ('store_northwind', 'dlv_t9002', 'ord_t9002', NULL, 'delivered',
             '{\"name\": \"Dana Whitfield\", \"street_address\": \"7 Bay St\", \"address_locality\": \"Hoboken\", \"address_region\": \"NJ\", \"postal_code\": \"07030\", \"address_country\": \"US\"}',
             'UPS', '2026-11-02T14:00:00-05:00'),
            ('store_northwind', 'dlv_t9004', 'ord_t9004', '[{\"order_item_id\": \"oln_t9004_1\", \"order_quantity\": 1}]', 'ship_instructed',
             '{\"name\": \"Dana Whitfield\", \"street_address\": \"7 Bay St\", \"address_locality\": \"Hoboken\", \"address_region\": \"NJ\", \"postal_code\": \"07030\", \"address_country\": \"US\"}',
             NULL, NULL)",
    "INSERT INTO fulfillment.claim (tenant_id, id, order_id, customer_id, claim_type, claim_status,
       claim_path, claim_reason, liability, return_fees, send_back_method, amount_value, amount_currency,
       refund_estimate_value, refund_estimate_currency, start_time)
     VALUES ('store_northwind', 'clm_t9002', 'ord_t9002', 'cus_dana', 'exchange', 'picked_up', 'self_service',
             'wrong_size', 'customer_fault', 'return_fees_customer_responsibility', 'pickup', 58.00, 'USD',
             NULL, NULL, '2026-11-03T09:00:00-05:00'),
            ('store_northwind', 'clm_t9003', 'ord_t9002', 'cus_dana', 'return', 'approved', 'store_review',
             'defect', 'store_fault', 'free_return', 'pickup', 89.00, 'USD', 89.00, 'USD',
             '2026-11-03T09:30:00-05:00')",
    "INSERT INTO fulfillment.claim_item (tenant_id, claim_id, order_item_id, order_quantity, replacement_product_id)
     VALUES ('store_northwind', 'clm_t9002', 'oln_t9002_1', 1, 'var_merino_tee_l_charcoal'),
            ('store_northwind', 'clm_t9003', 'oln_t9002_2', 1, NULL)",
    "INSERT INTO fulfillment.refund (tenant_id, id, claim_id, order_id, payment_id, amount_value,
       amount_currency, refund_status, start_time, end_time)
     VALUES ('store_northwind', 'rfd_t9003', 'clm_t9003', 'ord_t9002', 'pay_t9002', 89.00, 'USD', 'failed',
             '2026-11-04T10:00:00-05:00', '2026-11-05T08:00:00-05:00')",
];

/// Removes the fixture and anything a committed step could have added to it.
const CLEANUP: &[&str] = &[
    "DELETE FROM fulfillment.claim_event WHERE tenant_id = $1 AND claim_id IN
       (SELECT id FROM fulfillment.claim WHERE tenant_id = $1 AND order_id IN ('ord_t9001', 'ord_t9002', 'ord_t9004'))",
    "DELETE FROM fulfillment.claim_item WHERE tenant_id = $1 AND claim_id IN
       (SELECT id FROM fulfillment.claim WHERE tenant_id = $1 AND order_id IN ('ord_t9001', 'ord_t9002', 'ord_t9004'))",
    "DELETE FROM fulfillment.refund_event WHERE tenant_id = $1 AND refund_id IN
       (SELECT id FROM fulfillment.refund WHERE tenant_id = $1 AND order_id IN ('ord_t9001', 'ord_t9002', 'ord_t9004'))",
    "DELETE FROM fulfillment.refund WHERE tenant_id = $1 AND order_id IN ('ord_t9001', 'ord_t9002', 'ord_t9004')",
    "DELETE FROM fulfillment.inspection WHERE tenant_id = $1 AND claim_id IN
       (SELECT id FROM fulfillment.claim WHERE tenant_id = $1 AND order_id IN ('ord_t9001', 'ord_t9002', 'ord_t9004'))",
    "DELETE FROM fulfillment.parcel_delivery_event WHERE tenant_id = $1 AND parcel_delivery_id IN
       (SELECT id FROM fulfillment.parcel_delivery WHERE tenant_id = $1 AND order_id IN ('ord_t9001', 'ord_t9002', 'ord_t9004'))",
    "DELETE FROM fulfillment.parcel_delivery WHERE tenant_id = $1 AND order_id IN ('ord_t9001', 'ord_t9002', 'ord_t9004')",
    "DELETE FROM fulfillment.claim WHERE tenant_id = $1 AND order_id IN ('ord_t9001', 'ord_t9002', 'ord_t9004')",
    "DELETE FROM sales.order_event WHERE tenant_id = $1 AND order_id IN ('ord_t9001', 'ord_t9002', 'ord_t9004')",
    "DELETE FROM sales.payment WHERE tenant_id = $1 AND order_id IN ('ord_t9001', 'ord_t9002', 'ord_t9004')",
    "DELETE FROM sales.order_item WHERE tenant_id = $1 AND order_id IN ('ord_t9001', 'ord_t9002', 'ord_t9004')",
    "DELETE FROM sales.sales_order WHERE tenant_id = $1 AND id IN ('ord_t9001', 'ord_t9002', 'ord_t9004')",
];

async fn run_all(cx: &Cx, statements: &[&str], bind_store: bool) {
    let mut tx = cx.begin().await.unwrap();
    for s in statements {
        let q = sqlx::query(sqlx::AssertSqlSafe(*s));
        let q = if bind_store { q.bind(STORE) } else { q };
        q.execute(&mut *tx.tx)
            .await
            .unwrap_or_else(|e| panic!("{s}: {e}"));
    }
    tx.commit().await.unwrap();
}

/// The owner role (it may delete) at the scene time, with the fixture loaded.
async fn with_fixture() -> Cx {
    let owner = cx_on(
        db("MIGRATION_DATABASE_URL").await,
        "adm_sj",
        "2026-11-06T10:40",
    );
    run_all(&owner, CLEANUP, true).await;
    run_all(&owner, FIXTURE, false).await;
    owner
}

async fn scalar(tx: &mut TxCx, sql: &str) -> Value {
    let v: Option<Value> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT to_jsonb(x) FROM ({sql}) AS t(x)"
    )))
    .fetch_optional(&mut *tx.tx)
    .await
    .unwrap();
    v.unwrap_or(Value::Null)
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn zones_list_and_set_transit_days_and_surcharges() {
    let cx = cx_on(db("DATABASE_URL").await, "adm_sj", "2026-11-06T10:40");
    let zones = ListShippingZones.read(&cx, input(json!({}))).await.unwrap();
    assert_eq!(zones["total"], 3);
    assert_eq!(zones["items"][0]["id"], "zone_us_contiguous");
    assert_eq!(zones["items"][0]["free_over"]["value"].to_string(), "80.00");

    let jamie = cx_on(db("DATABASE_URL").await, "cus_jamie", "2026-11-07T10:00");
    let to_alaska = GetDeliveryDates
        .read(
            &jamie,
            input(json!({"product_id": "prd_windbreaker", "address_mode": "one_time",
                         "delivery_address": {"street_address": "1 Main St", "address_locality": "Anchorage",
                                              "address_region": "AK", "postal_code": "99501", "address_country": "US"}})),
        )
        .await
        .unwrap();
    assert_eq!(to_alaska["items"][0]["earliest_arrival"], "2026-11-13");
    assert_eq!(
        to_alaska["items"][0]["remote_area_surcharge"]["value"].to_string(),
        "12.00"
    );
    let abroad = GetDeliveryDates
        .read(
            &jamie,
            input(
                json!({"product_id": "prd_windbreaker", "address_mode": "one_time",
                         "delivery_address": {"street_address": "1 Rue", "address_locality": "Lyon",
                                              "postal_code": "69001", "address_country": "FR"}}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(abroad.code, "address_not_deliverable");

    let canada = ManageShippingZone
        .preview(
            &cx,
            input(json!({"name": "Canada", "shipping_destination": [{"address_country": "CA"}],
                         "surcharge": {"value": 9.00, "currency": "USD"}, "transit_business_days": 4,
                         "zone_type": "remote"})),
        )
        .await
        .unwrap();
    assert_eq!(canada.plan.zone_id, "zone_canada");
    assert_eq!(canada.approval, ApprovalNeed::NotNeeded);
    let mut tx = cx.begin().await.unwrap();
    ManageShippingZone
        .commit(&mut tx, &canada.plan)
        .await
        .unwrap();
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT surcharge_value FROM fulfillment.shipping_zone WHERE id = 'zone_canada'"
        )
        .await,
        json!(9.00)
    );
    drop(tx);
    let clash = ManageShippingZone
        .preview(
            &cx,
            input(json!({"name": "Alaska", "shipping_destination": [{"address_country": "US", "address_region": "AK"}]})),
        )
        .await
        .unwrap_err();
    assert_eq!(clash.code, "overlapping_zone");
    assert_eq!(clash.next_tool.as_deref(), Some("list_shipping_zones"));

    let rate = SetShippingRate
        .preview(
            &cx,
            input(
                json!({"zone_id": "zone_us_contiguous", "fee_type": "free_over_min_subtotal",
                         "shipping_rate": {"value": 7.00, "currency": "USD"},
                         "free_over": {"value": 80.00, "currency": "USD"}}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        rate.checks[1].detail.as_deref(),
        Some("Free over $80.00 kept")
    );
    let mut tx = cx.begin().await.unwrap();
    SetShippingRate.commit(&mut tx, &rate.plan).await.unwrap();
    SetShippingRate
        .undo(
            &mut tx,
            &rate.plan,
            &acom_contract::Committed {
                id: "zone_us_contiguous".into(),
                produced: Default::default(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        scalar(&mut tx, "SELECT shipping_rate_value FROM fulfillment.shipping_zone WHERE id = 'zone_us_contiguous'").await,
        json!(6.00)
    );
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn jamie_exchanges_her_tee_and_pays_for_a_color_swap() {
    let cx = cx_on(db("DATABASE_URL").await, "cus_jamie", "2026-11-06T10:40");
    let options = GetExchangeOptions
        .read(
            &cx,
            input(json!({"order_id": "ord_20381", "order_item_id": "oln_20381_1", "reason": "wrong_size"})),
        )
        .await
        .unwrap();
    let large = options["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["variant_id"] == "var_merino_tee_l_charcoal")
        .unwrap();
    assert_eq!(large["selectable"], true);
    assert!(
        !options["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["variant_id"] == "var_merino_tee_m_charcoal"),
        "a size swap does not offer the same variant"
    );
    assert_eq!(options["you_pay"]["value"].to_string(), "0.00");
    assert_eq!(options["confirmed_by"], "store_automatic");
    assert_eq!(
        options["pickup"]["window"]["start"],
        "2026-11-09T09:00:00-05:00"
    );
    assert_eq!(options["reship"]["ships_on"], "2026-11-12");
    assert_eq!(options["order_item"]["days_since_delivery"], 9);

    let mut tx = cx.begin().await.unwrap();
    let before = scalar(
        &mut tx,
        "SELECT inventory_level FROM catalog.offer WHERE product_id = 'var_merino_tee_l_charcoal'",
    )
    .await;
    let sent = SendExchangeRequest
        .run(
            &mut tx,
            input(json!({"order_id": "ord_20381", "reason": "wrong_size",
                         "items": [{"order_item_id": "oln_20381_1", "quantity": 1,
                                    "replacement_variant_id": "var_merino_tee_l_charcoal"}],
                         "pickup_window": {"start": "2026-11-09T09:00:00-05:00", "end": "2026-11-09T12:00:00-05:00"},
                         "expected_charge": {"value": 0.00, "currency": "USD"}})),
        )
        .await
        .unwrap();
    assert_eq!(sent["status"], "pickup_instructed");
    let held = scalar(
        &mut tx,
        "SELECT inventory_level FROM catalog.offer WHERE product_id = 'var_merino_tee_l_charcoal'",
    )
    .await;
    assert_eq!(held.as_i64().unwrap(), before.as_i64().unwrap() - 1);
    let w = WithdrawClaim
        .run(&mut tx, input(json!({"claim_id": sent["id"]})))
        .await
        .unwrap();
    assert_eq!(w["status"], "withdrawn");
    let back = scalar(
        &mut tx,
        "SELECT inventory_level FROM catalog.offer WHERE product_id = 'var_merino_tee_l_charcoal'",
    )
    .await;
    assert_eq!(back, before);
    drop(tx);

    // A color swap is the shopper's choice: a round trip at the zone's rate, paid first.
    let colour = GetExchangeOptions
        .read(
            &cx,
            input(
                json!({"order_id": "ord_20381", "order_item_id": "oln_20381_1",
                         "reason": "different_option_wanted"}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(colour["you_pay"]["value"].to_string(), "12.00");
    assert_eq!(colour["pay_when"], "before_accept");
    let mut tx = cx.begin().await.unwrap();
    let changed = SendExchangeRequest
        .run(
            &mut tx,
            input(
                json!({"order_id": "ord_20381", "reason": "different_option_wanted",
                         "items": [{"order_item_id": "oln_20381_1", "quantity": 1,
                                    "replacement_variant_id": "var_merino_tee_l_charcoal"}],
                         "expected_charge": {"value": 0.00, "currency": "USD"}}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(changed.code, "estimate_changed");
    let sent = SendExchangeRequest
        .run(
            &mut tx,
            input(
                json!({"order_id": "ord_20381", "reason": "different_option_wanted",
                         "items": [{"order_item_id": "oln_20381_1", "quantity": 1,
                                    "replacement_variant_id": "var_merino_tee_l_charcoal"}],
                         "expected_charge": {"value": 12.00, "currency": "USD"}}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(sent["status"], "charge_due");
    let declined = PayClaimCharge
        .run(
            &mut tx,
            input(
                json!({"claim_id": sent["id"], "amount": {"value": 12.00, "currency": "USD"},
                         "payment_method_ref": "declined_card"}),
            ),
        )
        .await
        .unwrap_err();
    assert_eq!(declined.code, "payment_declined");
    let paid = PayClaimCharge
        .run(
            &mut tx,
            input(
                json!({"claim_id": sent["id"], "amount": {"value": 12.00, "currency": "USD"},
                         "payment_method_ref": "pm_card_4821"}),
            ),
        )
        .await
        .unwrap();
    assert_eq!(paid["status"], "pickup_instructed");
    let w = WithdrawClaim
        .run(&mut tx, input(json!({"claim_id": sent["id"]})))
        .await
        .unwrap();
    assert_eq!(w["charge_refunded"]["value"].to_string(), "12.00");
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn the_store_ships_labels_runs_short_and_cancels_lines() {
    let owner = with_fixture().await;
    let cx = cx_on(db("DATABASE_URL").await, "adm_sj", "2026-11-06T10:40");

    let ship = ScheduleShipment
        .preview(&cx, input(json!({"order_id": "ord_t9001"})))
        .await
        .unwrap();
    assert_eq!(ship.plan.lines.len(), 2);
    assert_eq!(ship.plan.delivery_state, "ship_instructed");
    assert_eq!(ship.plan.address["street_address"], "7 Bay St");
    assert_eq!(
        ship.checks[0].detail.as_deref(),
        Some("All 2 items in stock")
    );
    let mut tx = cx.begin().await.unwrap();
    let done = ScheduleShipment.commit(&mut tx, &ship.plan).await.unwrap();
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT fulfillment_stage FROM sales.order_item WHERE id = 'oln_t9001_1'"
        )
        .await,
        "ship_instructed"
    );
    ScheduleShipment
        .undo(&mut tx, &ship.plan, &done)
        .await
        .unwrap();
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT fulfillment_stage FROM sales.order_item WHERE id = 'oln_t9001_1'"
        )
        .await,
        Value::Null
    );
    drop(tx);

    let label = ConfirmShipment
        .preview(
            &cx,
            input(json!({"delivery_id": "dlv_t9004", "tracking_number": "1ZNWT90040001", "carrier": "UPS"})),
        )
        .await
        .unwrap();
    assert_eq!(
        label.plan.order_status_before.as_deref(),
        Some("processing")
    );
    assert_eq!(
        label.checks[0].detail.as_deref(),
        Some("Tracking number matches the UPS format")
    );
    let mut tx = cx.begin().await.unwrap();
    let done = ConfirmShipment.commit(&mut tx, &label.plan).await.unwrap();
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT order_status FROM sales.sales_order WHERE id = 'ord_t9004'"
        )
        .await,
        "in_transit"
    );
    ConfirmShipment
        .undo(&mut tx, &label.plan, &done)
        .await
        .unwrap();
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT delivery_state FROM fulfillment.parcel_delivery WHERE id = 'dlv_t9004'"
        )
        .await,
        "ship_instructed"
    );
    drop(tx);

    let short = RegisterShortage
        .preview(
            &cx,
            input(json!({"delivery_id": "dlv_t9004", "order_item_ids": ["oln_t9004_1"]})),
        )
        .await
        .unwrap();
    assert_eq!(
        short.checks[1].detail.as_deref(),
        Some("The customer can cancel it themselves while waiting")
    );
    let mut tx = cx.begin().await.unwrap();
    RegisterShortage.commit(&mut tx, &short.plan).await.unwrap();
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT delivery_state FROM fulfillment.parcel_delivery WHERE id = 'dlv_t9004'"
        )
        .await,
        "awaiting_stock"
    );
    drop(tx);

    let wrong = CancelOrderLines
        .preview(
            &cx,
            input(json!({"order_id": "ord_t9001", "reason": "changed_mind",
                         "items": [{"order_item_id": "oln_t9001_2", "quantity": 1}]})),
        )
        .await
        .unwrap_err();
    assert_eq!(wrong.code, "reason_not_store_fault");
    let cancel = CancelOrderLines
        .preview(
            &cx,
            input(json!({"order_id": "ord_t9001", "reason": "out_of_stock",
                         "items": [{"order_item_id": "oln_t9001_2", "quantity": 1}]})),
        )
        .await
        .unwrap();
    assert_eq!(
        cancel.approval,
        ApprovalNeed::Needed {
            reason: "Store-initiated cancel refunds $58.00".into()
        }
    );
    assert_eq!(
        cancel.plan.price_after.total_payment_due.value.to_string(),
        "89.00"
    );
    let mut tx = cx.begin().await.unwrap();
    CancelOrderLines
        .commit(&mut tx, &cancel.plan)
        .await
        .unwrap();
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT total_payment_due_value FROM sales.sales_order WHERE id = 'ord_t9001'"
        )
        .await,
        json!(89.00)
    );
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT order_item_status FROM sales.order_item WHERE id = 'oln_t9001_2'"
        )
        .await,
        "cancelled"
    );
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT amount_value FROM fulfillment.refund WHERE order_id = 'ord_t9001'"
        )
        .await,
        json!(58.00)
    );
    drop(tx);
    let shipped = CancelOrderLines
        .preview(
            &cx,
            input(json!({"order_id": "ord_20402", "reason": "store_cancelled",
                         "items": [{"order_item_id": "oln_20402_1", "quantity": 1}]})),
        )
        .await
        .unwrap_err();
    assert_eq!(shipped.code, "already_shipped");
    assert_eq!(shipped.next_tool.as_deref(), Some("get_delivery"));
    run_all(&owner, CLEANUP, true).await;
}

#[tokio::test]
#[ignore = "integration: needs a PostgreSQL database"]
async fn returns_come_back_exchanges_convert_and_refunds_retry() {
    let owner = with_fixture().await;
    let cx = cx_on(db("DATABASE_URL").await, "adm_sj", "2026-11-06T10:40");

    let check = AssessReturnCondition
        .preview(
            &cx,
            input(json!({"claim_id": "clm_t9002", "condition": "as_new"})),
        )
        .await
        .unwrap();
    assert_eq!(check.plan.to_status, "reship_requested");
    assert_eq!(
        check.checks[0].detail.as_deref(),
        Some("Condition as new: 1 unit back to stock")
    );
    let mut tx = cx.begin().await.unwrap();
    let done = AssessReturnCondition
        .commit(&mut tx, &check.plan)
        .await
        .unwrap();
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT claim_status FROM fulfillment.claim WHERE id = 'clm_t9002'"
        )
        .await,
        "reship_requested"
    );
    AssessReturnCondition
        .undo(&mut tx, &check.plan, &done)
        .await
        .unwrap();
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT count(*) FROM fulfillment.inspection WHERE claim_id = 'clm_t9002'"
        )
        .await,
        json!(0)
    );
    drop(tx);
    let early = AssessReturnCondition
        .preview(
            &cx,
            input(json!({"claim_id": "clm_20402", "condition": "damaged"})),
        )
        .await
        .unwrap_err();
    assert_eq!(early.code, "not_picked_up");

    let convert = ConvertExchangeToReturn
        .preview(
            &cx,
            input(json!({"claim_id": "clm_t9002", "reason": "customer_declines"})),
        )
        .await
        .unwrap();
    assert_eq!(convert.plan.liability, "customer_fault");
    assert_eq!(
        convert.plan.release,
        vec![("var_merino_tee_l_charcoal".to_owned(), 1)]
    );
    let mut tx = cx.begin().await.unwrap();
    ConvertExchangeToReturn
        .commit(&mut tx, &convert.plan)
        .await
        .unwrap();
    let sql = format!(
        "SELECT claim_status FROM fulfillment.claim WHERE id = '{}'",
        convert.plan.return_id
    );
    assert_eq!(scalar(&mut tx, &sql).await, "requested");
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT claim_status FROM fulfillment.claim WHERE id = 'clm_t9002'"
        )
        .await,
        "converted"
    );
    drop(tx);
    let not_exchange = ConvertExchangeToReturn
        .preview(
            &cx,
            input(json!({"claim_id": "clm_20402", "reason": "customer_declines"})),
        )
        .await
        .unwrap_err();
    assert_eq!(not_exchange.code, "not_an_exchange");

    let retry = RetryRefund
        .preview(&cx, input(json!({"refund_id": "rfd_t9003"})))
        .await
        .unwrap();
    assert_eq!(retry.checks[1].detail.as_deref(), Some("Attempt 2 of 3"));
    assert_eq!(
        retry.approval,
        ApprovalNeed::Needed {
            reason: "Retries a $89.00 refund".into()
        }
    );
    let mut tx = cx.begin().await.unwrap();
    let done = RetryRefund.commit(&mut tx, &retry.plan).await.unwrap();
    assert_eq!(
        RetryRefund.commit(&mut tx, &retry.plan).await.unwrap(),
        done
    );
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT refund_status FROM fulfillment.refund WHERE id = 'rfd_t9003'"
        )
        .await,
        "requested"
    );
    drop(tx);
    let done_already = RetryRefund
        .preview(&cx, input(json!({"refund_id": "rfd_20298"})))
        .await
        .unwrap_err();
    assert_eq!(done_already.code, "not_failed");

    let photos = UploadEvidence
        .preview(
            &cx,
            input(json!({"claim_id": "clm_20366", "evidence_type": "damage_photo",
                         "files": [{"upload_id": "upl_parka_1", "name": "parka-1.jpg",
                                    "encoding_format": "image/jpeg", "content_size_bytes": 412000},
                                   {"upload_id": "upl_parka_2", "name": "parka-2.jpg",
                                    "encoding_format": "image/jpeg", "content_size_bytes": 398000}]})),
        )
        .await
        .unwrap();
    assert_eq!(photos.checks[0].detail.as_deref(), Some("2 JPEG photos"));
    let mut tx = cx.begin().await.unwrap();
    UploadEvidence.commit(&mut tx, &photos.plan).await.unwrap();
    assert_eq!(
        scalar(
            &mut tx,
            "SELECT jsonb_array_length(evidence) FROM fulfillment.claim WHERE id = 'clm_20366'"
        )
        .await,
        json!(2)
    );
    drop(tx);
    run_all(&owner, CLEANUP, true).await;
}
