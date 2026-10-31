//! The `channel` resource (docs/api/integration.md `## channel`): the outside systems the store
//! is connected to, how their status words map to ours, and the secrets they are reached and
//! checked with. Secrets are kept only as hashes and never returned.

pub(crate) mod list_channels;
pub(crate) mod register_channel;

pub use list_channels::ListChannels;
pub use register_channel::RegisterChannel;

use acom_contract::{ErrorCode, ToolError};
use acom_store::TxCx;
use serde_json::{Value, json};

use crate::repo::{self, ChannelRow};
use crate::{CHANNEL_NOT_REGISTERED, check_id};

/// A channel as reads show it: never its secrets, only whether it can check signed events.
pub(crate) fn channel_json(c: &ChannelRow, time: &acom_core::StoreTime) -> Value {
    json!({
        "id": c.id,
        "name": c.name,
        "acom:channel_type": c.channel_type,
        "url": c.url,
        "acom:connection_status": c.connection_status,
        "acom:last_synced_at": c.last_synced_at.map(|t| time.format(t)),
        "seller_id": c.seller_id,
        "acom:status_mapping": c.status_mapping,
        "acom:signs_events": c.signing_secret_hash.is_some(),
    })
}

/// A channel an operator names, or `channel_not_registered` (`code` lets an event sender get
/// `unknown_channel` instead).
pub(crate) async fn registered(
    tx: &mut TxCx,
    channel_id: &str,
    code: ErrorCode,
) -> Result<ChannelRow, ToolError> {
    check_id("channel_id", "chn", channel_id)?;
    repo::channel(tx, channel_id).await?.ok_or_else(|| {
        let e = ToolError::new(
            code,
            format!("No channel {channel_id} is registered in this store."),
        )
        .detail(json!({"entity": "channel", "id": channel_id}));
        if code == CHANNEL_NOT_REGISTERED {
            e.next("register_channel")
        } else {
            e
        }
    })
}
