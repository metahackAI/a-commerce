//! Who is writing a post or a reply: the store (an operator, or the change-set runner), a
//! signed-in shopper (read through the customer domain's `for_community` service, never its
//! tables), or a guest by their browser session. A shopper's name is masked here, when written,
//! as `moderation.author_masking` says.

use acom_contract::{ErrorCode, ToolError};
use acom_store::{Cx, TxCx};
use serde_json::json;

use crate::is_operator;
use crate::model::WriterKind;
use crate::rules::AuthorMasking;

/// The writer of one post or reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Writer {
    /// `adm_…`, `cus_…` or `ses_…`.
    pub id: String,
    pub kind: WriterKind,
    /// The masked name kept with what they write; `None` for the store and guests.
    pub name: Option<String>,
    /// The shopper's account state; `None` for the store and guests, or an unknown shopper.
    pub member_state: Option<String>,
}

/// The caller as a writer. Callers without an account or a browser session must sign in.
pub(crate) async fn writer(cx: &Cx, tx: &mut TxCx) -> Result<Writer, ToolError> {
    let p = &cx.principal;
    if is_operator(p) {
        return Ok(Writer {
            id: p.subject.clone(),
            kind: WriterKind::Staff,
            name: None,
            member_state: None,
        });
    }
    if let Some(customer) = p.customer_id() {
        let masking = AuthorMasking::from_policy(&cx.policy)?;
        let found = acom_customer::service::for_community::writer(tx, customer).await?;
        return Ok(Writer {
            id: customer.to_owned(),
            kind: WriterKind::Customer,
            name: found
                .as_ref()
                .map(|w| masking.shown(&w.name))
                .filter(|n| !n.is_empty()),
            member_state: found.map(|w| w.member_state),
        });
    }
    if p.subject.starts_with("ses_") {
        return Ok(Writer {
            id: p.subject.clone(),
            kind: WriterKind::Guest,
            name: None,
            member_state: None,
        });
    }
    Err(
        ToolError::new(ErrorCode::FORBIDDEN, "Sign in to write on the community boards.")
            .detail(json!({"reason": "owner"})),
    )
}
