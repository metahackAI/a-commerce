//! The order policy values sales uses (`policies/order.yml`, and the delivery estimate in
//! `policies/fulfillment.yml`), read once per request into plain values.

use std::collections::BTreeMap;

use acom_contract::{ErrorCode, ToolError};
use acom_core::Money;
use acom_core::enums::FulfillmentStage;
use acom_policy::PolicySet;
use chrono::{Days, Months, NaiveDate, NaiveTime, Weekday};
use serde_json::Value;

fn missing(path: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INTERNAL,
        format!("policy {path} is missing or malformed"),
    )
}

fn words(policy: &PolicySet, path: &str) -> Vec<String> {
    policy
        .lookup(path)
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn count(policy: &PolicySet, path: &str) -> Result<u32, ToolError> {
    policy
        .lookup(path)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| missing(path))
}

/// `cart.*`: how big a cart may get.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CartLimits {
    pub max_lines: u32,
    pub max_quantity_per_line: u32,
}

pub(crate) fn cart_limits(policy: &PolicySet) -> Result<CartLimits, ToolError> {
    Ok(CartLimits {
        max_lines: count(policy, "order.cart.max_lines")?,
        max_quantity_per_line: count(policy, "order.cart.max_quantity_per_line")?,
    })
}

/// When parcels leave and how long they travel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShipCalendar {
    /// Orders before this local time ship the same day.
    pub cutoff: NaiveTime,
    /// The days parcels leave; transit is counted on these days too.
    pub ship_days: Vec<Weekday>,
    pub transit_days: u32,
}

fn weekday(word: &str) -> Option<Weekday> {
    Some(match word {
        "mon" => Weekday::Mon,
        "tue" => Weekday::Tue,
        "wed" => Weekday::Wed,
        "thu" => Weekday::Thu,
        "fri" => Weekday::Fri,
        "sat" => Weekday::Sat,
        "sun" => Weekday::Sun,
        _ => return None,
    })
}

pub(crate) fn ship_calendar(policy: &PolicySet) -> Result<ShipCalendar, ToolError> {
    let cutoff_path = "fulfillment.delivery.estimate.ship_cutoff_local_time";
    let cutoff = policy
        .lookup(cutoff_path)
        .and_then(Value::as_str)
        .and_then(|s| NaiveTime::parse_from_str(s, "%H:%M").ok())
        .ok_or_else(|| missing(cutoff_path))?;
    let ship_days: Vec<Weekday> = words(policy, "fulfillment.delivery.estimate.ship_days")
        .iter()
        .filter_map(|w| weekday(w))
        .collect();
    if ship_days.is_empty() {
        return Err(missing("fulfillment.delivery.estimate.ship_days"));
    }
    Ok(ShipCalendar {
        cutoff,
        ship_days,
        transit_days: count(policy, "order.shipping.transit_business_days")?,
    })
}

/// `order.shipping.ships_to`: countries parcels go to.
pub(crate) fn ships_to(policy: &PolicySet) -> Vec<String> {
    words(policy, "order.shipping.ships_to")
}

/// `order.checkout_validation`: the checks marked `required`, in policy order.
pub(crate) fn checkout_checks(policy: &PolicySet) -> Vec<String> {
    policy
        .lookup("order.checkout_validation")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter(|(_, v)| v.as_str() == Some("required"))
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// `order.payment`: which methods the store takes and how.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PaymentRules {
    pub methods: Vec<String>,
    pub loyalty_as_secondary: bool,
    pub guest_allows_loyalty: bool,
    pub installment_methods: Vec<String>,
    pub installment_min: Option<Money>,
    pub installment_months: Vec<u32>,
}

pub(crate) fn payment_rules(policy: &PolicySet) -> Result<PaymentRules, ToolError> {
    let methods = words(policy, "order.payment.methods");
    if methods.is_empty() {
        return Err(missing("order.payment.methods"));
    }
    let flag = |p: &str| policy.lookup(p).and_then(Value::as_bool).unwrap_or(false);
    let installment_min = policy
        .lookup("order.payment.installment.min_amount")
        .map(|v| serde_json::from_value::<Money>(v.clone()))
        .transpose()
        .map_err(|_| missing("order.payment.installment.min_amount"))?;
    let installment_months = policy
        .lookup("order.payment.installment.months")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_u64)
                .filter_map(|n| u32::try_from(n).ok())
                .collect()
        })
        .unwrap_or_default();
    Ok(PaymentRules {
        methods,
        loyalty_as_secondary: flag("order.payment.loyalty_as_secondary"),
        guest_allows_loyalty: flag("order.payment.guest_allows_loyalty"),
        installment_methods: words(policy, "order.payment.installment.methods"),
        installment_min,
        installment_months,
    })
}

/// `order.gift` and `fulfillment.gift_exchange_until`: how gift orders work in this store.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GiftRules {
    /// `address_known`, `recipient_accepts`.
    pub recipient_modes: Vec<String>,
    pub recipient_per_order: u32,
    /// Added once per wrapped gift.
    pub wrap_fee: Money,
    pub receipt_hides_prices: bool,
    /// How long a `recipient_accepts` gift waits for its recipient.
    pub accept_days: u32,
    /// What a recipient may still change (`acceptance`, `address`, `options`); never the price.
    pub recipient_can_change: Vec<String>,
    /// Month and day of the size-exchange deadline, in the year of the order.
    pub exchange_until: (u32, u32),
}

pub(crate) fn gift_rules(policy: &PolicySet) -> Result<GiftRules, ToolError> {
    let wrap_fee = policy
        .lookup("order.gift.wrap_fee")
        .and_then(|v| serde_json::from_value::<Money>(v.clone()).ok())
        // In cents, like every amount the order shows ("5.00", not "5.0").
        .map(|m| acom_promotion::service::for_sales::money(m.value, m.currency))
        .ok_or_else(|| missing("order.gift.wrap_fee"))?;
    let modes = words(policy, "order.gift.recipient_modes");
    if modes.is_empty() {
        return Err(missing("order.gift.recipient_modes"));
    }
    let exchange_until = (
        count(policy, "fulfillment.gift_exchange_until.month")?,
        count(policy, "fulfillment.gift_exchange_until.day")?,
    );
    Ok(GiftRules {
        recipient_modes: modes,
        recipient_per_order: count(policy, "order.gift.recipient_per_order")?,
        wrap_fee,
        receipt_hides_prices: policy
            .lookup("order.gift.gift_receipt_hides_prices")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        accept_days: count(policy, "order.gift.auto_cancel_if_no_address_days")?,
        recipient_can_change: words(policy, "order.gift.recipient_can_change"),
        exchange_until,
    })
}

/// How far apart two orders of a subscription are (`order.subscription.cycle_length`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CycleLength {
    pub days: u32,
    pub months: u32,
}

impl CycleLength {
    /// The charge date one cycle after `from`. A month ahead keeps the day of the month, or
    /// the month's last day when it has no such day.
    pub fn after(self, from: NaiveDate) -> NaiveDate {
        let later = from
            .checked_add_months(Months::new(self.months))
            .unwrap_or(from);
        later
            .checked_add_days(Days::new(u64::from(self.days)))
            .unwrap_or(later)
    }

    /// The first charge date of the schedule through `from` that is after `today`.
    pub fn next_after(self, from: NaiveDate, today: NaiveDate) -> NaiveDate {
        let mut d = from;
        // A cycle always moves forward (checked when the policy is read), so this ends.
        while d <= today {
            let next = self.after(d);
            if next <= d {
                return today.succ_opt().unwrap_or(today);
            }
            d = next;
        }
        d
    }
}

/// `order.subscription` and `order.payment.subscription_card_only`: how standing orders work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubscriptionRules {
    /// The billing cycles this store offers.
    pub billing_cycles: Vec<String>,
    pub cycle_length: BTreeMap<String, CycleLength>,
    /// The discount layers a subscription's orders get (`item_price`, `order_discount`).
    pub discount_layers: Vec<String>,
    pub card_only: bool,
}

impl SubscriptionRules {
    /// The length of `cycle`, when the store offers it.
    pub fn cycle(&self, cycle: &str) -> Option<CycleLength> {
        self.billing_cycles
            .iter()
            .any(|c| c == cycle)
            .then(|| self.cycle_length.get(cycle).copied())
            .flatten()
    }
}

pub(crate) fn subscription_rules(policy: &PolicySet) -> Result<SubscriptionRules, ToolError> {
    let billing_cycles = words(policy, "order.subscription.billing_cycles");
    if billing_cycles.is_empty() {
        return Err(missing("order.subscription.billing_cycles"));
    }
    let mut cycle_length = BTreeMap::new();
    for cycle in &billing_cycles {
        let path = format!("order.subscription.cycle_length.{cycle}");
        let part = |unit: &str| {
            policy
                .lookup(&format!("{path}.{unit}"))
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0)
        };
        let length = CycleLength {
            days: part("days"),
            months: part("months"),
        };
        if length.days == 0 && length.months == 0 {
            return Err(missing(&path));
        }
        cycle_length.insert(cycle.clone(), length);
    }
    Ok(SubscriptionRules {
        billing_cycles,
        cycle_length,
        discount_layers: words(policy, "order.subscription.discount_layers"),
        card_only: policy
            .lookup("order.payment.subscription_card_only")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

/// `order.receipt`: which documents an order can get and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiptRules {
    pub types: Vec<String>,
    /// Only in the store-time month the order was placed.
    pub same_month_only: bool,
    /// What happens to a receipt whose order's amount changed: `cancel_and_reissue`.
    pub after_claim: String,
}

pub(crate) fn receipt_rules(policy: &PolicySet) -> Result<ReceiptRules, ToolError> {
    let types = words(policy, "order.receipt.types");
    if types.is_empty() {
        return Err(missing("order.receipt.types"));
    }
    Ok(ReceiptRules {
        types,
        same_month_only: policy
            .lookup("order.receipt.same_month_only")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        after_claim: policy
            .lookup("order.receipt.after_claim")
            .and_then(Value::as_str)
            .unwrap_or("cancel_and_reissue")
            .to_owned(),
    })
}

/// `order.guest_lookup`: opening an order without signing in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GuestLookupRules {
    /// Wrong tries in a row that lock the order number.
    pub max_attempts: u32,
    pub lock_minutes: u32,
    pub token_ttl_seconds: u32,
}

pub(crate) fn guest_lookup_rules(policy: &PolicySet) -> Result<GuestLookupRules, ToolError> {
    Ok(GuestLookupRules {
        max_attempts: count(policy, "order.guest_lookup.max_attempts")?,
        lock_minutes: count(policy, "order.guest_lookup.lock_minutes")?,
        token_ttl_seconds: count(policy, "order.guest_lookup.token_ttl_seconds")?,
    })
}

/// `order.payment.virtual_account_hold_days`: days a virtual account waits for the money.
pub(crate) fn virtual_account_hold_days(policy: &PolicySet) -> Result<u32, ToolError> {
    count(policy, "order.payment.virtual_account_hold_days")
}

/// `order.change.allowed_before`: the warehouse step (`acom:fulfillment_stage`) from which an
/// order can no longer be changed (its address, its card installments).
pub(crate) fn change_allowed_before(policy: &PolicySet) -> Result<FulfillmentStage, ToolError> {
    let path = "order.change.allowed_before";
    policy
        .lookup(path)
        .and_then(Value::as_str)
        .and_then(|w| w.parse().ok())
        .ok_or_else(|| missing(path))
}

/// The shipping zone every address is in until zone data exists
/// (`fulfillment.delivery.default_zone`): its countries and the surcharge it adds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShippingZone {
    pub name: String,
    pub countries: Vec<String>,
    pub surcharge: Money,
}

pub(crate) fn default_zone(policy: &PolicySet) -> Result<ShippingZone, ToolError> {
    let path = "fulfillment.delivery.default_zone";
    let surcharge = policy
        .lookup(&format!("{path}.surcharge"))
        .and_then(|v| serde_json::from_value::<Money>(v.clone()).ok())
        .map(|m| acom_promotion::service::for_sales::money(m.value, m.currency))
        .ok_or_else(|| missing(&format!("{path}.surcharge")))?;
    Ok(ShippingZone {
        name: policy
            .lookup(&format!("{path}.name"))
            .and_then(Value::as_str)
            .unwrap_or("the default zone")
            .to_owned(),
        countries: words(policy, &format!("{path}.countries")),
        surcharge,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn reads_the_gift_policy() {
        let g = gift_rules(&defaults()).unwrap();
        assert_eq!(g.wrap_fee.value.to_string(), "5.00");
        assert_eq!(g.recipient_modes, ["address_known", "recipient_accepts"]);
        assert_eq!(g.recipient_per_order, 1);
        assert_eq!(g.accept_days, 3);
        assert_eq!(g.exchange_until, (12, 31));
        assert!(g.receipt_hides_prices);
        assert!(g.recipient_can_change.iter().any(|c| c == "options"));
    }

    pub(crate) fn defaults() -> PolicySet {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
        let texts: Vec<(&str, String)> = acom_policy::FILES
            .iter()
            .map(|f| {
                (
                    *f,
                    std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
                )
            })
            .collect();
        PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str()))).unwrap()
    }

    #[test]
    fn reads_the_order_policy() {
        let p = defaults();
        assert_eq!(
            cart_limits(&p).unwrap(),
            CartLimits {
                max_lines: 50,
                max_quantity_per_line: 20
            }
        );
        let cal = ship_calendar(&p).unwrap();
        assert_eq!(cal.cutoff, NaiveTime::from_hms_opt(14, 0, 0).unwrap());
        assert_eq!(cal.ship_days.len(), 6);
        assert_eq!(cal.transit_days, 2);
        assert_eq!(ships_to(&p), vec!["US"]);
        assert_eq!(
            checkout_checks(&p),
            vec![
                "common_payment_method_exists",
                "stock_available",
                "product_sellable",
                "no_domestic_international_mix",
                "server_total_matches_client",
                "price_floor_respected"
            ]
        );
        let pay = payment_rules(&p).unwrap();
        assert_eq!(pay.installment_months, vec![0, 3, 6]);
        assert!(pay.loyalty_as_secondary && !pay.guest_allows_loyalty);
        let r = receipt_rules(&p).unwrap();
        assert_eq!(r.types, ["deduction", "proof", "tax_invoice"]);
        assert!(r.same_month_only);
        let g = guest_lookup_rules(&p).unwrap();
        assert_eq!(
            (g.max_attempts, g.lock_minutes, g.token_ttl_seconds),
            (5, 15, 1800)
        );
        assert_eq!(virtual_account_hold_days(&p).unwrap(), 3);
        assert_eq!(
            change_allowed_before(&p).unwrap(),
            FulfillmentStage::Shipped
        );
        let zone = default_zone(&p).unwrap();
        assert_eq!(zone.countries, ["US"]);
        assert_eq!(zone.surcharge.value.to_string(), "0.00");
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn billing_cycles_come_from_the_policy() {
        let s = subscription_rules(&defaults()).unwrap();
        assert!(s.card_only);
        assert_eq!(s.discount_layers, ["item_price"]);
        let week = s.cycle("every_week").unwrap();
        let two = s.cycle("every_2_weeks").unwrap();
        let month = s.cycle("every_month").unwrap();
        assert_eq!(week.after(day("2026-12-01")), day("2026-12-08"));
        assert_eq!(two.after(day("2026-12-01")), day("2026-12-15"));
        assert_eq!(month.after(day("2026-12-01")), day("2027-01-01"));
        // A month ahead of Jan 31 is the last day of February.
        assert_eq!(month.after(day("2027-01-31")), day("2027-02-28"));
        assert!(s.cycle("every_day").is_none());
        // Resuming on Nov 20 a monthly schedule that was due Nov 1: Dec 1.
        assert_eq!(
            month.next_after(day("2026-11-01"), day("2026-11-20")),
            day("2026-12-01")
        );
        // A date still ahead stays.
        assert_eq!(
            week.next_after(day("2026-12-01"), day("2026-11-20")),
            day("2026-12-01")
        );
    }
}
