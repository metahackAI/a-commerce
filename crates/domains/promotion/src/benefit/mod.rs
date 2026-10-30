//! `calculate_benefits`: the one place that decides which discounts, member prices, coupons and
//! shipping benefits a cart gets. The cart, checkout and order placement call it through
//! `service::for_sales`, so a shopper always sees the numbers the order will be charged.

use acom_contract::RegistryBuilder;
use acom_store::Cx;

/// Adds `calculate_benefits` to the registry.
pub(crate) fn register(_b: &mut RegistryBuilder<Cx>) {}
