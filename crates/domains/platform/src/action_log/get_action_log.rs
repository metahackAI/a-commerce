//! `get_action_log` (`platform.action_log.get`): one action in full.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ACTION_NOT_FOUND, COLUMNS, Row, action, db};

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
