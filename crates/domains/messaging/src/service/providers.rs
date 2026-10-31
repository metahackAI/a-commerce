//! Readers that messaging uses from domains it does not depend on, installed at start-up by the
//! registry (`acom_registry::build`): the members of a segment, which promotion keeps. Each reader
//! is a plain function over the caller's transaction. A campaign for a segment fails with a clear
//! error when its reader is missing, rather than guessing who is in it.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use acom_contract::ToolError;
use acom_store::TxCx;

/// What a reader returns: a boxed future borrowing the transaction.
pub type Reading<'a, T> = Pin<Box<dyn Future<Output = Result<T, ToolError>> + Send + 'a>>;

/// The shoppers in a segment now (`cus_` ids), or `None` when the store has no such segment.
pub type SegmentMembersReader =
    for<'a> fn(&'a mut TxCx, &'a str) -> Reading<'a, Option<Vec<String>>>;

static SEGMENT_MEMBERS: OnceLock<SegmentMembersReader> = OnceLock::new();

/// Installs the segment reader (once; later calls keep the first).
pub fn provide_segment_members(reader: SegmentMembersReader) {
    let _ = SEGMENT_MEMBERS.set(reader);
}

pub(crate) fn segment_members() -> Option<SegmentMembersReader> {
    SEGMENT_MEMBERS.get().copied()
}
