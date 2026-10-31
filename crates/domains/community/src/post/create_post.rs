//! `create_post` (docs/api/community.md#post): a post on a board. The preview runs the word
//! filter, the spam rule, the attachment rule and the board's write permission; the post shows
//! at once, or waits for review on a `pre_review` board or when a filter holds instead of
//! blocking. The same writer posting the same headline on the same board within
//! `spam.duplicate_window_minutes` gets the first post back.

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::model::{self, SpamFinding, WriterKind};
use crate::repo::{self, NewPost};
use crate::rules::{AttachmentRules, FilterRules, SpamRules};
use crate::writer::writer;
use crate::{
    ATTACHMENT_NOT_ALLOWED, BOARD_NOT_FOUND, PROFANITY_BLOCKED, WRITE_NOT_ALLOWED, board_not_found,
    policy_approval, profanity_blocked, required_text, spam_refused, trimmed,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePostInput {
    /// The board to post on (`list_boards`).
    #[schemars(regex(pattern = r"^brd_[a-z0-9_]+$"), example = &"brd_reviews")]
    pub board_id: String,
    /// The title.
    #[schemars(length(min = 1, max = 200), example = &"Runs a bit small")]
    pub headline: String,
    /// The text.
    #[schemars(length(min = 1, max = 10000), example = &"The merino trail tee came out of a warm wash half a size smaller.")]
    pub article_body: String,
    /// Uploaded images to attach (`upl_…`), only on a board that takes attachments.
    #[schemars(length(max = 20), example = ["upl_8c1f"])]
    pub associated_media: Option<Vec<String>>,
    /// true: only the writer and the store can read it (only on a board that allows secret
    /// posts). Default false.
    #[serde(rename = "acom:is_secret", alias = "is_secret")]
    #[schemars(example = false)]
    pub is_secret: Option<bool>,
    /// The language it is written in (BCP 47).
    #[schemars(length(min = 2, max = 35), example = &"en")]
    pub in_language: Option<String>,
}

/// What `create_post` runs, with what its checks found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePostPlan {
    pub post_id: String,
    pub board_id: String,
    pub author_id: String,
    pub author_name: Option<String>,
    pub headline: String,
    pub article_body: String,
    pub associated_media: Vec<String>,
    pub is_secret: bool,
    pub in_language: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub at: DateTime<Utc>,
    /// The same post written moments ago: running returns it.
    pub duplicate_of: Option<String>,
    /// Why the board refuses this writer, when it does.
    pub write_refused: Option<String>,
    /// Why the attachments are refused, when they are.
    pub attachment_refused: Option<String>,
    /// The blocked word, when the store blocks such text.
    pub blocked_word: Option<String>,
    /// What the spam rule counted, when it refuses the post.
    pub spam_refused: Option<SpamFinding>,
}

/// Write a post on a board.
#[derive(Debug)]
pub struct CreatePost;

impl Operation for CreatePost {
    const META: OpMeta = OpMeta {
        address: "community.post.create_post",
        tool: "create_post",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/community/boards/{board_id}/posts",
        summary: "Write a post on a community board: headline, text, images where the board takes them, secret where it allows. Checks blocked words, spam, attachments and who may write; on a pre-review board the post waits for the store.",
        consumes: &[Slot::one("board_id")],
        produces: &[Slot::one("post_id")],
        checks: &[
            "moderation.profanity_filter",
            "spam",
            "attachment",
            "board.write_permission",
        ],
        errors: &[
            PROFANITY_BLOCKED,
            WRITE_NOT_ALLOWED,
            ATTACHMENT_NOT_ALLOWED,
            BOARD_NOT_FOUND,
        ],
        audience: Audience::Both,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreatePostInput;
    type Output = Value;
}

/// The uploads named, trimmed and checked for their prefix.
pub(crate) fn uploads(media: Option<Vec<String>>) -> Result<Vec<String>, ToolError> {
    let media: Vec<String> = media
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty())
        .collect();
    if let Some(bad) = media.iter().find(|m| {
        !m.strip_prefix("upl_").is_some_and(|s| {
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
    }) {
        return Err(
            ToolError::invalid_input(format!("`{bad}` is not an upload id (upl_…)."))
                .detail(json!({"field": "associated_media", "reason": "format"})),
        );
    }
    Ok(media)
}

/// `invalid_input` for a secret post on a board that takes none.
pub(crate) fn no_secret_here(board: &str) -> ToolError {
    ToolError::invalid_input(format!(
        "Board {board} takes no secret posts; leave out `acom:is_secret`."
    ))
    .detail(json!({"field": "acom:is_secret", "reason": "not_allowed"}))
}

/// Why a planned post cannot go up, in the order the spec names the errors.
pub(crate) fn refusal(plan: &CreatePostPlan) -> Option<ToolError> {
    if let Some(why) = &plan.write_refused {
        return Some(
            ToolError::new(
                WRITE_NOT_ALLOWED,
                format!("You cannot post on {}: {why}.", plan.board_id),
            )
            .detail(json!({"reason": "role", "rule": "board.write_permission"}))
            .next("list_boards"),
        );
    }
    if let Some(why) = &plan.attachment_refused {
        return Some(
            ToolError::new(
                ATTACHMENT_NOT_ALLOWED,
                format!("{why}. Leave out the images and post again."),
            )
            .detail(json!({"rule": "attachment", "field": "associated_media"}))
            .next("create_post"),
        );
    }
    if let Some(word) = &plan.blocked_word {
        return Some(profanity_blocked(word, "create_post"));
    }
    plan.spam_refused
        .as_ref()
        .map(|spam| spam_refused(spam, "create_post"))
}

impl WriteOp<Cx> for CreatePost {
    type Plan = CreatePostPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreatePostInput,
    ) -> Result<Planned<CreatePostPlan>, ToolError> {
        let headline = required_text("headline", &input.headline)?;
        let body = required_text("article_body", &input.article_body)?;
        let media = uploads(input.associated_media)?;
        let filter = FilterRules::from_policy(&cx.policy)?;
        let spam = SpamRules::from_policy(&cx.policy)?;
        let files = AttachmentRules::from_policy(&cx.policy)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let board = repo::board(&mut tx, &input.board_id)
            .await?
            .ok_or_else(|| board_not_found(&input.board_id))?;
        let who = writer(cx, &mut tx).await?;
        let this_hour = repo::writes_since(&mut tx, &who.id, now - Duration::hours(1)).await? + 1;
        let duplicate = repo::same_post_since(
            &mut tx,
            &who.id,
            &board.id,
            &headline,
            now - Duration::minutes(spam.duplicate_window_minutes),
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let secret = input.is_secret.unwrap_or(false);
        if secret && !board.allow_secret {
            return Err(no_secret_here(&board.id));
        }

        let text = format!("{headline}\n{body}");
        let found = model::filter_finds(&filter, &text);
        let finding = SpamFinding::new(&spam, model::link_count(&text), this_hour);
        let checks = vec![
            model::profanity_check(&filter, found.as_deref(), "create_post"),
            model::spam_check(&spam, &finding, "create_post"),
            model::attachment_check(&files, board.allow_attachment, media.len()),
            model::write_permission_check(
                &board.write_permission,
                who.kind,
                who.member_state.as_deref(),
            ),
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
        let mut changes = vec![Change {
            op: ChangeOp::Create,
            entity: "post".into(),
            id: String::new(),
            from: Value::Null,
            to: json!(status),
            note: Some(format!(
                "\"{headline}\" on {} by {shown}{}{}",
                board.name,
                if secret { ", secret" } else { "" },
                match media.len() {
                    0 => String::new(),
                    1 => ", 1 image".into(),
                    n => format!(", {n} images"),
                }
            )),
        }];
        if let Some(first) = &duplicate {
            changes = vec![Change {
                op: ChangeOp::Exclude,
                entity: "post".into(),
                id: first.clone(),
                from: Value::Null,
                to: Value::Null,
                note: Some(format!(
                    "The same post went up moments ago as {first}; running returns it"
                )),
            }];
        }
        let attachment_refused =
            (!checks[2].passed).then(|| checks[2].detail.clone().unwrap_or_default());
        let write_refused =
            (!checks[3].passed).then(|| checks[3].detail.clone().unwrap_or_default());
        let staff = who.kind == WriterKind::Staff;
        Ok(Planned {
            changes,
            approval: policy_approval(
                cx,
                Self::META.tool,
                &json!({"board_type": board.board_type, "by_store": staff}),
            ),
            basis: vec![Basis {
                entity: "community.board".into(),
                id: board.id.clone(),
                version: i64::from(board.version),
            }],
            plan: CreatePostPlan {
                post_id: format!("post_{}", acom_core::id::new_suffix()),
                board_id: board.id,
                author_id: who.id,
                author_name: who.name,
                headline,
                article_body: body,
                associated_media: media,
                is_secret: secret,
                in_language: trimmed(input.in_language.as_deref()),
                status: status.into(),
                reason: reason.map(str::to_owned),
                at: now,
                duplicate_of: duplicate,
                write_refused,
                attachment_refused,
                blocked_word: found.filter(|_| filter.blocks),
                spam_refused: (finding.matched() && spam.blocks).then_some(finding),
            },
            checks,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &CreatePostPlan) -> Result<Committed, ToolError> {
        let done = |id: &str| {
            let mut produced = Map::new();
            produced.insert("post_id".into(), json!(id));
            Committed {
                id: id.to_owned(),
                produced,
            }
        };
        if let Some(first) = &plan.duplicate_of {
            return Ok(done(first));
        }
        if let Some(e) = refusal(plan) {
            return Err(e);
        }
        // The same post may have gone up from another preview since this one.
        let window = SpamRules::from_policy(&tx.policy)?.duplicate_window_minutes;
        if let Some(first) = repo::same_post_since(
            tx,
            &plan.author_id,
            &plan.board_id,
            &plan.headline,
            plan.at - Duration::minutes(window),
        )
        .await?
            && first != plan.post_id
        {
            return Ok(done(&first));
        }
        repo::insert_post(
            tx,
            &NewPost {
                id: &plan.post_id,
                board_id: &plan.board_id,
                author_id: &plan.author_id,
                author_name: plan.author_name.as_deref(),
                headline: &plan.headline,
                article_body: &plan.article_body,
                associated_media: &plan.associated_media,
                is_secret: plan.is_secret,
                in_language: plan.in_language.as_deref(),
                status: &plan.status,
                reason: plan.reason.as_deref(),
                at: plan.at,
            },
        )
        .await?;
        Ok(done(&plan.post_id))
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CreatePostPlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::post(&mut tx, &done.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = row.is_some_and(|p| {
            p.board_id == plan.board_id
                && p.author_id == plan.author_id
                && (plan.duplicate_of.is_some() || p.headline == plan.headline)
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
        plan: &CreatePostPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} is its writer's own post; they remove it with delete_post.",
                plan.post_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    fn input(board: &str) -> CreatePostInput {
        CreatePostInput {
            board_id: board.into(),
            headline: "Runs a bit small".into(),
            article_body: "Size up if you layer.".into(),
            associated_media: None,
            is_secret: None,
            in_language: Some("en".into()),
        }
    }

    #[test]
    fn uploads_are_upload_ids() {
        assert_eq!(
            uploads(Some(vec![" upl_8c1f ".into(), String::new()])).unwrap_or_default(),
            ["upl_8c1f"]
        );
        let e = uploads(Some(vec!["https://x.example/a.jpg".into()])).unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[tokio::test]
    async fn an_empty_headline_is_refused_before_anything_is_read() {
        let e = CreatePost
            .preview(
                &testing::shopper(),
                CreatePostInput {
                    headline: "  ".into(),
                    ..input("brd_reviews")
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        let schema = acom_contract::schema::closed_schema_for::<CreatePostInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert!(schema["properties"].get("acom:is_secret").is_some());
    }

    #[test]
    fn a_refused_plan_names_the_first_rule_it_breaks() {
        let plan = CreatePostPlan {
            post_id: "post_x".into(),
            board_id: "brd_events".into(),
            author_id: "cus_jamie".into(),
            author_name: Some("Ja***".into()),
            headline: "Hi".into(),
            article_body: "Hello".into(),
            associated_media: Vec::new(),
            is_secret: false,
            in_language: None,
            status: "published".into(),
            reason: None,
            at: chrono::Utc::now(),
            duplicate_of: None,
            write_refused: Some("Only the store writes here".into()),
            attachment_refused: None,
            blocked_word: None,
            spam_refused: None,
        };
        let e = refusal(&plan).unwrap_or_else(|| panic!("refused"));
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("write_not_allowed", Some("list_boards"))
        );
        let words = CreatePostPlan {
            write_refused: None,
            blocked_word: Some("scam".into()),
            ..plan.clone()
        };
        let e = refusal(&words).unwrap_or_else(|| panic!("refused"));
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("profanity_blocked", Some("create_post"))
        );
        let fine = CreatePostPlan {
            blocked_word: None,
            ..words
        };
        assert!(refusal(&fine).is_none());
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_posts_on_a_member_board_but_not_on_the_store_board() {
        let cx = testing::on_db(testing::shopper()).await;
        let planned = CreatePost
            .preview(&cx, input("brd_style"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            planned.checks.iter().all(|c| c.passed),
            "{:?}",
            planned.checks
        );
        assert_eq!(planned.changes[0].to, json!("published"));
        assert_eq!(planned.plan.author_name.as_deref(), Some("Ja***"));
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = CreatePost
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let row = repo::post(&mut tx, &done.id)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            row.map(|p| p.moderation_status).as_deref(),
            Some("published")
        );
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));

        let staff_only = CreatePost
            .preview(&cx, input("brd_events"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let permission = &staff_only.checks[3];
        assert_eq!(permission.rule, "board.write_permission");
        assert!(!permission.passed);
        assert_eq!(permission.fix_tool.as_deref(), Some("list_boards"));
        let e = CreatePost
            .preview(
                &cx,
                CreatePostInput {
                    is_secret: Some(true),
                    ..input("brd_reviews")
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }
}
