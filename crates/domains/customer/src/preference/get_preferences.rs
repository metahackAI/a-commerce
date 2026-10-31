//! `get_preferences`.

use crate::CUSTOMER_NOT_FOUND;
use crate::access::customer_for;
use crate::person::customer_not_found;
use crate::repo::{self};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::PreferenceKey;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPreferencesInput {
    /// The member; a shopper uses `me`.
    #[schemars(regex(pattern = r"^(me|cus_[a-z0-9_]+)$"), example = &"me")]
    pub customer_id: String,
    /// Only memories about this.
    #[schemars(example = &"travel_priority")]
    pub key: Option<PreferenceKey>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct GetPreferences;

impl Operation for GetPreferences {
    const META: OpMeta = OpMeta {
        address: "customer.preference.get",
        tool: "get_preferences",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/people/{customer_id}/preferences",
        summary: "Everything the agent remembers about a member, newest first, each as the line the shopper sees.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::many("memory_id")],
        checks: &[],
        errors: &[CUSTOMER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPreferencesInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetPreferences {
    async fn read(&self, cx: &Cx, input: GetPreferencesInput) -> Result<Value, ToolError> {
        let customer_id = customer_for(cx, &input.customer_id)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if repo::person(&mut tx, &customer_id).await?.is_none() {
            return Err(customer_not_found(&customer_id));
        }
        let key = input.key.map(|k| k.as_str());
        let (total, rows) =
            repo::preferences(&mut tx, &customer_id, key, page.limit, page.offset).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|p| {
                json!({"id": p.id, "key": p.preference_key, "value": p.value,
                       "text": p.description, "source": p.source,
                       "created_at": cx.time.format(p.learned_at)})
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<GetPreferencesInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }
}
