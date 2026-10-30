//! `search_action_log`, `get_action_log`: the append-only record of every action, as schema.org
//! `Action`s.

use acom_contract::{
    ActionType, Audience, ErrorClass, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp,
    Slot, ToolError,
};
use acom_core::enums::ActionStatus;
use acom_store::{Cx, Page, StoreError};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::parse_time;

pub const ACTION_NOT_FOUND: ErrorCode = ErrorCode::new("action_not_found", ErrorClass::NotFound);

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: String,
    action_type: String,
    action_status: String,
    actor_kind: String,
    agent_id: Option<String>,
    agent_name: Option<String>,
    instrument: String,
    object_type: Option<String>,
    object_id: Option<String>,
    result: Option<Value>,
    error: Option<Value>,
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    change_set_id: Option<String>,
    preview_id: Option<String>,
    policy_version: Option<String>,
    request_id: Option<String>,
    usage: Option<Value>,
    latency_ms: Option<i32>,
}

const COLUMNS: &str = "id, action_type, action_status, actor_kind, agent_id, agent_name,
    instrument, object_type, object_id, result, error, start_time, end_time, change_set_id,
    preview_id, policy_version, request_id, usage, latency_ms";

/// The schema.org type of an object type word (`unit_price_specification` → `UnitPriceSpecification`).
fn type_name(word: &str) -> String {
    word.split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect()
}

fn action(cx: &Cx, r: &Row) -> Value {
    let agent = match r.actor_kind.as_str() {
        "system" => {
            json!({"@type": "SoftwareApplication", "id": r.agent_id, "name": r.agent_name.as_deref().unwrap_or("system")})
        }
        "agent_on_behalf" | "external_agent" => json!({"@type": "SoftwareApplication",
            "name": r.agent_name, "acom:on_behalf_of": r.agent_id}),
        _ => json!({"@type": "Person", "id": r.agent_id, "name": r.agent_name}),
    };
    let mut out = Map::new();
    out.insert("@type".into(), json!(r.action_type));
    out.insert("id".into(), json!(r.id));
    out.insert("action_status".into(), json!(r.action_status));
    out.insert("agent".into(), agent);
    out.insert("instrument".into(), json!(r.instrument));
    if let (Some(t), Some(id)) = (&r.object_type, &r.object_id) {
        out.insert("object".into(), json!({"@type": type_name(t), "id": id}));
    }
    if let Some(v) = &r.result {
        out.insert("result".into(), v.clone());
    }
    if let Some(e) = &r.error {
        out.insert("error".into(), e.clone());
    }
    out.insert("start_time".into(), json!(cx.time.format(r.start_time)));
    out.insert(
        "end_time".into(),
        json!(r.end_time.map(|t| cx.time.format(t))),
    );
    out.insert("acom:change_set_id".into(), json!(r.change_set_id));
    out.insert("acom:preview_id".into(), json!(r.preview_id));
    out.insert("acom:request_id".into(), json!(r.request_id));
    if let Some(u) = &r.usage {
        out.insert("acom:tokens".into(), u.clone());
    }
    if let Some(ms) = r.latency_ms {
        out.insert("acom:latency_ms".into(), json!(ms));
    }
    Value::Object(out)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchActionLogInput {
    /// Who acted: a model name, or a person or system id.
    #[schemars(example = &"adm_sj")]
    pub agent: Option<String>,
    /// The tool used.
    #[schemars(example = &"set_sale_prices")]
    pub instrument: Option<String>,
    /// The record acted on.
    #[schemars(example = &"prd_windbreaker")]
    pub object_id: Option<String>,
    /// The change set the action belongs to.
    #[schemars(regex(pattern = r"^cs_[a-z0-9_]+$"), example = &"cs_1042")]
    pub change_set_id: Option<String>,
    /// Only actions in this state.
    #[schemars(example = &"completed")]
    pub action_status: Option<ActionStatus>,
    /// Started at or after this time (store time).
    #[schemars(example = &"2026-11-06T00:00")]
    pub start_time_from: Option<String>,
    /// Started at or before this time (store time).
    #[schemars(example = &"2026-11-06T23:59")]
    pub start_time_through: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchActionLog;

impl Operation for SearchActionLog {
    const META: OpMeta = OpMeta {
        address: "platform.action_log.search",
        tool: "search_action_log",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/action-logs",
        summary: "Find actions in the log by who, which tool, which record, which change set, state and time. For audits, disputes, and usage numbers.",
        consumes: &[],
        produces: &[Slot::many("action_id")],
        checks: &[],
        errors: &[ErrorCode::FORBIDDEN],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchActionLogInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchActionLog {
    async fn read(&self, cx: &Cx, input: SearchActionLogInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let from = parse_time(cx, "start_time_from", input.start_time_from.as_deref())?;
        let through = parse_time(
            cx,
            "start_time_through",
            input.start_time_through.as_deref(),
        )?;
        let filter = "tenant_id = $1
            AND ($2::text IS NULL OR agent_id = $2 OR agent_name = $2)
            AND ($3::text IS NULL OR instrument = $3)
            AND ($4::text IS NULL OR object_id = $4)
            AND ($5::text IS NULL OR change_set_id = $5)
            AND ($6::text IS NULL OR action_status = $6)
            AND ($7::timestamptz IS NULL OR start_time >= $7)
            AND ($8::timestamptz IS NULL OR start_time <= $8)";
        let status = input.action_status.map(|s| s.as_str());
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM platform.action_log WHERE {filter}"
        )))
        .bind(cx.tenant.as_str())
        .bind(input.agent.as_deref())
        .bind(input.instrument.as_deref())
        .bind(input.object_id.as_deref())
        .bind(input.change_set_id.as_deref())
        .bind(status)
        .bind(from)
        .bind(through)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM platform.action_log WHERE {filter}
             ORDER BY start_time DESC, id DESC LIMIT $9 OFFSET $10"
        )))
        .bind(cx.tenant.as_str())
        .bind(input.agent.as_deref())
        .bind(input.instrument.as_deref())
        .bind(input.object_id.as_deref())
        .bind(input.change_set_id.as_deref())
        .bind(status)
        .bind(from)
        .bind(through)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|r| action(cx, r)).collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetActionLogInput {
    /// The action.
    #[schemars(regex(pattern = r"^act_[a-z0-9_]+$"), example = &"act_1042_03")]
    pub action_id: String,
}

pub struct GetActionLog;

impl Operation for GetActionLog {
    const META: OpMeta = OpMeta {
        address: "platform.action_log.get",
        tool: "get_action_log",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/action-logs/{action_id}",
        summary: "One action in full: the preview's changes and checks, who approved it, and the policy version it ran under.",
        consumes: &[Slot::one("action_id")],
        produces: &[],
        checks: &[],
        errors: &[ACTION_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetActionLogInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetActionLog {
    async fn read(&self, cx: &Cx, input: GetActionLogInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = sqlx::query_as::<_, Row>(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM platform.action_log WHERE tenant_id = $1 AND id = $2"
        )))
        .bind(cx.tenant.as_str())
        .bind(&input.action_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?
        .ok_or_else(|| {
            ToolError::new(
                ACTION_NOT_FOUND,
                format!("There is no action {}.", input.action_id),
            )
            .next("search_action_log")
        })?;
        let preview: Option<(Value, Value)> = match &r.preview_id {
            Some(p) => sqlx::query_as(
                "SELECT changes, checks FROM platform.preview WHERE tenant_id = $1 AND id = $2",
            )
            .bind(cx.tenant.as_str())
            .bind(p)
            .fetch_optional(&mut *tx.tx)
            .await
            .map_err(db)?,
            None => None,
        };
        let approved_by: Option<String> = match &r.change_set_id {
            Some(cs) => sqlx::query_scalar(
                "SELECT approved_by FROM platform.change_set WHERE tenant_id = $1 AND id = $2",
            )
            .bind(cx.tenant.as_str())
            .bind(cs)
            .fetch_optional(&mut *tx.tx)
            .await
            .map_err(db)?
            .flatten(),
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;
        let mut out = action(cx, &r);
        out["acom:policy_version"] = json!(r.policy_version);
        out["acom:approved_by"] = json!(approved_by);
        if let Some((changes, checks)) = preview {
            out["acom:changes"] = changes;
            out["acom:checks"] = checks;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn names_object_types_like_schema_org() {
        assert_eq!(
            super::type_name("unit_price_specification"),
            "UnitPriceSpecification"
        );
        assert_eq!(super::type_name("change_set"), "ChangeSet");
    }
}
