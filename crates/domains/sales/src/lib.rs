//! The `sales` domain: cart, order preparation, orders, payments, receipts, subscriptions, gift orders.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/sales.md`.
//! Layout follows the API hierarchy `sales.{resource}.{operation}`: one folder per resource
//! (`cart`, `checkout`, `order`, `payment`, `gift_order`), one file per operation
//! (`src/{resource}/{tool}.rs`), each operation one type implementing `ReadOp`/`WriteOp`/
//! `PersonOp`; what a resource's operations share is in its `mod.rs`. Domain-wide: `pricing`
//! reads the shopper's cart with every benefit from promotion's `calculate_benefits`; `repo` holds
//! the SQL; `owner`, `merge`, `delivery` and `rules` are pure rules with unit tests; `watch` holds
//! the scheduled jobs. Other domains are reached only through their `service` modules.

pub mod service;

mod cart;
mod checkout;
mod delivery;
mod gift_order;
mod installment;
mod merge;
mod order;
mod owner;
mod payment;
mod pricing;
mod receipt;
mod repo;
mod rules;
mod subscription;
#[cfg(test)]
mod testdb;
mod watch;

use acom_contract::{ApprovalNeed, ErrorClass, ErrorCode, RegistryBuilder, ToolError};
use acom_store::Cx;
use serde_json::{Value, json};

pub const CART_EMPTY: ErrorCode = ErrorCode::new("cart_empty", ErrorClass::Domain);
pub const OUT_OF_STOCK: ErrorCode = ErrorCode::new("out_of_stock", ErrorClass::Domain);
pub const NO_COMMON_PAYMENT_METHOD: ErrorCode =
    ErrorCode::new("no_common_payment_method", ErrorClass::Domain);
pub const MIXED_ORDER_TYPE: ErrorCode = ErrorCode::new("mixed_order_type", ErrorClass::Domain);
pub const PAYMENT_DECLINED: ErrorCode = ErrorCode::new("payment_declined", ErrorClass::Domain);
pub const ADDRESS_NOT_DELIVERABLE: ErrorCode =
    ErrorCode::new("address_not_deliverable", ErrorClass::Domain);
/// More lines, or more of one line, than `cart.max_lines` / `cart.max_quantity_per_line`.
pub const PURCHASE_LIMIT_EXCEEDED: ErrorCode =
    ErrorCode::new("purchase_limit_exceeded", ErrorClass::LimitExceeded);
pub const PRODUCT_NOT_FOUND: ErrorCode = ErrorCode::new("product_not_found", ErrorClass::NotFound);
pub const VARIANT_NOT_FOUND: ErrorCode = ErrorCode::new("variant_not_found", ErrorClass::NotFound);
pub const CART_LINE_NOT_FOUND: ErrorCode =
    ErrorCode::new("cart_line_not_found", ErrorClass::NotFound);
pub const ORDER_NOT_FOUND: ErrorCode = ErrorCode::new("order_not_found", ErrorClass::NotFound);
pub const ADDRESS_NOT_FOUND: ErrorCode = ErrorCode::new("address_not_found", ErrorClass::NotFound);
/// Neither a signed-in shopper nor a browser session (or a guest asking for orders).
pub const NOT_SIGNED_IN: ErrorCode = ErrorCode::new("not_signed_in", ErrorClass::Unauthenticated);
pub const INVALID_FILTER: ErrorCode = ErrorCode::new("invalid_filter", ErrorClass::InvalidInput);
/// A gift's recipient did not accept it in time (`gift.auto_cancel_if_no_address_days`).
pub const GIFT_EXPIRED: ErrorCode = ErrorCode::new("gift_expired", ErrorClass::Domain);
/// A subscription or installments need a card, and there is none (or the payment is not one).
pub const NOT_CARD_PAYMENT: ErrorCode = ErrorCode::new("not_card_payment", ErrorClass::Domain);
/// Installments in a number of months the store does not offer (`payment.installment.months`).
pub const MONTHS_NOT_AVAILABLE: ErrorCode =
    ErrorCode::new("months_not_available", ErrorClass::Domain);
/// Installments on an order under `payment.installment.min_amount`.
pub const BELOW_MIN_AMOUNT: ErrorCode = ErrorCode::new("below_min_amount", ErrorClass::Domain);
/// `verify_guest`: the order number and the email or phone do not match an order (the same
/// answer whether or not such an order exists).
pub const GUEST_MATCH_FAILED: ErrorCode = ErrorCode::new("guest_match_failed", ErrorClass::Domain);
/// `verify_guest`: too many wrong tries on an order number (`guest_lookup.max_attempts`).
pub const TOO_MANY_ATTEMPTS: ErrorCode = ErrorCode::new("too_many_attempts", ErrorClass::Locked);
/// The order has left the warehouse (`order.change.allowed_before`).
pub const ALREADY_SHIPPED: ErrorCode = ErrorCode::new("already_shipped", ErrorClass::Domain);
/// A virtual account's deposit came after `payment.virtual_account_hold_days`.
pub const DEADLINE_PASSED: ErrorCode = ErrorCode::new("deadline_passed", ErrorClass::Domain);
/// The deposit is not the amount the payment is for.
pub const AMOUNT_MISMATCH: ErrorCode = ErrorCode::new("amount_mismatch", ErrorClass::Domain);
/// The order already has a receipt of that type.
pub const ALREADY_ISSUED: ErrorCode = ErrorCode::new("already_issued", ErrorClass::AlreadyExists);
/// A tax invoice without the buyer's business tax id.
pub const BUSINESS_ID_REQUIRED: ErrorCode =
    ErrorCode::new("business_id_required", ErrorClass::InvalidInput);

/// Adds every sales operation to the registry, and installs the readers other domains use to
/// see sales data (they cannot depend on sales): the cart for promotion's
/// `calculate_benefits`, orders and products for customer's tier progress and wishlist.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    acom_promotion::service::for_sales::provide_cart_reader(service::for_promotion::benefit_cart);
    acom_customer::service::providers::provide_member_orders(service::for_customer::member_orders);
    acom_customer::service::providers::provide_product_cards(service::for_customer::product_cards);
    b.read(cart::get_cart::GetCart)
        .write(cart::add_to_cart::AddToCart)
        .write(cart::update_cart_item::UpdateCartItem)
        .write(cart::remove_cart_item::RemoveCartItem)
        .write(cart::clear_cart::ClearCart)
        .read(checkout::get_payment_methods::GetPaymentMethods)
        .write(order::prepare_order::PrepareOrder)
        .person(order::place_order::PlaceOrder)
        .read(order::get_my_orders::GetMyOrders)
        .read(order::get_order::GetOrder)
        .read(order::search_orders::SearchOrders)
        .write(order::verify_guest::VerifyGuest)
        .write(order::change_shipping_address::ChangeShippingAddress)
        .read(payment::get_payment::GetPayment)
        .write(payment::confirm_virtual_account_deposit::ConfirmVirtualAccountDeposit)
        .write(receipt::issue_receipt::IssueReceipt)
        .read(receipt::get_receipt::GetReceipt)
        .write(subscription::prepare_subscription::PrepareSubscription)
        .person(subscription::start_subscription::StartSubscription)
        .write(subscription::pause_subscription::PauseSubscription)
        .write(subscription::resume_subscription::ResumeSubscription)
        .write(subscription::cancel_subscription::CancelSubscription)
        .write(subscription::update_schedule::UpdateSchedule)
        .read(subscription::list_subscriptions::ListSubscriptions)
        .write(installment::set_installment::SetInstallment)
        .write(gift_order::prepare_gift_order::PrepareGiftOrder)
        .read(gift_order::get_gift_order::GetGiftOrder)
        .person(gift_order::accept_gift_order::AcceptGiftOrder);
    // Scheduled work (platform.job): the stock-out forecast over recent sales, and the order of
    // a subscription on each charge date.
    b.job(watch::StockOutForecast);
    b.job(subscription::renew::RenewSubscription);
}

pub(crate) fn not_signed_in() -> ToolError {
    ToolError::new(
        NOT_SIGNED_IN,
        "Sign in first; this needs a shopper account or a browser session.",
    )
}

pub(crate) fn cart_empty() -> ToolError {
    ToolError::new(CART_EMPTY, "The cart is empty.").next("add_to_cart")
}

pub(crate) fn address_not_found(id: &str) -> ToolError {
    ToolError::new(
        ADDRESS_NOT_FOUND,
        format!("You have no saved address {id}."),
    )
    .detail(json!({"entity": "postal_address", "id": id}))
    .next("get_my_profile")
}

/// What a write reports about a system outside this store it would tell (a card provider, a
/// bank, a tax service): those are not connected, so nothing was sent and the result is only
/// recorded here. Deterministic, and said in the output so nobody takes it for the real thing.
pub(crate) fn outside_system(system: &str, note: &str) -> Value {
    json!({"system": system, "connected": false, "note": note})
}

/// The store's calendar day now (store time zone).
pub(crate) fn today(
    clock: &dyn acom_core::Clock,
    time: &acom_core::StoreTime,
) -> chrono::NaiveDate {
    clock.now().with_timezone(&time.zone).date_naive()
}

/// A day input (`2026-12-01`).
pub(crate) fn parse_day(field: &str, s: &str) -> Result<chrono::NaiveDate, ToolError> {
    chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").map_err(|_| {
        ToolError::invalid_input(format!("`{field}` is not a day like 2026-12-01."))
            .detail(json!({"field": field, "reason": "format"}))
    })
}

/// What `approval.yml` says about a write, from the facts its preview computed.
pub(crate) fn approval(cx: &Cx, tool: &str, facts: &Value) -> ApprovalNeed {
    let a = cx.policy.approval_for(tool, facts);
    if a.needed {
        ApprovalNeed::Needed {
            reason: a
                .reason
                .unwrap_or_else(|| "Needs a person's approval".into()),
        }
    } else {
        ApprovalNeed::NotNeeded
    }
}

/// A time input in store time.
pub(crate) fn parse_time(
    cx: &Cx,
    field: &str,
    s: Option<&str>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, ToolError> {
    s.map(|s| {
        cx.time.parse(s).map_err(|e| {
            ToolError::invalid_input(e.to_string())
                .detail(json!({"field": field, "reason": "format"}))
        })
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use acom_contract::{Kind, RegistryBuilder};

    #[test]
    fn every_operation_registers_with_a_closed_schema() {
        let mut b = RegistryBuilder::default();
        super::register(&mut b);
        let registry = b.build().unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(registry.len(), 28);
        for op in registry.iter() {
            let m = op.meta();
            assert_eq!(m.domain(), "sales", "{}", m.tool);
            assert!(m.path.starts_with("/v1/sales/"), "{}", m.tool);
            match m.kind {
                Kind::Person => assert_eq!(m.person_scope, "human:order"),
                _ => assert!(m.person_scope.is_empty(), "{}", m.tool),
            }
        }
    }
}
