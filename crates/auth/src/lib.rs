//! Who is calling and what they may do.
//!
//! The gateway is the only signer of internal tokens; commerce and agent only verify them.
//! `human:*` scopes are never present on tokens minted for chat turns, so no prompt can make an
//! agent approve, order, return, or undo. Design: `docs/backend/gateway.md`. Token signing and
//! verification arrive in M3; the principal type is shared from the start.

use std::collections::BTreeSet;

use acom_core::TenantId;
use acom_core::enums::ActorKind;
use serde::{Deserialize, Serialize};

/// Which surface a request came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// Back office (operators).
    Bo,
    /// Storefront (shoppers).
    Fo,
    /// Outside agents over MCP.
    Mcp,
    /// Scheduled jobs and the change-set runner.
    System,
}

/// The caller of one request, as stated by the gateway-signed internal token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    /// The person (`adm_…`, `cus_…`), outside client, or system actor (`sys_…`).
    pub subject: String,
    pub kind: ActorKind,
    pub tenant: TenantId,
    pub surface: Surface,
    pub scopes: BTreeSet<String>,
}

impl Principal {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }

    /// Reads need `tools:read:{domain}`.
    pub fn can_read(&self, domain: &str) -> bool {
        self.has_scope(&format!("tools:read:{domain}"))
    }

    /// Previews and no-approval writes need `tools:write:{domain}`.
    pub fn can_write(&self, domain: &str) -> bool {
        self.has_scope(&format!("tools:write:{domain}"))
    }

    /// A person-only action (`human:approve`, `human:order`, …). Never true for an agent.
    pub fn can_act_as_person(&self, scope: &str) -> bool {
        scope.starts_with("human:")
            && matches!(self.kind, ActorKind::Operator | ActorKind::Shopper)
            && self.has_scope(scope)
    }

    /// The system actor that runs approved change sets and scheduled jobs.
    pub fn system(tenant: TenantId, subject: &str) -> Self {
        Self {
            subject: subject.to_owned(),
            kind: ActorKind::System,
            tenant,
            surface: Surface::System,
            scopes: BTreeSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn principal(kind: ActorKind, scopes: &[&str]) -> Principal {
        Principal {
            subject: "cus_jamie".into(),
            kind,
            tenant: TenantId::parse("store_northwind").unwrap(),
            surface: Surface::Fo,
            scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn agents_never_act_as_people_even_with_the_scope() {
        let agent = principal(
            ActorKind::AgentOnBehalf,
            &["human:order", "tools:write:sales"],
        );
        assert!(agent.can_write("sales"));
        assert!(!agent.can_act_as_person("human:order"));
        let shopper = principal(ActorKind::Shopper, &["human:order"]);
        assert!(shopper.can_act_as_person("human:order"));
        assert!(!shopper.can_act_as_person("tools:write:sales"));
    }
}
