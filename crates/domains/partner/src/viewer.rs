//! Who is asking. Partner data is for the store's operators and for the seller's own staff: an
//! operator who belongs to a seller's vendor organization (`partner.seller.organization_id`)
//! acts for that seller. They see only their seller, in full (their own tax id and payout
//! account), and its dashboard with the `seller` role; store operators see every seller with
//! sensitive fields masked (`privacy` in `partner.yml`). Shoppers and outside agents see nothing.

use acom_contract::{ErrorCode, ToolError};
use acom_core::enums::ActorKind;
use acom_store::{Cx, TxCx};
use serde_json::json;

use crate::repo;

/// The caller of a partner operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Viewer {
    /// A store operator, our agent acting for one, or the system.
    Store,
    /// Staff of this seller (`slr_…`).
    Seller(String),
}

impl Viewer {
    /// The dashboard role (`dashboard.visible_metrics_by_role`).
    pub(crate) fn role(&self) -> &'static str {
        match self {
            Viewer::Store => "operator",
            Viewer::Seller(_) => "seller",
        }
    }

    /// Whether sensitive fields of this seller are shown in full.
    pub(crate) fn sees_unmasked(&self, seller_id: &str) -> bool {
        matches!(self, Viewer::Seller(s) if s == seller_id)
    }

    /// A seller's staff may look at their own seller only.
    pub(crate) fn may_see(&self, seller_id: &str) -> Result<(), ToolError> {
        match self {
            Viewer::Seller(s) if s != seller_id => Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                "A seller's staff can only see their own seller.",
            )
            .detail(json!({"reason": "owner"}))),
            _ => Ok(()),
        }
    }

    /// Changes to the store's terms with a seller are the store's to make.
    pub(crate) fn store_only(&self, what: &str) -> Result<(), ToolError> {
        match self {
            Viewer::Store => Ok(()),
            Viewer::Seller(_) => Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                format!("Only the store's operators can {what}."),
            )
            .detail(json!({"reason": "role"}))),
        }
    }
}

/// Refuses shoppers, guests and outside agents before anything is read.
pub(crate) fn staff(cx: &Cx) -> Result<(), ToolError> {
    let p = &cx.principal;
    match p.kind {
        ActorKind::System => Ok(()),
        ActorKind::Operator | ActorKind::AgentOnBehalf if p.subject.starts_with("adm_") => Ok(()),
        _ => Err(ToolError::new(
            ErrorCode::FORBIDDEN,
            "Sellers, contracts and grades are for the store's operators.",
        )
        .detail(json!({"reason": "role"}))),
    }
}

/// Decides who is asking, inside the caller's read transaction.
pub(crate) async fn viewer(cx: &Cx, tx: &mut TxCx) -> Result<Viewer, ToolError> {
    staff(cx)?;
    let p = &cx.principal;
    if p.kind == ActorKind::System {
        return Ok(Viewer::Store);
    }
    let org = acom_platform::service::for_partner::organization_of(tx, &p.subject).await?;
    if let Some(org) = org
        && let Some(seller) = repo::seller_of_organization(tx, &org).await?
    {
        return Ok(Viewer::Seller(seller));
    }
    Ok(Viewer::Store)
}
