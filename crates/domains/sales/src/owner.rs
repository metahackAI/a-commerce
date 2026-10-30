//! Whose cart and orders a request is about. A signed-in shopper's cart belongs to their
//! customer id; a guest's cart belongs to the browser session. Never taken from the input.

use acom_auth::Principal;
use acom_contract::ToolError;
use serde::{Deserialize, Serialize};

use crate::not_signed_in;

/// The owner of a cart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Owner {
    /// The signed-in shopper (`cus_…`).
    pub customer_id: Option<String>,
    /// The guest session (`ses_…`) when nobody is signed in.
    pub session_id: Option<String>,
}

/// The cart owner for this caller, and the session whose guest cart a signed-in shopper takes
/// over (their cart from before they signed in).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Owners {
    pub owner: Owner,
    pub guest_session: Option<String>,
}

pub(crate) fn owners(p: &Principal) -> Result<Owners, ToolError> {
    if let Some(c) = p.customer_id() {
        return Ok(Owners {
            owner: Owner {
                customer_id: Some(c.to_owned()),
                session_id: None,
            },
            guest_session: p.session_id.clone(),
        });
    }
    let session = p
        .session_id
        .clone()
        .or_else(|| p.subject.starts_with("ses_").then(|| p.subject.clone()));
    match session {
        Some(s) => Ok(Owners {
            owner: Owner {
                customer_id: None,
                session_id: Some(s),
            },
            guest_session: None,
        }),
        None => Err(not_signed_in()),
    }
}

/// Who the caller is for orders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Viewer {
    /// A signed-in shopper sees only their own orders.
    Shopper(String),
    /// A guest sees no orders.
    Guest,
    /// An operator (or the system) sees every order of the store.
    Staff,
}

pub(crate) fn viewer(p: &Principal) -> Viewer {
    match p.customer_id() {
        Some(c) => Viewer::Shopper(c.to_owned()),
        None if p.subject.starts_with("ses_") => Viewer::Guest,
        None => Viewer::Staff,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_auth::{Surface, scopes};
    use acom_core::TenantId;
    use acom_core::enums::ActorKind;

    fn principal(subject: &str, session: Option<&str>) -> Principal {
        Principal {
            subject: subject.into(),
            kind: ActorKind::Shopper,
            tenant: TenantId::parse("store_northwind").unwrap(),
            surface: Surface::Fo,
            scopes: scopes::shopper(),
            name: None,
            session_id: session.map(str::to_owned),
        }
    }

    #[test]
    fn signed_in_shoppers_own_by_customer_and_guests_by_session() {
        let jamie = owners(&principal("cus_jamie", Some("ses_7"))).unwrap();
        assert_eq!(jamie.owner.customer_id.as_deref(), Some("cus_jamie"));
        assert_eq!(jamie.owner.session_id, None);
        assert_eq!(jamie.guest_session.as_deref(), Some("ses_7"));
        let mina = owners(&principal("ses_dev_guest", Some("ses_dev"))).unwrap();
        assert_eq!(mina.owner.session_id.as_deref(), Some("ses_dev"));
        assert_eq!(mina.guest_session, None);
        let bare = owners(&principal("ses_bare", None)).unwrap();
        assert_eq!(bare.owner.session_id.as_deref(), Some("ses_bare"));
        assert!(owners(&principal("adm_sj", None)).is_err());
        assert_eq!(
            viewer(&principal("cus_alex", None)),
            Viewer::Shopper("cus_alex".into())
        );
        assert_eq!(viewer(&principal("ses_x", None)), Viewer::Guest);
        assert_eq!(viewer(&principal("adm_sj", None)), Viewer::Staff);
    }
}
