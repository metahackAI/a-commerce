//! The `post` resource (docs/api/community.md#post): what its operations share. A post is a
//! schema.org `DiscussionForumPosting` on a board. Shoppers write, change and delete their own;
//! anyone signed in may report one; the store blinds or deletes it with a reason.

pub mod create_post;
pub mod delete_post;
pub mod get_post;
pub mod list_posts;
pub mod report_post;
pub mod update_post;

use acom_contract::ToolError;
use acom_core::StoreTime;
use acom_store::TxCx;
use serde_json::{Value, json};

use crate::model::{PostAccess, post_access, shown_author};
use crate::repo::{self, PostRow};
use crate::{SECRET_POST, Viewer, post_not_found};

/// A post as a list item. The text of a blinded or deleted post is not shown, only its place.
pub(crate) fn post_item(p: &PostRow, viewer: &Viewer, time: &StoreTime) -> Value {
    let full = post_access(viewer, &p.author_id, &p.moderation_status, p.is_secret)
        == PostAccess::Full;
    let mut item = json!({
        "id": p.id,
        "headline": if full { json!(p.headline) } else { Value::Null },
        "author": shown_author(&p.author_id, p.author_name.as_deref()),
        "date_published": time.format(p.date_published),
        "comment_count": p.comment_count,
        "acom:like_count": p.like_count,
        "acom:is_secret": p.is_secret,
        "acom:moderation_status": p.moderation_status,
    });
    if viewer.operator {
        item["acom:report_count"] = json!(p.report_count);
    }
    item
}

/// A post as `get_post` returns it: the entity itself, no list envelope.
pub(crate) fn post_entity(p: &PostRow, viewer: &Viewer, time: &StoreTime) -> Value {
    let full = post_access(viewer, &p.author_id, &p.moderation_status, p.is_secret)
        == PostAccess::Full;
    let mut out = json!({
        "id": p.id,
        "board_id": p.board_id,
        "headline": if full { json!(p.headline) } else { Value::Null },
        "article_body": if full { json!(p.article_body) } else { Value::Null },
        "associated_media": if full { json!(p.associated_media) } else { json!([]) },
        "author": shown_author(&p.author_id, p.author_name.as_deref()),
        "date_published": time.format(p.date_published),
        "date_modified": p.date_modified.map(|t| time.format(t)),
        "in_language": p.in_language,
        "comment_count": p.comment_count,
        "acom:like_count": p.like_count,
        "acom:is_secret": p.is_secret,
        "acom:moderation_status": p.moderation_status,
        "acom:report_count": p.report_count,
    });
    if viewer.operator {
        out["acom:moderation_reason"] = json!(p.moderation_reason);
    }
    out
}

/// The post, when the viewer may open it: `post_not_found` for a post that does not exist or is
/// still held for review by someone else, `secret_post` for someone else's secret post.
pub(crate) async fn open_post(
    tx: &mut TxCx,
    viewer: &Viewer,
    id: &str,
) -> Result<PostRow, ToolError> {
    let p = repo::post(tx, id)
        .await?
        .ok_or_else(|| post_not_found(id))?;
    match post_access(viewer, &p.author_id, &p.moderation_status, p.is_secret) {
        PostAccess::Hidden => Err(post_not_found(id)),
        PostAccess::Secret => Err(ToolError::new(
            SECRET_POST,
            format!("{id} is a secret post; only its writer and the store can read it."),
        )
        .detail(json!({"reason": "owner", "entity": "post", "id": id}))
        .next("list_posts")),
        PostAccess::Full | PostAccess::TextHidden => Ok(p),
    }
}
