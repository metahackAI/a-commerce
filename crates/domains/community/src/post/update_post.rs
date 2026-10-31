//! `update_post` (docs/api/community.md#post): the writer changes their post's headline, text or
//! secrecy. The word filter and the link limit run again on the new text; a filter that holds
//! instead of blocking puts the post back in the review queue. Sending the same values again
//! changes nothing and gives the same result.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::create_post::no_secret_here;
use crate::model::{self, SpamFinding};
use crate::repo::{self, PostEdit};
use crate::rules::{FilterRules, SpamRules};
use crate::{
    NOT_AUTHOR, POST_LOCKED, POST_NOT_FOUND, PROFANITY_BLOCKED, locked, not_author,
    policy_approval, post_not_found, profanity_blocked, required_text, spam_refused,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdatePostInput {
    /// The caller's own post (`list_posts` with `scope: mine`).
    #[schemars(regex(pattern = r"^post_[a-z0-9_]+$"), example = &"post_8812")]
    pub post_id: String,
    /// The new title.
    #[schemars(length(min = 1, max = 200), example = &"Runs a bit small")]
    pub headline: Option<String>,
    /// The new text.
    #[schemars(length(min = 1, max = 10000), example = &"Wash it cool, or size up if you like it loose.")]
    pub article_body: Option<String>,
    /// true: only the writer and the store can read it (only where the board allows secret
    /// posts); false: everyone can.
    #[serde(rename = "acom:is_secret", alias = "is_secret")]
    #[schemars(example = false)]
    pub is_secret: Option<bool>,
}

/// What `update_post` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePostPlan {
    pub post_id: String,
    pub author_id: String,
    pub headline: String,
    pub article_body: String,
    pub is_secret: bool,
    pub status: String,
    pub reason: Option<String>,
    /// Nothing differs from the post as it is.
    pub unchanged: bool,
    pub at: DateTime<Utc>,
    /// The blocked word, when the store blocks such text.
    pub blocked_word: Option<String>,
    /// What the spam rule counted, when it refuses the text.
    pub spam_refused: Option<SpamFinding>,
}

/// The writer changes their post.
#[derive(Debug)]
pub struct UpdatePost;

impl Operation for UpdatePost {
    const META: OpMeta = OpMeta {
        address: "community.post.update_post",
        tool: "update_post",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/community/posts/{post_id}",
        summary: "The writer changes their own community post: headline, text, or whether it is secret. Blocked words and links are checked again.",
        consumes: &[Slot::one("post_id")],
        produces: &[Slot::one("post_id")],
        checks: &["moderation.profanity_filter", "spam"],
        errors: &[NOT_AUTHOR, PROFANITY_BLOCKED, POST_NOT_FOUND, POST_LOCKED],
        audience: Audience::Both,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdatePostInput;
    type Output = Value;
}

fn change(id: &str, field: &str, from: Value, to: Value) -> Change {
    Change {
        op: ChangeOp::Update,
        entity: "post".into(),
        id: id.into(),
        from,
        to,
        note: Some(field.into()),
    }
}

impl WriteOp<Cx> for UpdatePost {
    type Plan = UpdatePostPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdatePostInput,
    ) -> Result<Planned<UpdatePostPlan>, ToolError> {
        if input.headline.is_none() && input.article_body.is_none() && input.is_secret.is_none() {
            return Err(ToolError::invalid_input(
                "Say what to change: `headline`, `article_body` or `acom:is_secret`.",
            )
            .detail(json!({"field": "headline", "reason": "missing"})));
        }
        let headline = input
            .headline
            .as_deref()
            .map(|h| required_text("headline", h))
            .transpose()?;
        let body = input
            .article_body
            .as_deref()
            .map(|b| required_text("article_body", b))
            .transpose()?;
        let filter = FilterRules::from_policy(&cx.policy)?;
        let spam = SpamRules::from_policy(&cx.policy)?;
        let me = &cx.principal.subject;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let p = repo::post(&mut tx, &input.post_id)
            .await?
            .ok_or_else(|| post_not_found(&input.post_id))?;
        if &p.author_id != me {
            return Err(if p.is_secret && !crate::is_operator(&cx.principal) {
                post_not_found(&p.id)
            } else {
                not_author("post", &p.id, "get_post")
            });
        }
        if matches!(p.moderation_status.as_str(), "blinded" | "deleted") {
            return Err(locked("post", &p.id, &p.moderation_status));
        }
        let board = repo::board(&mut tx, &p.board_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let secret = input.is_secret.unwrap_or(p.is_secret);
        if secret && !p.is_secret && !board.is_some_and(|b| b.allow_secret) {
            return Err(no_secret_here(&p.board_id));
        }
        let headline = headline.unwrap_or_else(|| p.headline.clone());
        let body = body.unwrap_or_else(|| p.article_body.clone());

        let mut changes = Vec::new();
        if headline != p.headline {
            changes.push(change(&p.id, "headline", json!(p.headline), json!(headline)));
        }
        if body != p.article_body {
            changes.push(change(
                &p.id,
                "article_body",
                json!(p.article_body),
                json!(body),
            ));
        }
        if secret != p.is_secret {
            changes.push(change(
                &p.id,
                "acom:is_secret",
                json!(p.is_secret),
                json!(secret),
            ));
        }
        let text = format!("{headline}\n{body}");
        let found = model::filter_finds(&filter, &text);
        // An edit is not a new write: only the link limit of the spam rule applies.
        let finding = SpamFinding::new(&spam, model::link_count(&text), 0);
        let checks = vec![
            model::profanity_check(&filter, found.as_deref(), "update_post"),
            model::spam_check(&spam, &finding, "update_post"),
        ];
        let held_by = if found.is_some() && !filter.blocks {
            Some("profanity_filter")
        } else if finding.matched() && !spam.blocks {
            Some("spam")
        } else {
            None
        };
        let (status, reason) = match held_by {
            Some(why) if !changes.is_empty() => {
                if p.moderation_status != "held_for_review" {
                    changes.push(change(
                        &p.id,
                        "acom:moderation_status",
                        json!(p.moderation_status),
                        json!("held_for_review"),
                    ));
                }
                ("held_for_review".to_owned(), Some(why.to_owned()))
            }
            _ => (p.moderation_status.clone(), p.moderation_reason.clone()),
        };
        Ok(Planned {
            approval: policy_approval(cx, Self::META.tool, &json!({"changes": changes.len()})),
            basis: vec![Basis {
                entity: "community.discussion_forum_posting".into(),
                id: p.id.clone(),
                version: i64::from(p.version),
            }],
            plan: UpdatePostPlan {
                post_id: p.id,
                author_id: p.author_id,
                unchanged: changes.is_empty(),
                headline,
                article_body: body,
                is_secret: secret,
                status,
                reason,
                at: cx.clock.now(),
                blocked_word: found.filter(|_| filter.blocks),
                spam_refused: (finding.matched() && spam.blocks).then_some(finding),
            },
            changes,
            checks,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &UpdatePostPlan) -> Result<Committed, ToolError> {
        if let Some(word) = &plan.blocked_word {
            return Err(profanity_blocked(word, "update_post"));
        }
        if let Some(spam) = &plan.spam_refused {
            return Err(spam_refused(spam, "update_post"));
        }
        if !plan.unchanged {
            let n = repo::edit_post(
                tx,
                &PostEdit {
                    id: &plan.post_id,
                    author_id: &plan.author_id,
                    headline: &plan.headline,
                    article_body: &plan.article_body,
                    is_secret: plan.is_secret,
                    status: &plan.status,
                    reason: plan.reason.as_deref(),
                    at: plan.at,
                },
            )
            .await?;
            if n == 0 {
                // Blinded or deleted since the preview.
                return Err(match repo::post(tx, &plan.post_id).await? {
                    Some(p) => locked("post", &p.id, &p.moderation_status),
                    None => post_not_found(&plan.post_id),
                });
            }
        }
        let mut produced = Map::new();
        produced.insert("post_id".into(), json!(plan.post_id));
        Ok(Committed {
            id: plan.post_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &UpdatePostPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::post(&mut tx, &plan.post_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = row.is_some_and(|p| {
            p.headline == plan.headline
                && p.article_body == plan.article_body
                && p.is_secret == plan.is_secret
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} does not read back as changed", plan.post_id)]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &UpdatePostPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} is its writer's own post; they change it again with update_post.",
                plan.post_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    fn input(post: &str) -> UpdatePostInput {
        UpdatePostInput {
            post_id: post.into(),
            headline: None,
            article_body: None,
            is_secret: None,
        }
    }

    #[tokio::test]
    async fn an_edit_must_change_something() {
        let e = UpdatePost
            .preview(&testing::shopper(), input("post_8812"))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        let schema = acom_contract::schema::closed_schema_for::<UpdatePostInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn only_the_writer_edits_and_the_filter_runs_again() {
        let jamie = testing::on_db(testing::shopper()).await;
        let planned = UpdatePost
            .preview(
                &jamie,
                UpdatePostInput {
                    headline: Some("Runs half a size small".into()),
                    ..input("post_8812")
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes.len(), 1);
        assert_eq!(planned.changes[0].from, json!("Runs a bit small"));
        assert_eq!(planned.changes[0].note.as_deref(), Some("headline"));
        let mut tx = jamie.begin().await.unwrap_or_else(|e| panic!("{e}"));
        UpdatePost
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let row = repo::post(&mut tx, "post_8812")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            row.map(|p| p.headline).as_deref(),
            Some("Runs half a size small")
        );
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));

        let grace = testing::as_subject(&jamie, "cus_grace");
        let e = UpdatePost
            .preview(
                &grace,
                UpdatePostInput {
                    headline: Some("Mine now".into()),
                    ..input("post_8812")
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("not_author", Some("get_post"))
        );

        let strict = jamie
            .policy
            .with_overrides([(
                "community",
                "moderation:\n  profanity_filter:\n    blocked_words: [cheap]\n",
            )])
            .unwrap_or_else(|e| panic!("{e}"));
        let strict_cx = Cx {
            policy: std::sync::Arc::new(strict),
            ..jamie.clone()
        };
        let blocked = UpdatePost
            .preview(
                &strict_cx,
                UpdatePostInput {
                    article_body: Some("Cheap wool, honestly.".into()),
                    ..input("post_8812")
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!blocked.checks[0].passed);
        let mut tx = jamie.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let e = UpdatePost
            .commit(&mut tx, &blocked.plan)
            .await
            .unwrap_err();
        assert_eq!(e.code, "profanity_blocked");
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
    }
}
