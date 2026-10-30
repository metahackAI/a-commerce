//! Scope sets (docs/backend/gateway.md §2).
//!
//! Operators get their scopes from their roles (`platform.role_scope`); shoppers and guests get the
//! fixed sets below. Agents acting for a person get the person's set minus `human:*`
//! (`Principal::on_behalf`).

use std::collections::BTreeSet;

/// Every domain of the API hierarchy (docs/api/README.md §1).
pub const DOMAINS: &[&str] = &[
    "catalog",
    "merchandising",
    "search",
    "sales",
    "fulfillment",
    "promotion",
    "customer",
    "messaging",
    "community",
    "platform",
    "analytics",
    "partner",
    "integration",
];

/// Person-only scopes (each gates a person-only endpoint).
pub const HUMAN: &[&str] = &[
    "human:approve",
    "human:reject",
    "human:undo",
    "human:task",
    "human:order",
    "human:cancel",
    "human:return",
    "human:exchange",
    "human:claim",
    "human:account",
];

/// `tools:read:*` and `tools:write:*` for every domain.
pub fn all_tool_scopes() -> BTreeSet<String> {
    DOMAINS
        .iter()
        .flat_map(|d| [format!("tools:read:{d}"), format!("tools:write:{d}")])
        .collect()
}

/// The platform administrator: every tool scope, every person-only scope, demo controls.
pub fn admin() -> BTreeSet<String> {
    let mut s = all_tool_scopes();
    s.extend(HUMAN.iter().map(|h| (*h).to_owned()));
    s.insert("changeset:draft".to_owned());
    s.insert("admin:demo".to_owned());
    s
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// A signed-in shopper: browse, keep a cart, prepare and place orders, ask for returns, review
/// products, manage their own profile and remembered preferences, and read the messages the store
/// sent them. Which tools of a domain a
/// shopper reaches is decided by each tool's audience (acom-http), not by these scopes alone.
pub fn shopper() -> BTreeSet<String> {
    set(&[
        "tools:read:search",
        "tools:read:catalog",
        "tools:write:catalog",
        "tools:read:merchandising",
        "tools:read:promotion",
        "tools:read:sales",
        "tools:write:sales",
        "tools:read:customer",
        "tools:write:customer",
        "tools:read:fulfillment",
        "tools:read:messaging",
        "human:order",
        "human:cancel",
        "human:return",
        "human:exchange",
        "human:claim",
        "human:undo",
        "human:account",
    ])
}

/// A guest: browse and keep a cart tied to the browser session. Signing in is needed to order.
pub fn guest() -> BTreeSet<String> {
    set(&[
        "tools:read:search",
        "tools:read:catalog",
        "tools:read:merchandising",
        "tools:read:promotion",
        "tools:read:sales",
        "tools:write:sales",
        "tools:read:customer",
        "tools:write:customer",
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guests_cannot_order_and_admins_can_do_everything() {
        assert!(!guest().contains("human:order"));
        assert!(shopper().contains("human:order"));
        let a = admin();
        assert!(
            a.contains("human:approve")
                && a.contains("tools:write:promotion")
                && a.contains("admin:demo")
        );
    }
}
