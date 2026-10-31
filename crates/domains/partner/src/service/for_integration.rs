//! What `integration` reads from `partner`: a seller's status, so a seller's product feed is
//! imported only while the seller is active. Partner and integration are peers in the domain
//! graph, so `acom-registry` hands this reader to integration's slot
//! (`acom_integration::service::for_partner::provide_seller_status`).

use std::future::Future;
use std::pin::Pin;

use acom_contract::ToolError;
use acom_store::TxCx;

/// The seller's `acom:seller_status` word (`active`, `suspended`, …), or `None` when the store
/// has no such seller.
pub fn seller_status<'a>(
    tx: &'a mut TxCx,
    seller_id: &'a str,
) -> Pin<Box<dyn Future<Output = Result<Option<String>, ToolError>> + Send + 'a>> {
    Box::pin(async move {
        Ok(crate::repo::seller(tx, seller_id)
            .await?
            .map(|s| s.seller_status))
    })
}
