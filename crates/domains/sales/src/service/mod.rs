//! What other domains may use from `sales`: plain async functions over `TxCx`/`Cx`, never
//! this domain's tables directly. A domain that needs something adds its own file here
//! (`for_<that_domain>.rs`) and one `pub mod` line below.

pub mod for_analytics;
pub mod for_catalog;
pub mod for_customer;
pub mod for_fulfillment;
pub mod for_integration;
pub mod for_messaging;
pub mod for_partner;
pub mod for_promotion;
pub mod order_delivery;
pub mod providers;
