//! `update_reply` (docs/api/community.md#reply): the writer changes their reply's text or
//! secrecy, and the word filter runs again. A blinded or deleted reply, or one on a blinded or
//! deleted post, cannot change (`post_locked`).

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::reply_not_found;
use crate::model;
use crate::post::create_post::no_secret_here;
use crate::repo::{self, ReplyEdit};
use crate::rules::FilterRules;
use crate::{
    NOT_AUTHOR, POST_LOCKED, PROFANITY_BLOCKED, locked, not_author, policy_approval,
    profanity_blocked, required_text,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateReplyInput {
    /// The caller's own reply (`list_replies`).
    #[schemars(regex(pattern = r"^rpl_[a-z0-9_]+$"), example = &"rpl_5521")]
    pub reply_id: String,
    /// The new text.
    #[schemars(length(min = 1, max = 2000), example = &"Same here, size up.")]
    pub text: Option<String>,
    /// true: only the writer, the post's writer and the store can read it (only on a board
    /// that allows secret posts); false: everyone can.
    #[serde(rename = "acom:is_secret", alias = "is_secret")]
    #[schemars(example = false)]
    pub is_secret: Option<bool>,
}

/// What `update_reply` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateReplyPlan {
    pub reply_id: String,
    pub author_id: String,
    pub text: String,
    pub is_secret: bool,
    pub status: String,
    pub reason: Option<String>,
    pub unchanged: bool,
    pub at: DateTime<Utc>,
    /// The blocked word, when the store blocks such text.
    pub blocked_word: Option<String>,
}

/// The writer changes their reply.
#[derive(Debug)]
pub struct UpdateReply;

impl Operation for UpdateReply {
    const META: OpMeta = OpMeta {
        address: "community.reply.update_reply",
        tool: "update_reply",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/community/replies/{reply_id}",
        summary: "The writer changes their own reply on a community post: its text, or whether it is secret. Blocked words are checked again.",
        consumes: &[Slot::one("reply_id")],
        produces: &[Slot::one("reply_id")],
        checks: &["moderation.profanity_filter"],
        errors: &[
            NOT_AUTHOR,
            PROFANITY_BLOCKED,
            POST_LOCKED,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Both,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateReplyInput;
    type Output = Value;
}

fn change(id: &str, field: &str, from: Value, to: Value) -> Change {
    Change {
        op: ChangeOp::Update,
        entity: "reply".into(),
        id: id.into(),
        from,
        to,
        note: Some(field.into()),
    }
}

impl WriteOp<Cx> for UpdateReply {
    type Plan = UpdateReplyPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateReplyInput,
    ) -> Result<Planned<UpdateReplyPlan>, ToolError> {
        if input.text.is_none() && input.is_secret.is_none() {
            return Err(ToolError::invalid_input(
                "Say what to change: `text` or `acom:is_secret`.",
            )
            .detail(json!({"field": "text", "reason": "missing"})));
        }
        let text = input
            .text
            .as_deref()
            .map(|t| required_text("text", t))
            .transpose()?;
        let filter = FilterRules::from_policy(&cx.policy)?;
        let me = &cx.principal.subject;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::reply(&mut tx, &input.reply_id)
            .await?
            .ok_or_else(|| reply_not_found(&input.reply_id))?;
        if &r.author_id != me {
            return Err(not_author("reply", &r.id, "list_replies"));
        }
        if matches!(r.moderation_status.as_str(), "blinded" | "deleted") {
            return Err(locked("reply", &r.id, &r.moderation_status));
        }
        let post = repo::post(&mut tx, &r.post_id).await?;
        let board = match &post {
            Some(p) => repo::board(&mut tx, &p.board_id).await?,
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;
        if let Some(p) = &post
            && matches!(p.moderation_status.as_str(), "blinded" | "deleted")
        {
            return Err(locked("post", &p.id, &p.moderation_status));
        }
        let secret = input.is_secret.unwrap_or(r.is_secret);
        if secret && !r.is_secret && !board.as_ref().is_some_and(|b| b.allow_secret) {
            return Err(no_secret_here(
                board.as_ref().map_or(r.post_id.as_str(), |b| b.id.as_str()),
            ));
        }
        let text = text.unwrap_or_else(|| r.text.clone());
        let mut changes = Vec::new();
        if text != r.text {
            changes.push(change(&r.id, "text", json!(r.text), json!(text)));
        }
        if secret != r.is_secret {
            changes.push(change(
                &r.id,
                "acom:is_secret",
                json!(r.is_secret),
                json!(secret),
            ));
        }
        let found = model::filter_finds(&filter, &text);
        let check = model::profanity_check(&filter, found.as_deref(), "update_reply");
        let (status, reason) = if found.is_some() && !filter.blocks && !changes.is_empty() {
            if r.moderation_status != "held_for_review" {
                changes.push(change(
                    &r.id,
                    "acom:moderation_status",
                    json!(r.moderation_status),
                    json!("held_for_review"),
                ));
            }
            (
                "held_for_review".to_owned(),
                Some("profanity_filter".to_owned()),
            )
        } else {
            (r.moderation_status.clone(), r.moderation_reason.clone())
        };
        Ok(Planned {
            approval: policy_approval(cx, Self::META.tool, &json!({"changes": changes.len()})),
            basis: vec![Basis {
                entity: "community.comment".into(),
                id: r.id.clone(),
                version: i64::from(r.version),
            }],
            plan: UpdateReplyPlan {
                reply_id: r.id,
                author_id: r.author_id,
                unchanged: changes.is_empty(),
                text,
                is_secret: secret,
                status,
                reason,
                at: cx.clock.now(),
                blocked_word: found.filter(|_| filter.blocks),
            },
            changes,
            checks: vec![check],
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &UpdateReplyPlan) -> Result<Committed, ToolError> {
        if let Some(word) = &plan.blocked_word {
            return Err(profanity_blocked(word, "update_reply"));
        }
        if !plan.unchanged
            && repo::edit_reply(
                tx,
                &ReplyEdit {
                    id: &plan.reply_id,
                    author_id: &plan.author_id,
                    text: &plan.text,
                    is_secret: plan.is_secret,
                    status: &plan.status,
                    reason: plan.reason.as_deref(),
                    at: plan.at,
                },
            )
            .await?
                == 0
        {
            return Err(match repo::reply(tx, &plan.reply_id).await? {
                Some(r) => locked("reply", &r.id, &r.moderation_status),
                None => reply_not_found(&plan.reply_id),
            });
        }
        let mut produced = Map::new();
        produced.insert("reply_id".into(), json!(plan.reply_id));
        Ok(Committed {
            id: plan.reply_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &UpdateReplyPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::reply(&mut tx, &plan.reply_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = row.is_some_and(|r| r.text == plan.text && r.is_secret == plan.is_secret);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} does not read back as changed", plan.reply_id)]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &UpdateReplyPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} is its writer's own reply; they change it again with update_reply.",
                plan.reply_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    async fn an_edit_must_change_something() {
        let e = UpdateReply
            .preview(
                &testing::shopper(),
                UpdateReplyInput {
                    reply_id: "rpl_5523".into(),
                    text: None,
                    is_secret: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        let schema = acom_contract::schema::closed_schema_for::<UpdateReplyInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn only_the_writer_changes_a_reply() {
        let jamie = testing::on_db(testing::shopper()).await;
        let planned = UpdateReply
            .preview(
                &jamie,
                UpdateReplyInput {
                    reply_id: "rpl_5523".into(),
                    text: Some("Good to know it is not just mine. Sized up.".into()),
                    is_secret: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes[0].note.as_deref(), Some("text"));
        let mut tx = jamie.begin().await.unwrap_or_else(|e| panic!("{e}"));
        UpdateReply
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let row = repo::reply(&mut tx, "rpl_5523")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(row.is_some_and(|r| r.text.ends_with("Sized up.") && r.date_modified.is_some()));
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
        let e = UpdateReply
            .preview(
                &jamie,
                UpdateReplyInput {
                    reply_id: "rpl_5521".into(),
                    text: Some("Not mine".into()),
                    is_secret: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("not_author", Some("list_replies"))
        );
    }
}
