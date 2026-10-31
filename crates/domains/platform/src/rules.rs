//! The platform rules the access operations apply, read from the store's policies
//! (`policies/platform.yml` plus overrides, and `customer.yml` for the sign-in lockout). Nothing
//! here is a constant: a missing or malformed key is an error, so a broken policy file shows up
//! at the first call instead of as a silent default.

use acom_contract::{ErrorCode, ToolError};
use acom_policy::PolicySet;
use serde::Deserialize;
use serde::de::DeserializeOwned;

fn section<T: DeserializeOwned>(p: &PolicySet, path: &str) -> Result<T, ToolError> {
    let v = p.lookup(path).ok_or_else(|| missing(path))?;
    serde_json::from_value(v.clone()).map_err(|e| {
        ToolError::new(
            ErrorCode::INTERNAL,
            format!("The store policy `{path}` is malformed: {e}"),
        )
    })
}

fn missing(path: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INTERNAL,
        format!("The store policy `{path}` is missing."),
    )
}

/// How `roles.least_privilege` treats person-only scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeastPrivilege {
    /// A change that gives `human:*` scopes says so in its approval reason.
    WarnOnHumanScopes,
}

/// `platform.roles`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRules {
    /// `roles.known_scopes`: the scope prefixes (`tools`, `human`, …) a role, menu or key may use.
    pub known_scopes: Vec<String>,
    /// `roles.known_roles`: the approver roles (`approval.yml` `approver_role`) a role may carry.
    pub known_roles: Vec<String>,
    /// `roles.least_privilege`.
    pub least_privilege: LeastPrivilege,
}

impl RoleRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        section(p, "platform.roles")
    }

    /// Whether a scope's prefix (`tools` of `tools:read:sales`) is one the store knows.
    pub fn knows_prefix(&self, scope: &str) -> bool {
        let prefix = scope.split(':').next().unwrap_or_default();
        self.known_scopes.iter().any(|k| k == prefix)
    }
}

/// `platform.admin_users`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminUserRules {
    /// `admin_users.email_unique`: no two accounts share an email.
    pub email_unique: bool,
}

impl AdminUserRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        section(p, "platform.admin_users")
    }
}

/// `platform.orgs`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrgRules {
    /// `orgs.no_cycle`: an organization is never its own ancestor.
    pub no_cycle: bool,
}

impl OrgRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        section(p, "platform.orgs")
    }
}

/// `platform.api_keys`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyRules {
    /// `api_keys.scope_within_issuer`: a key gets no scope its issuer's roles lack.
    pub scope_within_issuer: bool,
    /// `api_keys.external_agent_scopes`: what an outside agent's key may carry.
    pub external_agent_scopes: Vec<String>,
}

impl ApiKeyRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        section(p, "platform.api_keys")
    }
}

/// The lockout rule a client secret follows (`customer.yml` `password.lockout`).
#[derive(Debug, Clone, Deserialize)]
pub struct Lockout {
    /// Wrong secrets in one window that lock the key.
    pub failed_attempts: i64,
    /// The window the wrong secrets are counted in.
    pub window_minutes: i64,
    /// How long the key stays locked.
    pub lock_minutes: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientSessionsPolicy {
    access_token_minutes: i64,
    refresh_token_days: i64,
    max_sessions_per_client: i64,
    lockout_from: String,
}

/// `platform.client_sessions`: sessions of outside agents and integration clients.
#[derive(Debug, Clone)]
pub struct ClientSessionRules {
    /// Lifetime of an access token (`expires_in`).
    pub access_token_minutes: i64,
    /// Lifetime of a refresh token; every refresh hands out a new one.
    pub refresh_token_days: i64,
    /// Live sessions one client may hold; a new one beyond it ends the oldest.
    pub max_sessions_per_client: i64,
    /// Wrong secrets lock the key, as wrong passwords lock an account.
    pub lockout: Lockout,
}

impl ClientSessionRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let raw: ClientSessionsPolicy = section(p, "platform.client_sessions")?;
        let lockout = section(p, &raw.lockout_from)?;
        Ok(Self {
            access_token_minutes: raw.access_token_minutes,
            refresh_token_days: raw.refresh_token_days,
            max_sessions_per_client: raw.max_sessions_per_client,
            lockout,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The default policy files, as a new store has them.
    pub(crate) fn defaults() -> PolicySet {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
        let files: Vec<(&str, String)> = acom_policy::FILES
            .iter()
            .map(|f| {
                (
                    *f,
                    std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
                )
            })
            .collect();
        PolicySet::from_defaults(files.iter().map(|(f, t)| (*f, t.as_str()))).unwrap()
    }

    #[test]
    fn the_access_rules_come_from_the_policy_files() {
        let p = defaults();
        let roles = RoleRules::from_policy(&p).unwrap();
        assert!(roles.knows_prefix("tools:read:sales") && roles.knows_prefix("human:approve"));
        assert!(!roles.knows_prefix("root:all"));
        assert!(roles.known_roles.iter().any(|r| r == "platform_admin"));
        assert_eq!(roles.least_privilege, LeastPrivilege::WarnOnHumanScopes);
        assert!(AdminUserRules::from_policy(&p).unwrap().email_unique);
        assert!(OrgRules::from_policy(&p).unwrap().no_cycle);
        let keys = ApiKeyRules::from_policy(&p).unwrap();
        assert!(keys.scope_within_issuer);
        assert!(
            keys.external_agent_scopes
                .iter()
                .any(|s| s == "external:cart")
        );
        let sessions = ClientSessionRules::from_policy(&p).unwrap();
        assert_eq!(sessions.access_token_minutes, 15);
        assert_eq!(sessions.lockout.failed_attempts, 5);
    }
}
