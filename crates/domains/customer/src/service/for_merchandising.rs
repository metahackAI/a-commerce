//! What the merchandising domain reads from customers: the tiers a display can be booked for and
//! how many members each reaches (the same reads as the promotion domain uses), and a shopper's
//! own tier, which decides the bookings they see.

pub use super::for_catalog::tier_of;
pub use super::for_promotion::{Tier, tier, tiers};
