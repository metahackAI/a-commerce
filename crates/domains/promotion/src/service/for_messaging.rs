//! What `messaging` reads from promotion: who is in a segment now, for a campaign sent to a
//! segment. Messaging does not depend on promotion, so `acom_registry::build` hands this reader
//! to it (`acom_messaging::service::providers`).

use std::collections::BTreeSet;

use acom_contract::ToolError;
use acom_store::TxCx;

use crate::segment;

/// The shoppers in a segment now (`cus_` ids, active members who meet its conditions), or
/// `None` when the store has no such segment.
pub async fn segment_members(
    tx: &mut TxCx,
    segment_id: &str,
) -> Result<Option<Vec<String>>, ToolError> {
    let Some(s) = segment::segment(tx, segment_id).await? else {
        return Ok(None);
    };
    let now = tx.clock.now();
    let mut seen = BTreeSet::from([s.id.clone()]);
    let members = segment::members(tx, &s.rules(), now, &mut seen).await?;
    Ok(Some(members.into_iter().map(|m| m.customer_id).collect()))
}
