//! `get_customer_bot_session` (`platform.customer_bot.get_session`): one shopping session whole:
//! the persona it shopped as and every step it took, in order.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    CUSTOMER_BOT_SESSION_NOT_FOUND, PERSONA_COLUMNS, PersonaRow, SESSION_SELECT, SessionRow, db,
    not_found, persona_json, session_json,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCustomerBotSessionInput {
    /// The session.
    #[schemars(regex(pattern = r"^cbs_[a-z0-9_]+$"), example = &"cbs_01jcb1")]
    pub customer_bot_session_id: String,
}

pub struct GetCustomerBotSession;

impl Operation for GetCustomerBotSession {
    const META: OpMeta = OpMeta {
        address: "platform.customer_bot.get_session",
        tool: "get_customer_bot_session",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/customer-bot/sessions/{customer_bot_session_id}",
        summary: "One Ailism Customer Bot session whole: the member and persona it shopped as, and every step it took in order (sign-in, chat turns with the replies, buttons pressed, reviews).",
        consumes: &[Slot::one("customer_bot_session_id")],
        produces: &[Slot::one("customer_id"), Slot::many("order_id")],
        checks: &[],
        errors: &[CUSTOMER_BOT_SESSION_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCustomerBotSessionInput;
    type Output = Value;
}

#[derive(Debug, sqlx::FromRow)]
struct StepRow {
    position: i32,
    step_kind: String,
    step_status: String,
    title: String,
    request: Option<Value>,
    response: Option<Value>,
    detail: Value,
    duration_ms: i32,
    start_time: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl ReadOp<Cx> for GetCustomerBotSession {
    async fn read(&self, cx: &Cx, input: GetCustomerBotSessionInput) -> Result<Value, ToolError> {
        let id = input.customer_bot_session_id;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let session: Option<SessionRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "{SESSION_SELECT} WHERE s.tenant_id = $1 AND s.id = $2"
        )))
        .bind(cx.tenant.as_str())
        .bind(&id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        let Some(session) = session else {
            return Err(not_found(
                CUSTOMER_BOT_SESSION_NOT_FOUND,
                "customer_bot_session",
                &id,
                "list_customer_bot_sessions",
            ));
        };
        let steps: Vec<StepRow> = sqlx::query_as(
            "SELECT position, step_kind, step_status, title, request, response, detail,
                    duration_ms, start_time, created_at
             FROM platform.customer_bot_step WHERE tenant_id = $1 AND session_id = $2
             ORDER BY position",
        )
        .bind(cx.tenant.as_str())
        .bind(&id)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        let persona: Option<PersonaRow> = match &session.customer_id {
            Some(c) => sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {PERSONA_COLUMNS} FROM platform.shopper_persona
                 WHERE tenant_id = $1 AND customer_id = $2"
            )))
            .bind(cx.tenant.as_str())
            .bind(c)
            .fetch_optional(&mut *tx.tx)
            .await
            .map_err(db)?,
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;
        let mut out = session_json(&cx.time, &session);
        out["persona"] = persona
            .as_ref()
            .map_or(Value::Null, |p| persona_json(&cx.time, p));
        out["steps"] = json!(
            steps
                .iter()
                .map(|s| json!({"position": s.position, "kind": s.step_kind,
                                "status": s.step_status, "title": s.title,
                                "request": s.request, "response": s.response,
                                "detail": s.detail, "ms": s.duration_ms,
                                "at": cx.time.format(s.start_time),
                                "recorded": cx.time.format(s.created_at)}))
                .collect::<Vec<_>>()
        );
        Ok(out)
    }
}
