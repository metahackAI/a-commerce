//! What other domains may use from `promotion`: plain async functions over `TxCx`/`Cx`, never
//! this domain's tables directly. A domain that needs something adds its own file here
//! (`for_<that_domain>.rs`) and one `pub mod` line below.

pub mod for_fulfillment;
