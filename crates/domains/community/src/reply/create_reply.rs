//! `create_reply` (docs/api/community.md#reply): a reply to a post, or to another reply up to
//! `reply.max_depth`. The word filter and the spam rule run as for posts. Signed-in members and
//! the store reply anywhere; guests only on boards where everyone may write. A blinded or deleted
//! post takes no replies (`post_locked`). The same writer sending the same text to the same post
//! within `spam.duplicate_window_minutes` gets the first reply back.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::reply_not_found;
use crate::model::{self, SpamFinding};
use crate::post::create_post::no_secret_here;
use crate::post::open_post;
use crate::repo::{self, NewReply};
use crate::rules::{FilterRules, ReplyRules, SpamRules};
use crate::writer::writer;
use crate::{
    DEPTH_EXCEEDED, POST_LOCKED, POST_NOT_FOUND, PROFANITY_BLOCKED, Viewer, locked,
    policy_approval, post_not_found, profanity_blocked, required_text, spam_refused,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateReplyInput {
    /// The post to reply on (`list_posts`).
    #[schemars(regex(pattern = r"^post_[a-z0-9_]+$"), example = &"post_8812")]
    pub post_id: String,
    /// The reply.
    #[schemars(length(min = 1, max = 2000), example = &"Same here, size up.")]
    pub text: String,
    /// The reply this one answers (`list_replies`); leave out to reply to the post itself.
    #[schemars(regex(pattern = r"^rpl_[a-z0-9_]+$"), example = &"rpl_5521")]
    pub parent_item: Option<String>,
    /// true: only the writer, the post's writer and the store can read it (only on a board that
    /// allows secret posts). Default false.
    #[serde(rename = "acom:is_secret", alias = "is_secret")]
    #[schemars(example = false)]
    pub is_secret: Option<bool>,
}

/// What `create_reply` runs, with what its checks found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateReplyPlan {
    pub reply_id: String,
    pub post_id: String,
    pub parent_item: Option<String>,
    pub depth: i32,
    pub author_id: String,
    pub author_name: Option<String>,
    pub text: String,
    pub is_secret: bool,
    pub status: String,
    pub reason: Option<String>,
    pub at: DateTime<Utc>,
    /// The same reply written moments ago: running returns it.
    pub duplicate_of: Option<String>,
    pub max_depth: i32,
    /// The blocked word, when the store blocks such text.
    pub blocked_word: Option<String>,
    /// What the spam rule counted, when it refuses the reply.
    pub spam_refused: Option<SpamFinding>,
}

/// Reply to a post or to a reply.
#[derive(Debug)]
pub struct CreateReply;

impl Operation for CreateReply {
    const META: OpMeta = OpMeta {
        address: "community.reply.create_reply",
        tool: "create_reply",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/community/posts/{post_id}/replies",
        summary: "Reply to a community post, or to a reply on it (as deep as the store allows). Checks blocked words, spam and depth; secret where the board allows.",
        consumes: &[Slot::one("post_id")],
        produces: &[Slot::one("reply_id")],
        checks: &["moderation.profanity_filter", "spam", "reply.max_depth"],
        errors: &[
            PROFANITY_BLOCKED,
            POST_LOCKED,
            DEPTH_EXCEEDED,
            POST_NOT_FOUND,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Both,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateReplyInput;
    type Output = Value;
}

/// `depth_exceeded`: the reply would sit deeper than the store allows.
fn too_deep(depth: i32, max: i32) -> ToolError {
    ToolError::new(
        DEPTH_EXCEEDED,
        format!(
            "Replies go {max} deep at most; this one would be {depth}. Reply higher up the thread."
        ),
    )
    .detail(json!({"limit": max, "actual": depth, "policy_key": "reply.max_depth"}))
    .next("create_reply")
}

impl WriteOp<Cx> for CreateReply {
    type Plan = CreateReplyPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateReplyInput,
    ) -> Result<Planned<CreateReplyPlan>, ToolError> {
        let text = required_text("text", &input.text)?;
        let filter = FilterRules::from_policy(&cx.policy)?;
        let spam = SpamRules::from_policy(&cx.policy)?;
        let depth_rules = ReplyRules::from_policy(&cx.policy)?;
        let viewer = Viewer::of(&cx.principal);
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let post = open_post(&mut tx, &viewer, &input.post_id)
            .await
            .map_err(|e| {
                if e.code == crate::SECRET_POST.code {
                    post_not_found(&input.post_id)
                } else {
                    e
                }
            })?;
        if matches!(post.moderation_status.as_str(), "blinded" | "deleted") {
            return Err(locked("post", &post.id, &post.moderation_status));
        }
        let board = repo::board(&mut tx, &post.board_id)
            .await?
            .ok_or_else(|| post_not_found(&post.id))?;
        let parent = match &input.parent_item {
            None => None,
            Some(id) => {
                let r = repo::reply(&mut tx, id)
                    .await?
                    .filter(|r| {
                        r.post_id == post.id && super::readable(&viewer, &post.author_id, r)
                    })
                    .ok_or_else(|| reply_not_found(id))?;
                Some(r)
            }
        };
        let who = writer(cx, &mut tx).await?;
        let this_hour = repo::writes_since(&mut tx, &who.id, now - Duration::hours(1)).await? + 1;
        let duplicate = repo::same_reply_since(
            &mut tx,
            &who.id,
            &post.id,
            &text,
            now - Duration::minutes(spam.duplicate_window_minutes),
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let (may, why) = model::may_write(
            model::reply_permission(&board.write_permission),
            who.kind,
            who.member_state.as_deref(),
        );
        if !may {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                format!("You cannot reply on this board: {why}."),
            )
            .detail(json!({"reason": "role", "rule": "board.write_permission"})));
        }
        if let Some(p) = &parent
            && matches!(p.moderation_status.as_str(), "blinded" | "deleted")
        {
            return Err(locked("reply", &p.id, &p.moderation_status));
        }
        let secret = input.is_secret.unwrap_or(false);
        if secret && !board.allow_secret {
            return Err(no_secret_here(&board.id));
        }
        let depth = parent.as_ref().map_or(1, |p| p.reply_depth + 1);
        let found = model::filter_finds(&filter, &text);
        let finding = SpamFinding::new(&spam, model::link_count(&text), this_hour);
        let checks = vec![
            model::profanity_check(&filter, found.as_deref(), "create_reply"),
            model::spam_check(&spam, &finding, "create_reply"),
            model::depth_check(&depth_rules, depth),
        ];
        let held_by = if found.is_some() && !filter.blocks {
            Some("profanity_filter")
        } else if finding.matched() && !spam.blocks {
            Some("spam")
        } else {
            None
        };
        let (status, reason) = model::new_status(&board.moderation_mode, held_by);
        let shown = model::shown_author(&who.id, who.name.as_deref());
        let changes = match &duplicate {
            Some(first) => vec![Change {
                op: ChangeOp::Exclude,
                entity: "reply".into(),
                id: first.clone(),
                from: Value::Null,
                to: Value::Null,
                note: Some(format!(
                    "The same reply went up moments ago as {first}; running returns it"
                )),
            }],
            None => vec![Change {
                op: ChangeOp::Create,
                entity: "reply".into(),
                id: String::new(),
                from: Value::Null,
                to: json!(status),
                note: Some(format!(
                    "By {shown} on {}{}{}",
                    post.id,
                    parent
                        .as_ref()
                        .map(|p| format!(", answering {}", p.id))
                        .unwrap_or_default(),
                    if secret { ", secret" } else { "" }
                )),
            }],
        };
        Ok(Planned {
            changes,
            approval: policy_approval(cx, Self::META.tool, &json!({"depth": depth})),
            basis: vec![Basis {
                entity: "community.discussion_forum_posting".into(),
                id: post.id.clone(),
                version: i64::from(post.version),
            }],
            plan: CreateReplyPlan {
                reply_id: format!("rpl_{}", acom_core::id::new_suffix()),
                post_id: post.id,
                parent_item: parent.map(|p| p.id),
                depth,
                author_id: who.id,
                author_name: who.name,
                text,
                is_secret: secret,
                status: status.into(),
                reason: reason.map(str::to_owned),
                at: now,
                duplicate_of: duplicate,
                max_depth: depth_rules.max_depth,
                blocked_word: found.filter(|_| filter.blocks),
                spam_refused: (finding.matched() && spam.blocks).then_some(finding),
            },
            checks,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &CreateReplyPlan) -> Result<Committed, ToolError> {
        let done = |id: &str| {
            let mut produced = Map::new();
            produced.insert("reply_id".into(), json!(id));
            Committed {
                id: id.to_owned(),
                produced,
            }
        };
        if let Some(first) = &plan.duplicate_of {
            return Ok(done(first));
        }
        if plan.depth > plan.max_depth {
            return Err(too_deep(plan.depth, plan.max_depth));
        }
        if let Some(word) = &plan.blocked_word {
            return Err(profanity_blocked(word, "create_reply"));
        }
        if let Some(spam) = &plan.spam_refused {
            return Err(spam_refused(spam, "create_reply"));
        }
        match repo::post(tx, &plan.post_id).await? {
            None => return Err(post_not_found(&plan.post_id)),
            Some(p) if matches!(p.moderation_status.as_str(), "blinded" | "deleted") => {
                return Err(locked("post", &p.id, &p.moderation_status));
            }
            Some(_) => {}
        }
        let window = SpamRules::from_policy(&tx.policy)?.duplicate_window_minutes;
        if let Some(first) = repo::same_reply_since(
            tx,
            &plan.author_id,
            &plan.post_id,
            &plan.text,
            plan.at - Duration::minutes(window),
        )
        .await?
            && first != plan.reply_id
        {
            return Ok(done(&first));
        }
        repo::insert_reply(
            tx,
            &NewReply {
                id: &plan.reply_id,
                post_id: &plan.post_id,
                parent_item_id: plan.parent_item.as_deref(),
                depth: plan.depth,
                author_id: &plan.author_id,
                author_name: plan.author_name.as_deref(),
                text: &plan.text,
                is_secret: plan.is_secret,
                status: &plan.status,
                reason: plan.reason.as_deref(),
                at: plan.at,
            },
        )
        .await?;
        Ok(done(&plan.reply_id))
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CreateReplyPlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::reply(&mut tx, &done.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = row.is_some_and(|r| {
            r.post_id == plan.post_id
                && r.author_id == plan.author_id
                && (plan.duplicate_of.is_some() || r.text == plan.text)
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} does not read back as written", done.id)]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &CreateReplyPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} is its writer's own reply; they remove it with delete_reply.",
                plan.reply_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    fn input(post: &str, parent: Option<&str>) -> CreateReplyInput {
        CreateReplyInput {
            post_id: post.into(),
            text: "Same here, size up.".into(),
            parent_item: parent.map(str::to_owned),
            is_secret: None,
        }
    }

    #[test]
    fn too_deep_names_the_limit() {
        let e = too_deep(3, 2);
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("depth_exceeded", Some("create_reply"))
        );
        let schema = acom_contract::schema::closed_schema_for::<CreateReplyInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }

    #[tokio::test]
    async fn an_empty_reply_is_refused() {
        let e = CreateReply
            .preview(
                &testing::shopper(),
                CreateReplyInput {
                    text: " ".into(),
                    ..input("post_8812", None)
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn replies_nest_two_deep_and_no_further() {
        let owen = testing::as_subject(&testing::on_db(testing::shopper()).await, "cus_owen");
        let first = CreateReply
            .preview(&owen, input("post_8812", None))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(first.checks.iter().all(|c| c.passed), "{:?}", first.checks);
        assert_eq!(first.checks[2].detail.as_deref(), Some("Depth 1 of 2"));
        assert_eq!(first.plan.author_name.as_deref(), Some("Ow**"));
        let mut tx = owen.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = CreateReply
            .commit(&mut tx, &first.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(done.id, first.plan.reply_id);
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));

        // rpl_5523 is already a reply to a reply: answering it would be depth 3.
        let deep = CreateReply
            .preview(&owen, input("post_8812", Some("rpl_5523")))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!deep.checks[2].passed);
        let mut tx = owen.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let e = CreateReply.commit(&mut tx, &deep.plan).await.unwrap_err();
        assert_eq!(e.code, "depth_exceeded");
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));

        // A guest cannot reply on a members' board.
        let guest = testing::on_db(testing::guest()).await;
        let e = CreateReply
            .preview(&guest, input("post_8812", None))
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }
}
