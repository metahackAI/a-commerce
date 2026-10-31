//! The `service` resource (docs/api/analytics.md#service): `get_cs_overview`, customer service
//! figures. (Analytics exports nothing to other domains, so this folder is the API resource
//! `analytics.service`, not a `service/for_{domain}.rs` module.)

mod get_cs_overview;

pub use get_cs_overview::*;
