//! The `role` resource: roles are named bundles of scopes (`platform.role`, `platform.role_scope`);
//! the gateway reads them to build an operator's token (docs/backend/gateway.md §2). Shared here:
//! the role row, the scope catalog people choose from, and the scope checks.

pub mod assign_role;
pub mod create_role;
pub mod get_role;
pub mod list_permissions;
pub mod list_roles;

use std::collections::BTreeSet;

use acom_contract::{Check, ErrorClass, ErrorCode, ToolError};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::repo::db;
use crate::rules::{ApiKeyRules, RoleRules};

/// No role with that id.
pub const ROLE_NOT_FOUND: ErrorCode = ErrorCode::new("role_not_found", ErrorClass::NotFound);
/// Another role already has this name.
pub const DUPLICATE_ROLE: ErrorCode = ErrorCode::new("duplicate_role", ErrorClass::AlreadyExists);
/// A scope that is not in the store's scope catalog (`roles.known_scopes`).
pub const INVALID_SCOPE: ErrorCode = ErrorCode::new("invalid_scope", ErrorClass::InvalidInput);
/// The account already has the role.
pub const ALREADY_ASSIGNED: ErrorCode =
    ErrorCode::new("already_assigned", ErrorClass::AlreadyExists);
/// The account does not have the role it should lose.
pub const NOT_ASSIGNED: ErrorCode = ErrorCode::new("not_assigned", ErrorClass::InvalidState);

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct RoleRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub approver_roles: Vec<String>,
    pub is_system: bool,
    pub version: i32,
}

pub(crate) async fn role(tx: &mut TxCx, id: &str) -> Result<Option<RoleRow>, ToolError> {
    sqlx::query_as(
        "SELECT id, name, description, approver_roles, is_system, version FROM platform.role
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) async fn scopes_of(tx: &mut TxCx, role_id: &str) -> Result<Vec<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT scope FROM platform.role_scope WHERE tenant_id = $1 AND role_id = $2
         ORDER BY scope",
    )
    .bind(tx.tenant.as_str())
    .bind(role_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) fn not_found(id: &str) -> ToolError {
    ToolError::new(ROLE_NOT_FOUND, format!("There is no role {id}."))
        .detail(json!({"entity": "role", "id": id}))
        .next("list_roles")
}

/// One scope people can put in a role, a menu entry or an API key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Permission {
    /// The scope, for example `tools:write:promotion`.
    pub scope: String,
    /// The API domain it belongs to.
    pub domain: String,
    /// What it lets its holder do.
    pub description: String,
}

/// The API domains, as a word list for inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    /// Products, variants, prices, stock, reviews.
    Catalog,
    /// Categories, display slots, banners.
    Merchandising,
    /// Search and comparison.
    Search,
    /// Carts, orders, payments.
    Sales,
    /// Cancellations, returns, refunds, deliveries.
    Fulfillment,
    /// Promotions, sale prices, coupons, loyalty.
    Promotion,
    /// Members, tiers, addresses, preferences.
    Customer,
    /// Customer messages and campaigns.
    Messaging,
    /// Boards, posts, replies.
    Community,
    /// Policies, change sets, tasks, roles, accounts, API keys.
    Platform,
    /// Sales and customer figures.
    Analytics,
    /// Sellers, contracts, grades.
    Partner,
    /// Outside channels, imports, incoming events.
    Integration,
}

impl Domain {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::Merchandising => "merchandising",
            Self::Search => "search",
            Self::Sales => "sales",
            Self::Fulfillment => "fulfillment",
            Self::Promotion => "promotion",
            Self::Customer => "customer",
            Self::Messaging => "messaging",
            Self::Community => "community",
            Self::Platform => "platform",
            Self::Analytics => "analytics",
            Self::Partner => "partner",
            Self::Integration => "integration",
        }
    }
}

/// What a person-only scope lets a person do, and the domain of the endpoint it opens.
fn person_scope(scope: &str) -> (&'static str, &'static str) {
    match scope {
        "human:approve" => (
            "platform",
            "Approve change sets, from the button on the set",
        ),
        "human:reject" => ("platform", "Reject change sets"),
        "human:undo" => (
            "platform",
            "Undo a live change set, or a remembered preference",
        ),
        "human:task" => ("platform", "Put tasks away from the task list"),
        "human:order" => ("sales", "Place orders and start subscriptions"),
        "human:cancel" => ("fulfillment", "Cancel orders"),
        "human:return" => ("fulfillment", "Send return requests"),
        "human:exchange" => ("fulfillment", "Send exchange requests"),
        "human:claim" => (
            "fulfillment",
            "Pay a claim's extra charge or withdraw a claim",
        ),
        "human:account" => (
            "customer",
            "Change a password, recover a login id, close the account",
        ),
        _ => ("platform", "A person-only action"),
    }
}

fn outside_scope(scope: &str) -> &'static str {
    match scope {
        "external:read" => "An outside agent reads products, prices and the store's rules",
        "external:cart" => "An outside agent keeps a shopper's cart",
        "external:prepare_order" => "An outside agent prepares an order for the shopper to place",
        _ => "An outside agent's scope",
    }
}

/// Every scope a role, menu entry or key can name: the tool scopes of every domain, the
/// person-only scopes, change-set drafting, outside agents' scopes (`api_keys.
/// external_agent_scopes`) and the demo controls, limited to the prefixes the store knows
/// (`roles.known_scopes`).
pub(crate) fn catalog(cx: &Cx) -> Result<Vec<Permission>, ToolError> {
    let rules = RoleRules::from_policy(&cx.policy)?;
    let outside = ApiKeyRules::from_policy(&cx.policy)?.external_agent_scopes;
    let mut out = Vec::new();
    let mut add = |scope: String, domain: &str, description: String| {
        if rules.knows_prefix(&scope) {
            out.push(Permission {
                scope,
                domain: domain.to_owned(),
                description,
            });
        }
    };
    for d in acom_auth::scopes::DOMAINS {
        add(
            format!("tools:read:{d}"),
            d,
            format!("Read {d} with its tools"),
        );
        add(
            format!("tools:write:{d}"),
            d,
            format!("Preview {d} writes and run the ones that need no approval"),
        );
    }
    add(
        "changeset:draft".into(),
        "platform",
        "Draft change sets through the agent: create, add items, submit".into(),
    );
    for h in acom_auth::scopes::HUMAN {
        let (domain, description) = person_scope(h);
        add(
            (*h).to_owned(),
            domain,
            format!("{description} (only on the person's own click)"),
        );
    }
    for e in &outside {
        add(e.clone(), "sales", outside_scope(e).to_owned());
    }
    add(
        "admin:demo".into(),
        "platform",
        "Move the demo clock and reload the demo store (demo mode only)".into(),
    );
    Ok(out)
}

/// `roles.known_scopes`: every scope must be one of the catalog's; `invalid_scope` names the
/// first that is not.
pub(crate) fn known_scopes(cx: &Cx, scopes: &[String]) -> Result<Check, ToolError> {
    let known: BTreeSet<String> = catalog(cx)?.into_iter().map(|p| p.scope).collect();
    let unknown: Vec<&String> = scopes.iter().filter(|s| !known.contains(*s)).collect();
    if let Some(first) = unknown.first() {
        return Err(ToolError::new(
            INVALID_SCOPE,
            format!("`{first}` is not a scope this store knows; pick from list_permissions."),
        )
        .detail(
            json!({"field": "scopes", "reason": "enum", "rule": "roles.known_scopes",
                       "items": unknown}),
        )
        .next("list_permissions"));
    }
    Ok(Check {
        rule: "roles.known_scopes".into(),
        passed: true,
        detail: Some(format!(
            "All {} scope{} are known",
            scopes.len(),
            if scopes.len() == 1 { "" } else { "s" }
        )),
        fix_tool: None,
    })
}

/// Scopes in a stable order without repeats.
pub(crate) fn tidy(scopes: &[String]) -> Vec<String> {
    let set: BTreeSet<String> = scopes
        .iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    set.into_iter().collect()
}

/// The person-only rights among some scopes, as words (`approve`, `undo`).
pub(crate) fn person_rights(scopes: &[String]) -> Vec<String> {
    scopes
        .iter()
        .filter_map(|s| s.strip_prefix("human:"))
        .map(str::to_owned)
        .collect()
}
