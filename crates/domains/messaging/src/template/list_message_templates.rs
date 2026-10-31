//! `list_message_templates` (docs/api/messaging.md#template): templates by channel, purpose and
//! language.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError, fields,
};
use acom_core::enums::MessageChannel;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::repo;

use super::{INVALID_FILTER, MessagePurpose};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListMessageTemplatesInput {
    /// Only templates for this channel.
    #[schemars(example = &"email")]
    pub channel: Option<MessageChannel>,
    /// Only marketing or only transactional templates.
    #[schemars(example = &"marketing")]
    pub purpose: Option<MessagePurpose>,
    /// Only templates in this language.
    #[schemars(regex(pattern = r"^[a-z]{2,3}(-[A-Z]{2})?$"), example = &"en-US")]
    pub in_language: Option<String>,
    /// Top-level fields to return for each template.
    #[schemars(example = ["id", "name", "channel"])]
    pub fields: Option<Vec<String>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListMessageTemplates;

impl Operation for ListMessageTemplates {
    const META: OpMeta = OpMeta {
        address: "messaging.template.list",
        tool: "list_message_templates",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/messaging/templates",
        summary: "Message templates, filtered by channel, purpose and language.",
        consumes: &[],
        produces: &[Slot::many("template_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListMessageTemplatesInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListMessageTemplates {
    async fn read(&self, cx: &Cx, input: ListMessageTemplatesInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (total, rows) = repo::templates(
            &mut tx,
            input.channel.map(MessageChannel::as_str),
            input.purpose.map(MessagePurpose::as_str),
            input.in_language.as_deref(),
            page.limit,
            page.offset,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let fields = input.fields.unwrap_or_default();
        let items: Vec<Value> = rows
            .iter()
            .map(|t| {
                json!({"id": t.id, "name": t.name, "channel": t.message_channel,
                       "purpose": t.message_purpose, "status": t.template_status,
                       "headline": t.headline, "text": t.text, "variables": t.variables,
                       "in_language": t.in_language,
                       "created_at": cx.time.format(t.created_at)})
            })
            .collect();
        let items = fields::keep_each(items, &fields)?;
        let mut out = json!({"items": items, "total": total});
        if let Some(next) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(next);
        }
        Ok(out)
    }
}
