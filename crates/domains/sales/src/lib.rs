//! The `sales` domain: cart, order preparation, orders, payments, receipts, subscriptions, gift orders.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/sales.md`.
//! Layout: one module per resource (`cart`, `checkout`, `order`, `payment`), each operation one
//! type implementing `ReadOp`/`WriteOp`/`PersonOp`; `pricing` reads the shopper's cart with
//! every benefit from promotion's `calculate_benefits`; `repo` holds the SQL; `owner`, `merge`,
//! `delivery` and `rules` are pure rules with unit tests. Other domains are reached only through
//! their `service` modules.

pub mod service;

mod cart;
mod checkout;
mod delivery;
mod gift;
mod merge;
mod order;
mod owner;
mod payment;
mod pricing;
mod repo;
mod rules;
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

/// Adds every sales operation to the registry, and installs the readers other domains use to
/// see sales data (they cannot depend on sales): the cart for promotion's
/// `calculate_benefits`, orders and products for customer's tier progress and wishlist.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    acom_promotion::service::for_sales::provide_cart_reader(service::for_promotion::benefit_cart);
    acom_customer::service::providers::provide_member_orders(service::for_customer::member_orders);
    acom_customer::service::providers::provide_product_cards(service::for_customer::product_cards);
    b.read(cart::GetCart)
        .write(cart::AddToCart)
        .write(cart::UpdateCartItem)
        .write(cart::RemoveCartItem)
        .write(cart::ClearCart)
        .read(checkout::GetPaymentMethods)
        .write(order::PrepareOrder)
        .person(order::PlaceOrder)
        .read(order::GetMyOrders)
        .read(order::GetOrder)
        .read(order::SearchOrders)
        .read(payment::GetPayment)
        .write(gift::PrepareGiftOrder)
        .person(gift::AcceptGiftOrder);
    // Scheduled work (platform.job): the stock-out forecast over recent sales.
    b.job(watch::StockOutForecast);
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
        assert_eq!(registry.len(), 14);
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
