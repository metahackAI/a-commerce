//! `list_posts` (docs/api/community.md#post): the posts on a board, with the total. Shoppers see
//! published posts, secret ones only when they wrote them, and their own posts while held for
//! review; a deleted post that others replied to keeps its place without its text
//! (`moderation.blind.keep_replies_on_delete`). The store sees every post and may filter by
//! reports. Writers' names are masked (`moderation.author_masking`).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::post_item;
use crate::repo::{self, PostFilter, like_pattern};
use crate::rules::BlindRules;
use crate::{BOARD_NOT_FOUND, INVALID_FILTER, Viewer, board_not_found, list, trimmed};

/// Whose posts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PostScope {
    /// Every post the caller may see.
    #[default]
    All,
    /// Only the caller's own posts.
    Mine,
}

/// In which order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PostSort {
    /// Newest first.
    #[default]
    Newest,
    /// Most liked first, then most replied.
    Popular,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListPostsInput {
    /// The board (`list_boards`).
    #[schemars(regex(pattern = r"^brd_[a-z0-9_]+$"), example = &"brd_reviews")]
    pub board_id: String,
    /// Only posts by this shopper.
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_jamie")]
    pub author_id: Option<String>,
    /// Words in the headline or the text.
    #[schemars(length(min = 1, max = 100), example = &"size up")]
    pub keyword: Option<String>,
    /// true: only posts with a reply; false: only posts nobody answered yet.
    #[schemars(example = false)]
    pub has_reply: Option<bool>,
    /// true: only reported posts; false: only posts nobody reported. The store only.
    #[schemars(example = true)]
    pub reported: Option<bool>,
    /// Written at or after this time (store time when no offset is given).
    #[schemars(example = &"2026-11-01T00:00:00-05:00")]
    pub date_published_from: Option<String>,
    /// Written at or before this time (store time when no offset is given).
    #[schemars(example = &"2026-11-06T23:59:00-05:00")]
    pub date_published_to: Option<String>,
    /// all: every post the caller may see; mine: only the caller's own. Default all.
    #[schemars(example = &"all")]
    pub scope: Option<PostScope>,
    /// newest: newest first; popular: most liked, then most replied. Default newest.
    #[schemars(example = &"newest")]
    pub sort: Option<PostSort>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// A board's posts.
#[derive(Debug)]
pub struct ListPosts;

impl Operation for ListPosts {
    const META: OpMeta = OpMeta {
        address: "community.post.list_posts",
        tool: "list_posts",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Post,
        path: "/v1/community/posts/search",
        summary: "The posts on a community board with the total, newest or most popular first: by writer, words, replied or not, reported (store only), dates, or only the caller's own. Writers' names are masked; secret posts show only to their writer and the store.",
        consumes: &[Slot::one("board_id")],
        produces: &[Slot::many("post_id")],
        checks: &["moderation.author_masking"],
        errors: &[BOARD_NOT_FOUND, INVALID_FILTER],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListPostsInput;
    type Output = Value;
}

fn bad_filter(field: &str, message: String) -> ToolError {
    ToolError::new(INVALID_FILTER, message)
        .detail(json!({"field": field, "reason": "format"}))
        .next("list_posts")
}

impl ReadOp<Cx> for ListPosts {
    async fn read(&self, cx: &Cx, input: ListPostsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let viewer = Viewer::of(&cx.principal);
        if input.reported.is_some() && !viewer.operator {
            return Err(ToolError::new(
                INVALID_FILTER,
                "Only the store filters posts by reports; leave out `reported`.",
            )
            .detail(json!({"field": "reported", "reason": "role"}))
            .next("list_posts"));
        }
        let time = |field: &str, s: Option<&str>| {
            s.map(|s| {
                cx.time
                    .parse(s)
                    .map_err(|e| bad_filter(field, format!("`{field}` is not a time ({e}).")))
            })
            .transpose()
        };
        let from = time("date_published_from", input.date_published_from.as_deref())?;
        let to = time("date_published_to", input.date_published_to.as_deref())?;
        if let (Some(a), Some(b)) = (from, to)
            && a > b
        {
            return Err(bad_filter(
                "date_published_to",
                "`date_published_to` is before `date_published_from`.".into(),
            ));
        }
        let mine = input.scope.unwrap_or_default() == PostScope::Mine;
        let author_id = match (mine, input.author_id) {
            (true, Some(a)) if a != viewer.subject => {
                return Err(bad_filter(
                    "author_id",
                    "`scope: mine` lists your own posts; leave out `author_id` or use `scope: all`."
                        .into(),
                ));
            }
            (true, _) => Some(viewer.subject.clone()),
            (false, a) => a,
        };
        let blind = BlindRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if repo::board(&mut tx, &input.board_id).await?.is_none() {
            return Err(board_not_found(&input.board_id));
        }
        let (rows, total) = repo::posts(
            &mut tx,
            &PostFilter {
                board_id: input.board_id,
                operator: viewer.operator,
                viewer: viewer.subject.clone(),
                keep_replies: blind.keep_replies_on_delete,
                author_id,
                keyword: trimmed(input.keyword.as_deref()).map(|k| like_pattern(&k)),
                has_reply: input.has_reply,
                reported: input.reported,
                from,
                to,
                popular: input.sort.unwrap_or_default() == PostSort::Popular,
                limit: page.limit,
                offset: page.offset,
            },
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(list(
            rows.iter()
                .map(|p| post_item(p, &viewer, &cx.time))
                .collect(),
            total,
            &page,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    fn input(board: &str) -> ListPostsInput {
        ListPostsInput {
            board_id: board.into(),
            author_id: None,
            keyword: None,
            has_reply: None,
            reported: None,
            date_published_from: None,
            date_published_to: None,
            scope: None,
            sort: None,
            limit: None,
            cursor: None,
        }
    }

    #[tokio::test]
    async fn shoppers_cannot_filter_by_reports() {
        let e = ListPosts
            .read(
                &testing::shopper(),
                ListPostsInput {
                    reported: Some(true),
                    ..input("brd_style")
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("invalid_filter", Some("list_posts"))
        );
        let e = ListPosts
            .read(
                &testing::shopper(),
                ListPostsInput {
                    date_published_from: Some("last week".into()),
                    ..input("brd_style")
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_filter");
        let schema = acom_contract::schema::closed_schema_for::<ListPostsInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            schema["properties"]["sort"]["enum"],
            json!(["newest", "popular"])
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn boards_list_what_each_viewer_may_see() {
        let jamie = testing::on_db(testing::shopper()).await;
        let reviews = ListPosts
            .read(&jamie, input("brd_reviews"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(reviews["total"], json!(2));
        assert_eq!(reviews["items"][0]["id"], "post_8812");
        assert_eq!(reviews["items"][0]["author"], "Ja***");
        assert_eq!(reviews["items"][0]["comment_count"], json!(3));
        assert!(reviews["items"][0].get("acom:report_count").is_none());
        // Elena's secret question is hers and the store's only.
        let questions = ListPosts
            .read(&jamie, input("brd_questions"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let store = testing::on_db(testing::operator()).await;
        let all = ListPosts
            .read(&store, input("brd_questions"))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            all["total"].as_i64().map(|n| n - 1),
            questions["total"].as_i64()
        );
        let reported = ListPosts
            .read(
                &store,
                ListPostsInput {
                    reported: Some(true),
                    ..input("brd_style")
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(reported["items"][0]["id"], "post_8811");
        assert_eq!(reported["items"][0]["acom:report_count"], json!(3));
        let mine = ListPosts
            .read(
                &jamie,
                ListPostsInput {
                    scope: Some(PostScope::Mine),
                    keyword: Some("pocket".into()),
                    ..input("brd_reviews")
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(mine["total"], json!(1));
        assert_eq!(mine["items"][0]["id"], "post_8790");
        let e = ListPosts
            .read(&jamie, input("brd_nowhere"))
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("board_not_found", Some("list_boards"))
        );
    }
}
