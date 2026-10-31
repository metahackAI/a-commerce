//! What other domains may use from `catalog`: plain async functions over `TxCx`/`Cx`, never
//! this domain's tables directly. A domain that needs something adds its own file here
//! (`for_<that_domain>.rs`) and one `pub mod` line below.

pub mod for_fulfillment;
pub mod for_integration;
pub mod for_merchandising;
pub mod for_partner;
pub mod for_promotion;
pub mod for_sales;
pub mod for_search;
