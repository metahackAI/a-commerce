//! `get_my_profile`, `get_return_history`, `search_members`, `get_member`.

use acom_contract::{ActionType, ToolError};
use acom_core::enums::ActionStatus;
use acom_store::Cx;
use serde_json::{Value, json};

use crate::{ACCESS_REASON_REQUIRED, CUSTOMER_NOT_FOUND, UNMASK_FORBIDDEN, repo};

mod get_member;
mod get_my_profile;
mod get_return_history;
mod search_members;

pub use get_member::*;
pub use get_my_profile::*;
pub use get_return_history::*;
pub use search_members::*;

pub(crate) fn customer_not_found(id: &str) -> ToolError {
    ToolError::new(CUSTOMER_NOT_FOUND, format!("There is no customer {id}."))
        .detail(json!({"entity": "customer", "id": id}))
        .next("search_members")
}

/// A saved address as profiles and lists show it.
pub(crate) fn address_json(a: &repo::AddressRow) -> Value {
    json!({"id": a.id, "label": a.label, "street_address": a.street_address,
           "address_locality": a.address_locality, "address_region": a.address_region,
           "postal_code": a.postal_code, "address_country": a.address_country,
           "is_primary": a.is_primary})
}

/// A memory as the profile shows it.
fn memory_json(p: &repo::PreferenceRow) -> Value {
    json!({"memory_id": p.id, "key": p.preference_key, "value": p.value,
           "text": p.description, "source": p.source})
}

/// Checks an unmask request and writes it to the action log (privacy.access_reason_logging).
pub(crate) async fn record_unmask(
    cx: &Cx,
    tool: &'static str,
    unmask: bool,
    reason: Option<&str>,
    object: Option<&str>,
) -> Result<bool, ToolError> {
    if !unmask {
        return Ok(false);
    }
    let reason = reason
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| {
            ToolError::new(
                ACCESS_REASON_REQUIRED,
                "Say why you need the unmasked data; the reason goes to the action log.",
            )
            .detail(json!({"field": "access_reason", "reason": "missing"}))
            .next(tool)
        })?;
    use acom_core::enums::ActorKind;
    if !matches!(
        cx.principal.kind,
        ActorKind::Operator | ActorKind::AgentOnBehalf
    ) {
        return Err(ToolError::new(
            UNMASK_FORBIDDEN,
            "Only an operator may unmask customer data.",
        )
        .detail(json!({"reason": "role"}))
        .next("list_roles"));
    }
    let mut tx = cx.begin().await.map_err(ToolError::from)?;
    acom_changeset::log_action(
        &mut tx,
        acom_changeset::LogEntry {
            object: object.map(|id| ("person", id)),
            result: Some(json!({"unmasked": true, "access_reason": reason})),
            ..acom_changeset::LogEntry::new(ActionType::SEARCH.0, ActionStatus::Completed, tool)
        },
    )
    .await?;
    tx.commit().await.map_err(ToolError::from)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reads_people_from_the_database() {
        let mut tx = crate::testdb::tx("cus_jamie").await;
        let jamie = repo::person(&mut tx, "cus_jamie").await.unwrap().unwrap();
        assert_eq!(jamie.card_last4.as_deref(), Some("4821"));
        let (total, rows) = repo::search_members(
            &mut tx,
            &repo::MemberFilter {
                query: Some("alex".into()),
                ..Default::default()
            },
            20,
            0,
        )
        .await
        .unwrap();
        assert!(total >= 1);
        assert!(rows.iter().any(|r| r.id == "cus_alex"));
    }
}
