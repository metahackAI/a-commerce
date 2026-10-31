//! `logout` (`platform.auth.logout`): an API client ends its current session, or all of them.
//! Refresh tokens stop working at once; an access token lives out its short lifetime. People sign
//! out at the gateway (`/api/auth/logout`); this tool never ends a browser session.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, ErrorCode, Group, Kind, Method, OpMeta,
    Operation, Planned, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::ActorKind;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{end_sessions, live_sessions};
use crate::api_key::ALREADY_REVOKED;
use crate::repo::db;

/// Which sessions end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogoutScope {
    /// Only the session this request comes from.
    Current,
    /// Every session of this client.
    AllSessions,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogoutInput {
    /// current: this session only (the default); all_sessions: every session of this client.
    #[schemars(example = &"current")]
    pub scope: Option<LogoutScope>,
}

/// What `logout` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogoutPlan {
    pub client_id: String,
    /// The session the request came from.
    pub session_id: String,
    pub sessions: Vec<String>,
}

/// Sign an API client out.
pub struct Logout;

impl Operation for Logout {
    const META: OpMeta = OpMeta {
        address: "platform.auth.logout",
        tool: "logout",
        group: Group::Operate,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/platform/auth/logout",
        summary: "End an API client's current session, or all of its sessions: their refresh tokens stop working at once. People sign out at the gateway, never here.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[ALREADY_REVOKED],
        audience: Audience::Shopper,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = LogoutInput;
    type Output = Value;
}

impl WriteOp<Cx> for Logout {
    type Plan = LogoutPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(&self, cx: &Cx, input: LogoutInput) -> Result<Planned<LogoutPlan>, ToolError> {
        let p = &cx.principal;
        let session_id = match (&p.session_id, p.kind) {
            (Some(s), ActorKind::ExternalAgent) => s.clone(),
            _ => {
                return Err(ToolError::new(
                    ErrorCode::FORBIDDEN,
                    "Only an API client signs out with this tool; people sign out from the page.",
                )
                .detail(json!({"reason": "owner"})));
            }
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mine: Option<(Option<String>, Option<DateTime<Utc>>)> = sqlx::query_as(
            "SELECT subject_id, revoked_at FROM platform.session
             WHERE tenant_id = $1 AND id = $2 AND actor_kind = 'external_agent'",
        )
        .bind(cx.tenant.as_str())
        .bind(&session_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let live = live_sessions(&mut tx, &p.subject).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if mine
            .as_ref()
            .is_some_and(|(subject, _)| subject.as_deref() != Some(p.subject.as_str()))
        {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                "This session belongs to another client.",
            )
            .detail(json!({"reason": "owner"})));
        }
        let sessions: Vec<String> = match input.scope.unwrap_or(LogoutScope::Current) {
            LogoutScope::Current => live.into_iter().filter(|s| *s == session_id).collect(),
            LogoutScope::AllSessions => live,
        };
        if sessions.is_empty() {
            return Err(ToolError::new(
                ALREADY_REVOKED,
                "There is no live session to end; sign in again to get one.",
            )
            .detail(json!({"entity": "session", "id": session_id, "from": "revoked"}))
            .next("login"));
        }
        Ok(Planned {
            changes: sessions
                .iter()
                .map(|s| Change {
                    op: ChangeOp::Revoke,
                    entity: "session".into(),
                    id: s.clone(),
                    from: Value::Null,
                    to: Value::Null,
                    note: None,
                })
                .collect(),
            checks: Vec::new(),
            basis: Vec::new(),
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan: LogoutPlan {
                client_id: p.subject.clone(),
                session_id,
                sessions,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &LogoutPlan) -> Result<Committed, ToolError> {
        // Calling it again ends nothing more and answers the same.
        let now = tx.clock.now();
        end_sessions(tx, &plan.sessions, now).await?;
        let mut produced = Map::new();
        produced.insert("sessions_ended".into(), json!(plan.sessions.len()));
        Ok(Committed {
            id: plan.session_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &LogoutPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let still: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM platform.session
             WHERE tenant_id = $1 AND id = ANY($2) AND revoked_at IS NULL",
        )
        .bind(cx.tenant.as_str())
        .bind(&plan.sessions)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: still.is_empty(),
            mismatches: still.iter().map(|s| format!("{s} is still live")).collect(),
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &LogoutPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} signed out; sign in again for a new session.",
                plan.client_id
            ),
        )
        .next("login"))
    }
}
