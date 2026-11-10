//! `list_shopper_personas` (`platform.customer_bot.list_personas`): the personas Ailism Customer
//! Bot shops as, the most recently used first.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{PERSONA_COLUMNS, PersonaRow, db, persona_json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListShopperPersonasInput {
    /// Only this member's persona.
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_m000042")]
    pub customer_id: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListShopperPersonas;

impl Operation for ListShopperPersonas {
    const META: OpMeta = OpMeta {
        address: "platform.customer_bot.list_personas",
        tool: "list_shopper_personas",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/platform/customer-bot/personas",
        summary: "The personas Ailism Customer Bot shops as, one per member: style, sizes, budget, habits and voice, and how often each has shopped.",
        consumes: &[],
        produces: &[Slot::many("customer_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListShopperPersonasInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListShopperPersonas {
    async fn read(&self, cx: &Cx, input: ListShopperPersonasInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let filter = "tenant_id = $1 AND ($2::text IS NULL OR customer_id = $2)";
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM platform.shopper_persona WHERE {filter}"
        )))
        .bind(cx.tenant.as_str())
        .bind(input.customer_id.as_deref())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let rows: Vec<PersonaRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {PERSONA_COLUMNS} FROM platform.shopper_persona WHERE {filter}
             ORDER BY coalesce(last_session_at, created_at) DESC, id DESC LIMIT $3 OFFSET $4"
        )))
        .bind(cx.tenant.as_str())
        .bind(input.customer_id.as_deref())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|p| persona_json(&cx.time, p)).collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
