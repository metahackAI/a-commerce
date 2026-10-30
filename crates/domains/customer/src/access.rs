//! Who is asking, and whose data they may see. A shopper only ever reads and changes their own
//! records: the customer comes from the principal, never from the input.

use acom_auth::Principal;
use acom_contract::{ApprovalNeed, ErrorCode, ToolError};
use acom_store::{Cx, StoreError};
use serde_json::{Value, json};

use crate::NOT_SIGNED_IN;

/// The caller of a customer operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Caller {
    /// A signed-in shopper, or our agent acting for them.
    Shopper(String),
    /// A browser session that has not signed in.
    Guest,
    /// An operator, our agent acting for one, or the system.
    Staff,
}

pub(crate) fn caller(p: &Principal) -> Caller {
    match p.customer_id() {
        Some(id) => Caller::Shopper(id.to_owned()),
        None if p.subject.starts_with("ses_") => Caller::Guest,
        None => Caller::Staff,
    }
}

pub(crate) fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

pub(crate) fn not_signed_in() -> ToolError {
    ToolError::new(
        NOT_SIGNED_IN,
        "Sign in first; guests have no profile, addresses or memories.",
    )
}

/// The signed-in shopper's own id.
pub(crate) fn signed_in_shopper(cx: &Cx) -> Result<String, ToolError> {
    match caller(&cx.principal) {
        Caller::Shopper(id) => Ok(id),
        _ => Err(not_signed_in()),
    }
}

/// The customer an operation is about. `me` is the signed-in shopper; a shopper may name only
/// themselves; staff may name anyone.
pub(crate) fn customer_for(cx: &Cx, requested: &str) -> Result<String, ToolError> {
    match (caller(&cx.principal), requested) {
        (Caller::Shopper(id), "me") => Ok(id),
        (Caller::Shopper(id), other) if other == id => Ok(id),
        (Caller::Shopper(_), _) => Err(ToolError::new(
            ErrorCode::FORBIDDEN,
            "A shopper can only see and change their own account.",
        )
        .detail(json!({"reason": "owner"}))),
        (_, "me") => Err(not_signed_in()),
        (Caller::Guest, _) => Err(ToolError::new(
            ErrorCode::FORBIDDEN,
            "Only the store's operators can look up other customers.",
        )
        .detail(json!({"reason": "role"}))),
        (Caller::Staff, other) => Ok(other.to_owned()),
    }
}

/// Refuses shoppers and guests: the operation is for the store's operators.
pub(crate) fn staff_only(cx: &Cx) -> Result<(), ToolError> {
    match caller(&cx.principal) {
        Caller::Staff => Ok(()),
        _ => Err(ToolError::new(
            ErrorCode::FORBIDDEN,
            "Only the store's operators can look up customers.",
        )
        .detail(json!({"reason": "role"}))),
    }
}

/// The approval policy's answer for a write, as the contract wants it.
pub(crate) fn approval(cx: &Cx, tool: &str, facts: &Value) -> ApprovalNeed {
    let a = cx.policy.approval_for(tool, facts);
    if a.needed {
        ApprovalNeed::Needed {
            reason: a.reason.unwrap_or_else(|| "Needs approval".to_owned()),
        }
    } else {
        ApprovalNeed::NotNeeded
    }
}

/// A policy value by dotted path (`customer.address.max_per_member`).
pub(crate) fn policy<'a>(cx: &'a Cx, path: &str) -> Option<&'a Value> {
    cx.policy.lookup(path)
}

/// A whole-number policy value, or an internal error naming the key when it is missing.
pub(crate) fn policy_number(cx: &Cx, path: &str) -> Result<i64, ToolError> {
    policy(cx, path).and_then(Value::as_i64).ok_or_else(|| {
        ToolError::new(
            ErrorCode::INTERNAL,
            format!("The policy value `{path}` is missing."),
        )
    })
}

/// A list of words from the policy (`customer.preference.keys`).
pub(crate) fn policy_words(cx: &Cx, path: &str) -> Vec<String> {
    policy(cx, path)
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// A time input in store time.
pub(crate) fn parse_time(
    cx: &Cx,
    field: &str,
    s: Option<&str>,
) -> Result<Option<chrono::DateTime<chrono::Utc>>, ToolError> {
    s.map(|s| {
        cx.time.parse(s).map_err(|e| {
            ToolError::invalid_input(e.to_string())
                .detail(json!({"field": field, "reason": "format"}))
        })
    })
    .transpose()
}

/// Keeps only the requested top-level fields of an entity (`fields` input).
pub(crate) fn only_fields(value: Value, fields: &[String]) -> Value {
    if fields.is_empty() {
        return value;
    }
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, _)| fields.iter().any(|f| f == k))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use acom_auth::{Principal, Surface};
    use acom_core::TenantId;
    use acom_core::enums::ActorKind;

    use super::*;

    fn principal(subject: &str, kind: ActorKind) -> Principal {
        Principal {
            subject: subject.into(),
            kind,
            tenant: TenantId::parse("store_northwind").unwrap(),
            surface: Surface::Fo,
            scopes: BTreeSet::new(),
            name: None,
            session_id: None,
        }
    }

    #[test]
    fn tells_shoppers_guests_and_staff_apart() {
        assert_eq!(
            caller(&principal("cus_jamie", ActorKind::Shopper)),
            Caller::Shopper("cus_jamie".into())
        );
        assert_eq!(
            caller(&principal("cus_jamie", ActorKind::AgentOnBehalf)),
            Caller::Shopper("cus_jamie".into())
        );
        assert_eq!(
            caller(&principal("ses_dev_guest", ActorKind::Shopper)),
            Caller::Guest
        );
        assert_eq!(
            caller(&principal("adm_sj", ActorKind::Operator)),
            Caller::Staff
        );
    }

    #[test]
    fn keeps_only_requested_fields() {
        let v = json!({"id": "cus_jamie", "name": "Jamie", "sizes": {}});
        assert_eq!(
            only_fields(v.clone(), &["sizes".into()]),
            json!({"sizes": {}})
        );
        assert_eq!(only_fields(v.clone(), &[]), v);
    }
}
