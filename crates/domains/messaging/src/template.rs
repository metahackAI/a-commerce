//! `create_message_template` and `list_message_templates`: reusable messages per channel.
//! Creating one sends nothing, so it needs no approval.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, ErrorClass, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, ReadOp, Slot, ToolError, UndoSupport,
    Verification, WriteOp,
};
use acom_core::enums::MessageChannel;
use acom_store::{Cx, Page, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::model::{has_subject, undeclared};
use crate::repo::{self, NewTemplate};

acom_core::define_id!(
    /// A message template (`messaging.message_template`).
    TemplateId,
    "tpl"
);

pub const UNKNOWN_VARIABLE: ErrorCode =
    ErrorCode::new("unknown_variable", ErrorClass::InvalidInput);
pub const CHANNEL_NOT_SUPPORTED: ErrorCode =
    ErrorCode::new("channel_not_supported", ErrorClass::InvalidInput);
pub const INVALID_FILTER: ErrorCode = ErrorCode::new("invalid_filter", ErrorClass::InvalidInput);

/// The language a template is written in when none is given (the column's default).
const DEFAULT_LANGUAGE: &str = "en-US";

/// What a template is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessagePurpose {
    /// Promotions: only to shoppers who opted in, held back in quiet hours.
    Marketing,
    /// About an order or a claim.
    Transactional,
}

impl MessagePurpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Marketing => "marketing",
            Self::Transactional => "transactional",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateMessageTemplateInput {
    /// The name operators pick the template by.
    #[schemars(length(min = 1, max = 120), example = &"Back in stock")]
    pub name: String,
    /// email, sms, push or in_app.
    #[schemars(example = &"email")]
    pub channel: MessageChannel,
    /// marketing or transactional.
    #[schemars(example = &"marketing")]
    pub purpose: MessagePurpose,
    /// Subject line, for email and push.
    #[schemars(length(max = 200), example = &"{{product_name}} is back")]
    pub headline: Option<String>,
    /// The body; variables are written {{customer_name}}.
    #[schemars(length(min = 1, max = 2000), example = &"Hi {{customer_name}}, {{product_name}} is back in your size.")]
    pub text: String,
    /// The variable names the headline and text may use.
    #[schemars(inner(regex(pattern = r"^[a-z][a-z0-9_]*$")), example = ["customer_name", "product_name"])]
    pub variables: Option<Vec<String>>,
    /// Language tag of the text. Default en-US.
    #[schemars(regex(pattern = r"^[a-z]{2,3}(-[A-Z]{2})?$"), example = &"en-US")]
    pub in_language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplatePlan {
    pub template_id: String,
    pub name: String,
    pub channel: String,
    pub purpose: String,
    pub headline: Option<String>,
    pub text: String,
    pub variables: Vec<String>,
    pub in_language: String,
}

pub struct CreateMessageTemplate;

impl Operation for CreateMessageTemplate {
    const META: OpMeta = OpMeta {
        address: "messaging.template.create",
        tool: "create_message_template",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/messaging/templates",
        summary: "Create a message template for one channel (subject, body with named variables, language). It sends nothing.",
        consumes: &[],
        produces: &[Slot::one("template_id")],
        checks: &["messaging.template.variables_declared"],
        errors: &[UNKNOWN_VARIABLE, CHANNEL_NOT_SUPPORTED],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateMessageTemplateInput;
    type Output = Value;
}

impl WriteOp<Cx> for CreateMessageTemplate {
    type Plan = TemplatePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateMessageTemplateInput,
    ) -> Result<Planned<TemplatePlan>, ToolError> {
        let name = input.name.trim().to_owned();
        let text = input.text.trim().to_owned();
        if name.is_empty() || text.is_empty() {
            return Err(ToolError::invalid_input("A template needs a name and a text.")
                .detail(json!({"field": if name.is_empty() { "name" } else { "text" }, "reason": "missing"})));
        }
        let channel = input.channel.as_str();
        let headline = input
            .headline
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(str::to_owned);
        if headline.is_some() && !has_subject(channel) {
            return Err(ToolError::new(
                CHANNEL_NOT_SUPPORTED,
                format!(
                    "{} messages have no subject line; leave the headline out.",
                    channel.replace('_', "-")
                ),
            )
            .detail(json!({"field": "headline", "channel": channel}))
            .next("list_enums"));
        }
        let variables = input.variables.unwrap_or_default();
        let enforced = cx
            .policy
            .lookup("messaging.messaging.template.variables_declared")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let mut texts = vec![text.as_str()];
        if let Some(h) = &headline {
            texts.push(h);
        }
        let missing = undeclared(&texts, &variables);
        if enforced && !missing.is_empty() {
            return Err(ToolError::new(
                UNKNOWN_VARIABLE,
                format!(
                    "Declare every variable the template uses: {}.",
                    missing.join(", ")
                ),
            )
            .detail(json!({"rule": "messaging.template.variables_declared", "items": missing}))
            .next("create_message_template"));
        }
        let detail = match (variables.len(), enforced) {
            (0, _) => "No variables".to_owned(),
            (n, true) => format!(
                "{n} variable{}, all declared",
                if n == 1 { "" } else { "s" }
            ),
            (n, false) => format!(
                "{n} variable{} (not checked)",
                if n == 1 { "" } else { "s" }
            ),
        };
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        let plan = TemplatePlan {
            template_id: TemplateId::generate().to_string(),
            name: name.clone(),
            channel: channel.to_owned(),
            purpose: input.purpose.as_str().to_owned(),
            headline,
            text,
            variables,
            in_language: input
                .in_language
                .unwrap_or_else(|| DEFAULT_LANGUAGE.to_owned()),
        };
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "message_template".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"name": name, "channel": channel, "purpose": plan.purpose,
                           "status": "draft"}),
                note: None,
            }],
            checks: vec![Check {
                rule: "messaging.template.variables_declared".into(),
                passed: true,
                detail: Some(detail),
                fix_tool: None,
            }],
            basis: Vec::new(),
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Creates a message template".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &TemplatePlan) -> Result<Committed, ToolError> {
        repo::insert_template(
            tx,
            &NewTemplate {
                id: &plan.template_id,
                name: &plan.name,
                channel: &plan.channel,
                purpose: &plan.purpose,
                headline: plan.headline.as_deref(),
                text: &plan.text,
                variables: &plan.variables,
                in_language: &plan.in_language,
            },
        )
        .await?;
        Ok(Committed {
            id: plan.template_id.clone(),
            produced: Map::from_iter([("template_id".to_owned(), json!(plan.template_id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &TemplatePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let t = repo::template(&mut tx, &plan.template_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match t {
            None => vec![format!("template {} is missing", plan.template_id)],
            Some(t) if t.text != plan.text || t.message_channel != plan.channel => {
                vec![format!("template {} differs from the plan", t.id)]
            }
            Some(_) => Vec::new(),
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    /// Takes the new draft out of the list (rows are kept, never deleted).
    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &TemplatePlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        repo::set_template_status(tx, &plan.template_id, "archived").await
    }
}

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
                crate::select(
                    json!({"id": t.id, "name": t.name, "channel": t.message_channel,
                           "purpose": t.message_purpose, "status": t.template_status,
                           "headline": t.headline, "text": t.text, "variables": t.variables,
                           "in_language": t.in_language,
                           "created_at": cx.time.format(t.created_at)}),
                    &fields,
                )
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(next) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(next);
        }
        Ok(out)
    }
}
