//! `list_replies` (docs/api/community.md#reply): a post's replies as a thread, with the total.
//! Writers' names are masked; blinded replies are left out for shoppers; secret replies show only
//! to their writer, the post's writer and the store.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::{reply_item, thread};
use crate::post::open_post;
use crate::{POST_NOT_FOUND, Viewer, list, post_not_found, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListRepliesInput {
    /// The post (`list_posts`).
    #[schemars(regex(pattern = r"^post_[a-z0-9_]+$"), example = &"post_8812")]
    pub post_id: String,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// A post's replies.
#[derive(Debug)]
pub struct ListReplies;

impl Operation for ListReplies {
    const META: OpMeta = OpMeta {
        address: "community.reply.list_replies",
        tool: "list_replies",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/community/posts/{post_id}/replies",
        summary: "The replies on a community post as a thread (each reply, then the replies to it), with the total. Writers' names are masked; blinded replies are left out; secret replies show only to the people in the conversation and the store.",
        consumes: &[Slot::one("post_id")],
        produces: &[Slot::many("reply_id")],
        checks: &["moderation.author_masking"],
        errors: &[POST_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListRepliesInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListReplies {
    async fn read(&self, cx: &Cx, input: ListRepliesInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let viewer = Viewer::of(&cx.principal);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        // A post the caller cannot open has no replies they can see.
        let post = open_post(&mut tx, &viewer, &input.post_id)
            .await
            .map_err(|e| {
                if e.code == crate::SECRET_POST.code {
                    post_not_found(&input.post_id)
                } else {
                    e
                }
            })?;
        let replies = repo::replies_of(&mut tx, &post.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let shown = thread(&viewer, &post.author_id, &replies);
        let total = i64::try_from(shown.len()).unwrap_or(i64::MAX);
        let items = shown
            .iter()
            .skip(usize::try_from(page.offset).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit).unwrap_or(0))
            .map(|(r, text)| reply_item(r, *text, &cx.time))
            .collect();
        Ok(list(items, total, &page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;
    use serde_json::json;

    #[test]
    fn the_input_schema_is_closed() {
        let schema = acom_contract::schema::closed_schema_for::<ListRepliesInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn replies_read_as_a_thread_with_masked_names() {
        let owen = testing::as_subject(&testing::on_db(testing::shopper()).await, "cus_owen");
        let out = ListReplies
            .read(
                &owen,
                ListRepliesInput {
                    post_id: "post_8812".into(),
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["total"], json!(3));
        let ids: Vec<&str> = out["items"]
            .as_array()
            .map(|a| a.iter().filter_map(|i| i["id"].as_str()).collect())
            .unwrap_or_default();
        assert_eq!(ids, ["rpl_5521", "rpl_5523", "rpl_5522"]);
        assert_eq!(out["items"][0]["author"], "Gr***");
        assert_eq!(out["items"][1]["parent_item"], "rpl_5521");
        assert_eq!(out["items"][2]["author"], "The store");
        let e = ListReplies
            .read(
                &owen,
                ListRepliesInput {
                    post_id: "post_8810".into(),
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("post_not_found", Some("list_posts"))
        );
    }
}
