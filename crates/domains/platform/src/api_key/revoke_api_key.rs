//! `revoke_api_key` (`platform.api_key.revoke`): stop a key for good. Its sessions end at once.
//! It cannot be undone (issue a new key instead), and a person always approves it.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{ALREADY_REVOKED, API_KEY_NOT_FOUND, key, not_found};
use crate::repo::{db, end_client_sessions};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevokeApiKeyInput {
    /// The key.
    #[schemars(regex(pattern = r"^key_[a-z0-9_]+$"), example = &"key_31")]
    pub api_key_id: String,
    /// Why, in a sentence; kept with the key.
    #[schemars(length(min = 1, max = 500), example = &"The agent vendor rotated its credentials.")]
    pub reason: String,
}

/// What `revoke_api_key` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeApiKeyPlan {
    pub api_key_id: String,
    pub reason: String,
}

/// Revoke an API key.
pub struct RevokeApiKey;

impl Operation for RevokeApiKey {
    const META: OpMeta = OpMeta {
        address: "platform.api_key.revoke",
        tool: "revoke_api_key",
        group: Group::Operate,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/platform/api-keys/{api_key_id}/revoke",
        summary: "Revoke an API key: it stops working at once and its sessions end. Cannot be undone (issue a new key). A platform administrator always approves it.",
        consumes: &[Slot::one("api_key_id")],
        produces: &[Slot::one("api_key_id")],
        checks: &[],
        errors: &[ALREADY_REVOKED, API_KEY_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = RevokeApiKeyInput;
    type Output = Value;
}

impl WriteOp<Cx> for RevokeApiKey {
    type Plan = RevokeApiKeyPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RevokeApiKeyInput,
    ) -> Result<Planned<RevokeApiKeyPlan>, ToolError> {
        crate::require_admin(cx, Some("list_roles"))?;
        let reason = crate::required_text("reason", &input.reason)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let k = key(&mut tx, &input.api_key_id)
            .await?
            .ok_or_else(|| not_found(&input.api_key_id))?;
        let live: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM platform.session
             WHERE tenant_id = $1 AND subject_id = $2 AND actor_kind = 'external_agent'
               AND revoked_at IS NULL",
        )
        .bind(cx.tenant.as_str())
        .bind(&k.id)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        if k.api_key_status == "revoked" {
            return Err(ToolError::new(
                ALREADY_REVOKED,
                format!("{} ({}) is already revoked.", k.id, k.name),
            )
            .detail(json!({"entity": "api_key", "id": k.id, "from": "revoked"}))
            .next("list_api_keys"));
        }
        let last_used = k.last_used_at.map_or_else(
            || "never used".to_owned(),
            |t| format!("last used {}", cx.time.format(t)),
        );
        let plan = RevokeApiKeyPlan {
            api_key_id: k.id.clone(),
            reason,
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "api_key".into(),
                id: k.id.clone(),
                from: json!({"status": "active", "name": k.name, "key_prefix": k.key_prefix}),
                to: json!({"status": "revoked", "sessions_ended": live}),
                note: Some(format!("{}; {}", last_used, plan.reason)),
            }],
            checks: Vec::new(),
            basis: vec![Basis {
                entity: "platform.api_key".into(),
                id: k.id,
                version: i64::from(k.version),
            }],
            approval: ApprovalNeed::Needed {
                reason: format!(
                    "Stops the key {} ({last_used}) for good; it cannot be undone",
                    k.name
                ),
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &RevokeApiKeyPlan) -> Result<Committed, ToolError> {
        let now = tx.clock.now();
        // A key already revoked stays as it is: the same result as the first time.
        sqlx::query(
            "UPDATE platform.api_key SET api_key_status = 'revoked', revoked_at = $3,
               revocation_reason = $4, updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2 AND api_key_status = 'active'",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.api_key_id)
        .bind(now)
        .bind(&plan.reason)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        let ended = end_client_sessions(tx, &plan.api_key_id, now).await?;
        let mut produced = Map::new();
        produced.insert("api_key_id".into(), json!(plan.api_key_id));
        produced.insert("sessions_ended".into(), json!(ended.len()));
        Ok(Committed {
            id: plan.api_key_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &RevokeApiKeyPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let k = key(&mut tx, &plan.api_key_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let status = k.map(|k| k.api_key_status);
        let matched = status.as_deref() == Some("revoked");
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} is {}",
                    plan.api_key_id,
                    status.unwrap_or_else(|| "missing".into())
                )]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &RevokeApiKeyPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "A revoked key stays revoked; issue a new key in place of {}.",
                plan.api_key_id
            ),
        )
        .next("issue_api_key"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(v: Value) -> RevokeApiKeyInput {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database"]
    async fn revokes_with_the_last_use_shown_and_ends_sessions() {
        let cx = crate::testdb::admin().await;
        let e = RevokeApiKey
            .preview(&cx, input(json!({"api_key_id": "key_27", "reason": "x"})))
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("already_revoked", Some("list_api_keys"))
        );
        let p = RevokeApiKey
            .preview(
                &cx,
                input(json!({"api_key_id": "key_31", "reason": "Rotated by the vendor."})),
            )
            .await
            .unwrap();
        assert!(p.changes[0].note.as_deref().unwrap().contains("last used"));
        assert_eq!(RevokeApiKey::UNDO, UndoSupport::NotSupported);
        // Rolled back: the seeded key stays active.
        let mut tx = cx.begin().await.unwrap();
        let done = RevokeApiKey.commit(&mut tx, &p.plan).await.unwrap();
        assert_eq!(done.id, "key_31");
        let status: String = sqlx::query_scalar(
            "SELECT api_key_status FROM platform.api_key WHERE tenant_id = $1 AND id = $2",
        )
        .bind("store_northwind")
        .bind("key_31")
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        assert_eq!(status, "revoked");
        drop(tx);
    }
}
