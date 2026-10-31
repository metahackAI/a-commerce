//! The `message` resource (docs/api/messaging.md#message): `send_customer_message`, one
//! transactional message to one shopper, and `send_bulk`, a campaign to everyone it reaches.
//! A person always approves, and a sent message cannot be taken back.

mod send_customer_message;

pub use send_customer_message::*;
