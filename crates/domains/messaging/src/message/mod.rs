//! The `message` resource (docs/api/messaging.md#message): `send_customer_message`, one
//! transactional message to one shopper, and `send_bulk`, a campaign to everyone it reaches.
//! A person always approves, and a sent message cannot be taken back.

mod send_bulk;
mod send_customer_message;

pub(crate) use send_bulk::ready;
pub use send_bulk::*;
pub use send_customer_message::*;
