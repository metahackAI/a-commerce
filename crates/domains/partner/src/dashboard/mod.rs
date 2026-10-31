//! `partner.dashboard`: one seller's operations at a glance, for the seller's own staff and the
//! store's operators. Which sections each role sees is the store's policy
//! (`dashboard.visible_metrics_by_role`).

pub mod get_seller_overview;

pub use get_seller_overview::GetSellerOverview;

/// The sections an overview has, in the order it lists them.
pub(crate) const SECTIONS: &[&str] = &[
    "sales",
    "settlement",
    "fulfillment",
    "claims",
    "grade",
    "tasks",
];
