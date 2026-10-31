//! The slot where `partner` hands integration a seller's status. Partner and integration are
//! peers in the domain graph (docs/backend/README.md §2.1: both sit on catalog, sales and
//! fulfillment), so integration cannot call partner; `acom-registry` installs partner's reader
//! here once partner exports one. Until then a seller feed is taken without that check, and the
//! import's preview says so.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use acom_contract::ToolError;
use acom_store::TxCx;

/// A future that reads inside the caller's transaction.
pub type Reading<'a, T> = Pin<Box<dyn Future<Output = Result<T, ToolError>> + Send + 'a>>;

/// A seller's `acom:sellerStatus` word (`active`, `suspended`, …), or `None` when there is no
/// such seller.
pub type SellerStatusReader = for<'a> fn(&'a mut TxCx, &'a str) -> Reading<'a, Option<String>>;

static SELLERS: OnceLock<SellerStatusReader> = OnceLock::new();

/// Partner hands its reader over once at start; later calls are ignored.
pub fn provide_seller_status(reader: SellerStatusReader) {
    let _ = SELLERS.set(reader);
}

/// What the store knows about a seller: `Ok(None)` when no reader is installed (not checked),
/// `Ok(Some(None))` when there is no such seller, `Ok(Some(Some(status)))` otherwise.
pub(crate) async fn seller_status(
    tx: &mut TxCx,
    seller_id: &str,
) -> Result<Option<Option<String>>, ToolError> {
    match SELLERS.get() {
        Some(read) => read(tx, seller_id).await.map(Some),
        None => Ok(None),
    }
}
