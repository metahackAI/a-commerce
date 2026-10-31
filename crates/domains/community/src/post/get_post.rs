//! `get_post` (docs/api/community.md#post): one post with its text, attachments and counts.
//! A secret post opens only for its writer and the store; a blinded or deleted one shows its
//! place and state but not its text. Reading has no side effects (no view counter is kept).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::{open_post, post_entity};
use crate::{POST_NOT_FOUND, SECRET_POST, Viewer};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPostInput {
    /// The post (`list_posts`).
    #[schemars(regex(pattern = r"^post_[a-z0-9_]+$"), example = &"post_8812")]
    pub post_id: String,
    /// Only these top-level fields of the post (`id` always comes back); all of them when left
    /// out.
    #[schemars(example = ["headline", "article_body", "comment_count"])]
    pub fields: Option<Vec<String>>,
}

/// One post.
#[derive(Debug)]
pub struct GetPost;

impl Operation for GetPost {
    const META: OpMeta = OpMeta {
        address: "community.post.get_post",
        tool: "get_post",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/community/posts/{post_id}",
        summary: "One community post: headline, text, attachments, masked writer, reply and report counts, and its moderation state. Secret posts open only for their writer and the store.",
        consumes: &[Slot::one("post_id")],
        produces: &[Slot::one("post")],
        checks: &["moderation.author_masking"],
        errors: &[SECRET_POST, POST_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPostInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetPost {
    async fn read(&self, cx: &Cx, input: GetPostInput) -> Result<Value, ToolError> {
        let viewer = Viewer::of(&cx.principal);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let p = open_post(&mut tx, &viewer, &input.post_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        acom_contract::fields::keep(
            post_entity(&p, &viewer, &cx.time),
            &input.fields.unwrap_or_default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;
    use serde_json::json;

    #[test]
    fn the_input_schema_is_closed() {
        let schema = acom_contract::schema::closed_schema_for::<GetPostInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(schema["required"], json!(["post_id"]));
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn a_secret_post_opens_for_its_writer_and_the_store_only() {
        // post_8810 is Elena's secret question to the store.
        let grace = testing::as_subject(&testing::on_db(testing::shopper()).await, "cus_grace");
        let e = GetPost
            .read(
                &grace,
                GetPostInput {
                    post_id: "post_8810".into(),
                    fields: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("secret_post", Some("list_posts"))
        );
        let elena = testing::as_subject(&grace, "cus_elena");
        let own = GetPost
            .read(
                &elena,
                GetPostInput {
                    post_id: "post_8810".into(),
                    fields: Some(vec!["headline".into(), "acom:is_secret".into()]),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(own["acom:is_secret"], json!(true));
        // `id` always comes back with the fields asked for.
        assert_eq!(own.as_object().map(|o| o.len()), Some(3));
        let e = GetPost
            .read(
                &elena,
                GetPostInput {
                    post_id: "post_8810".into(),
                    fields: Some(vec!["title".into()]),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        let store = testing::on_db(testing::operator()).await;
        let seen = GetPost
            .read(
                &store,
                GetPostInput {
                    post_id: "post_8812".into(),
                    fields: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(seen["author"], "Ja***");
        assert_eq!(seen["comment_count"], json!(3));
        assert_eq!(seen["acom:moderation_status"], "published");
    }
}
