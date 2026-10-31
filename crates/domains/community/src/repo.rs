//! SQL over the `community` schema. Every statement is scoped to the transaction's store
//! (`tenant_id = $1`, and row-level security besides).

use acom_contract::ToolError;
use acom_store::TxCx;
use chrono::{DateTime, Utc};

use crate::db;

// ── board ──────────────────────────────────────────────────────────────────────────────────

/// A board and its settings.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub(crate) struct BoardRow {
    pub id: String,
    pub name: String,
    pub board_type: String,
    pub write_permission: String,
    pub allow_secret: bool,
    pub allow_attachment: bool,
    pub moderation_mode: String,
    pub version: i32,
}

const BOARD: &str = "SELECT id, name, board_type, write_permission, allow_secret, allow_attachment,
       moderation_mode, version
FROM community.board";

pub(crate) async fn board(tx: &mut TxCx, id: &str) -> Result<Option<BoardRow>, ToolError> {
    sqlx::query_as::<_, BoardRow>(sqlx::AssertSqlSafe(format!(
        "{BOARD} WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Boards by name, optionally of one type, with the total.
pub(crate) async fn boards(
    tx: &mut TxCx,
    board_type: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<BoardRow>, i64), ToolError> {
    let rows = sqlx::query_as::<_, BoardRow>(sqlx::AssertSqlSafe(format!(
        "{BOARD} WHERE tenant_id = $1 AND ($2::text IS NULL OR board_type = $2)
         ORDER BY name, id LIMIT $3 OFFSET $4"
    )))
    .bind(tx.tenant.as_str())
    .bind(board_type)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM community.board
         WHERE tenant_id = $1 AND ($2::text IS NULL OR board_type = $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(board_type)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((rows, total))
}

/// Another board with this name (ignoring case and outer spaces), if any.
pub(crate) async fn board_named(
    tx: &mut TxCx,
    name: &str,
    except: Option<&str>,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM community.board
         WHERE tenant_id = $1 AND lower(btrim(name)) = lower(btrim($2))
           AND ($3::text IS NULL OR id <> $3)
         ORDER BY id LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(name)
    .bind(except)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) async fn insert_board(tx: &mut TxCx, b: &BoardRow) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO community.board (tenant_id, id, name, board_type, write_permission,
           allow_secret, allow_attachment, moderation_mode)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&b.id)
    .bind(&b.name)
    .bind(&b.board_type)
    .bind(&b.write_permission)
    .bind(b.allow_secret)
    .bind(b.allow_attachment)
    .bind(&b.moderation_mode)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Writes a board's settings; the rows changed.
pub(crate) async fn update_board(tx: &mut TxCx, b: &BoardRow) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE community.board SET name = $3, board_type = $4, write_permission = $5,
           allow_secret = $6, allow_attachment = $7, moderation_mode = $8,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(&b.id)
    .bind(&b.name)
    .bind(&b.board_type)
    .bind(&b.write_permission)
    .bind(b.allow_secret)
    .bind(b.allow_attachment)
    .bind(&b.moderation_mode)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

/// Posts on a board, of any state.
pub(crate) async fn posts_on(tx: &mut TxCx, board_id: &str) -> Result<i64, ToolError> {
    sqlx::query_scalar(
        "SELECT count(*) FROM community.discussion_forum_posting
         WHERE tenant_id = $1 AND board_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(board_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Removes a board nobody has posted on; the rows removed.
pub(crate) async fn delete_empty_board(tx: &mut TxCx, id: &str) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "DELETE FROM community.board b
         WHERE b.tenant_id = $1 AND b.id = $2
           AND NOT EXISTS (SELECT 1 FROM community.discussion_forum_posting p
                           WHERE p.tenant_id = b.tenant_id AND p.board_id = b.id)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

// ── post ───────────────────────────────────────────────────────────────────────────────────

/// A post with its reply and report counts.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct PostRow {
    pub id: String,
    pub board_id: String,
    pub author_id: String,
    pub author_name: Option<String>,
    pub headline: String,
    pub article_body: String,
    pub associated_media: Vec<String>,
    pub is_secret: bool,
    pub in_language: Option<String>,
    pub moderation_status: String,
    pub moderation_reason: Option<String>,
    pub like_count: i32,
    pub date_published: DateTime<Utc>,
    pub date_modified: Option<DateTime<Utc>>,
    pub version: i32,
    /// Published replies.
    pub comment_count: i64,
    /// Different shoppers who reported it.
    pub report_count: i64,
}

const POST: &str = "SELECT p.id, p.board_id, p.author_id, p.author_name, p.headline,
       p.article_body, p.associated_media, p.is_secret, p.in_language, p.moderation_status,
       p.moderation_reason, p.like_count, p.date_published, p.date_modified, p.version,
       (SELECT count(*) FROM community.comment c
        WHERE c.tenant_id = p.tenant_id AND c.discussion_forum_posting_id = p.id
          AND c.moderation_status = 'published') AS comment_count,
       (SELECT count(*) FROM community.content_report r
        WHERE r.tenant_id = p.tenant_id AND r.discussion_forum_posting_id = p.id) AS report_count
FROM community.discussion_forum_posting p";

pub(crate) async fn post(tx: &mut TxCx, id: &str) -> Result<Option<PostRow>, ToolError> {
    sqlx::query_as::<_, PostRow>(sqlx::AssertSqlSafe(format!(
        "{POST} WHERE p.tenant_id = $1 AND p.id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// What `list_posts` asks for, after its inputs were read.
#[derive(Debug, Clone, Default)]
pub(crate) struct PostFilter {
    pub board_id: String,
    /// The store sees every post.
    pub operator: bool,
    /// The caller: their own posts show in any state but deleted.
    pub viewer: String,
    /// `moderation.blind.keep_replies_on_delete`: a deleted post with others' replies keeps
    /// its place.
    pub keep_replies: bool,
    pub author_id: Option<String>,
    /// An ILIKE pattern, escaped.
    pub keyword: Option<String>,
    pub has_reply: Option<bool>,
    pub reported: Option<bool>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub popular: bool,
    pub limit: i64,
    pub offset: i64,
}

/// Who sees which post in a list (docs/api/community.md, `list_posts`): the store all of them;
/// the writer their own unless deleted; everyone else published posts that are not secret, and
/// the place of a deleted post others replied to.
const VISIBLE_POSTS: &str = "p.tenant_id = $1 AND p.board_id = $2
  AND ($3
       OR (p.author_id = $4 AND p.moderation_status <> 'deleted')
       OR (NOT p.is_secret AND (
             p.moderation_status = 'published'
             OR (p.moderation_status = 'deleted' AND $5 AND EXISTS (
                   SELECT 1 FROM community.comment c
                   WHERE c.tenant_id = p.tenant_id AND c.discussion_forum_posting_id = p.id
                     AND c.author_id <> p.author_id AND c.moderation_status = 'published')))))
  AND ($6::text IS NULL OR p.author_id = $6)
  AND ($7::text IS NULL OR ((p.moderation_status IN ('published', 'held_for_review') OR $3)
                            AND (p.headline ILIKE $7 OR p.article_body ILIKE $7)))
  AND ($8::boolean IS NULL OR $8 = EXISTS (
        SELECT 1 FROM community.comment c
        WHERE c.tenant_id = p.tenant_id AND c.discussion_forum_posting_id = p.id
          AND c.moderation_status = 'published'))
  AND ($9::boolean IS NULL OR $9 = EXISTS (
        SELECT 1 FROM community.content_report r
        WHERE r.tenant_id = p.tenant_id AND r.discussion_forum_posting_id = p.id))
  AND ($10::timestamptz IS NULL OR p.date_published >= $10)
  AND ($11::timestamptz IS NULL OR p.date_published <= $11)";

/// One page of a board's posts the caller may see, with the total.
pub(crate) async fn posts(tx: &mut TxCx, f: &PostFilter) -> Result<(Vec<PostRow>, i64), ToolError> {
    let order = if f.popular {
        "p.like_count DESC, comment_count DESC, p.date_published DESC, p.id DESC"
    } else {
        "p.date_published DESC, p.id DESC"
    };
    let rows = sqlx::query_as::<_, PostRow>(sqlx::AssertSqlSafe(format!(
        "{POST} WHERE {VISIBLE_POSTS} ORDER BY {order} LIMIT $12 OFFSET $13"
    )))
    .bind(tx.tenant.as_str())
    .bind(&f.board_id)
    .bind(f.operator)
    .bind(&f.viewer)
    .bind(f.keep_replies)
    .bind(f.author_id.as_deref())
    .bind(f.keyword.as_deref())
    .bind(f.has_reply)
    .bind(f.reported)
    .bind(f.from)
    .bind(f.to)
    .bind(f.limit)
    .bind(f.offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM community.discussion_forum_posting p WHERE {VISIBLE_POSTS}"
    )))
    .bind(tx.tenant.as_str())
    .bind(&f.board_id)
    .bind(f.operator)
    .bind(&f.viewer)
    .bind(f.keep_replies)
    .bind(f.author_id.as_deref())
    .bind(f.keyword.as_deref())
    .bind(f.has_reply)
    .bind(f.reported)
    .bind(f.from)
    .bind(f.to)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((rows, total))
}

/// Whether someone other than the post's writer has a published reply on it.
pub(crate) async fn others_replied(
    tx: &mut TxCx,
    post_id: &str,
    author_id: &str,
) -> Result<i64, ToolError> {
    sqlx::query_scalar(
        "SELECT count(*) FROM community.comment
         WHERE tenant_id = $1 AND discussion_forum_posting_id = $2 AND author_id <> $3
           AND moderation_status = 'published'",
    )
    .bind(tx.tenant.as_str())
    .bind(post_id)
    .bind(author_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Posts and replies a writer made since `since`.
pub(crate) async fn writes_since(
    tx: &mut TxCx,
    author_id: &str,
    since: DateTime<Utc>,
) -> Result<i64, ToolError> {
    sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM community.discussion_forum_posting
                 WHERE tenant_id = $1 AND author_id = $2 AND date_published >= $3)
              + (SELECT count(*) FROM community.comment
                 WHERE tenant_id = $1 AND author_id = $2 AND date_created >= $3)",
    )
    .bind(tx.tenant.as_str())
    .bind(author_id)
    .bind(since)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A post the same writer put on the same board with the same headline since `since`.
pub(crate) async fn same_post_since(
    tx: &mut TxCx,
    author_id: &str,
    board_id: &str,
    headline: &str,
    since: DateTime<Utc>,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM community.discussion_forum_posting
         WHERE tenant_id = $1 AND author_id = $2 AND board_id = $3
           AND lower(btrim(headline)) = lower(btrim($4)) AND date_published >= $5
           AND moderation_status <> 'deleted'
         ORDER BY date_published, id LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(author_id)
    .bind(board_id)
    .bind(headline)
    .bind(since)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A new post (`create_post`).
#[derive(Debug, Clone)]
pub(crate) struct NewPost<'a> {
    pub id: &'a str,
    pub board_id: &'a str,
    pub author_id: &'a str,
    pub author_name: Option<&'a str>,
    pub headline: &'a str,
    pub article_body: &'a str,
    pub associated_media: &'a [String],
    pub is_secret: bool,
    pub in_language: Option<&'a str>,
    pub status: &'a str,
    pub reason: Option<&'a str>,
    pub at: DateTime<Utc>,
}

pub(crate) async fn insert_post(tx: &mut TxCx, p: &NewPost<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO community.discussion_forum_posting (tenant_id, id, board_id, author_id,
           author_name, headline, article_body, associated_media, is_secret, in_language,
           moderation_status, moderation_reason, date_published)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         ON CONFLICT DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(p.id)
    .bind(p.board_id)
    .bind(p.author_id)
    .bind(p.author_name)
    .bind(p.headline)
    .bind(p.article_body)
    .bind(p.associated_media)
    .bind(p.is_secret)
    .bind(p.in_language)
    .bind(p.status)
    .bind(p.reason)
    .bind(p.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// What `update_post` writes.
#[derive(Debug, Clone)]
pub(crate) struct PostEdit<'a> {
    pub id: &'a str,
    pub author_id: &'a str,
    pub headline: &'a str,
    pub article_body: &'a str,
    pub is_secret: bool,
    pub status: &'a str,
    pub reason: Option<&'a str>,
    pub at: DateTime<Utc>,
}

/// Rewrites a post, only for its writer and only while it is not blinded or deleted.
pub(crate) async fn edit_post(tx: &mut TxCx, e: &PostEdit<'_>) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE community.discussion_forum_posting SET headline = $4, article_body = $5,
           is_secret = $6, moderation_status = $7, moderation_reason = $8, date_modified = $9,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND author_id = $3
           AND moderation_status IN ('published', 'held_for_review')",
    )
    .bind(tx.tenant.as_str())
    .bind(e.id)
    .bind(e.author_id)
    .bind(e.headline)
    .bind(e.article_body)
    .bind(e.is_secret)
    .bind(e.status)
    .bind(e.reason)
    .bind(e.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

/// Moves a post from one moderation state to another with its reason; `0` when it was not in
/// `from` any more.
pub(crate) async fn move_post(
    tx: &mut TxCx,
    id: &str,
    from: &str,
    to: &str,
    reason: Option<&str>,
) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE community.discussion_forum_posting SET moderation_status = $4,
           moderation_reason = $5, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND moderation_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(from)
    .bind(to)
    .bind(reason)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

// ── reply ──────────────────────────────────────────────────────────────────────────────────

/// A reply.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct ReplyRow {
    pub id: String,
    pub post_id: String,
    pub parent_item_id: Option<String>,
    pub reply_depth: i32,
    pub author_id: String,
    pub author_name: Option<String>,
    pub text: String,
    pub is_secret: bool,
    pub moderation_status: String,
    pub moderation_reason: Option<String>,
    pub date_created: DateTime<Utc>,
    pub date_modified: Option<DateTime<Utc>>,
    pub version: i32,
}

const REPLY: &str = "SELECT id, discussion_forum_posting_id AS post_id, parent_item_id,
       reply_depth, author_id, author_name, text, is_secret, moderation_status,
       moderation_reason, date_created, date_modified, version
FROM community.comment";

pub(crate) async fn reply(tx: &mut TxCx, id: &str) -> Result<Option<ReplyRow>, ToolError> {
    sqlx::query_as::<_, ReplyRow>(sqlx::AssertSqlSafe(format!(
        "{REPLY} WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Every reply of a post, oldest first.
pub(crate) async fn replies_of(tx: &mut TxCx, post_id: &str) -> Result<Vec<ReplyRow>, ToolError> {
    sqlx::query_as::<_, ReplyRow>(sqlx::AssertSqlSafe(format!(
        "{REPLY} WHERE tenant_id = $1 AND discussion_forum_posting_id = $2
         ORDER BY date_created, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(post_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A reply the same writer put on the same post with the same text since `since`.
pub(crate) async fn same_reply_since(
    tx: &mut TxCx,
    author_id: &str,
    post_id: &str,
    text: &str,
    since: DateTime<Utc>,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM community.comment
         WHERE tenant_id = $1 AND author_id = $2 AND discussion_forum_posting_id = $3
           AND btrim(text) = btrim($4) AND date_created >= $5
           AND moderation_status <> 'deleted'
         ORDER BY date_created, id LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(author_id)
    .bind(post_id)
    .bind(text)
    .bind(since)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A new reply (`create_reply`).
#[derive(Debug, Clone)]
pub(crate) struct NewReply<'a> {
    pub id: &'a str,
    pub post_id: &'a str,
    pub parent_item_id: Option<&'a str>,
    pub depth: i32,
    pub author_id: &'a str,
    pub author_name: Option<&'a str>,
    pub text: &'a str,
    pub is_secret: bool,
    pub status: &'a str,
    pub reason: Option<&'a str>,
    pub at: DateTime<Utc>,
}

pub(crate) async fn insert_reply(tx: &mut TxCx, r: &NewReply<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO community.comment (tenant_id, id, discussion_forum_posting_id,
           parent_item_id, reply_depth, author_id, author_name, text, is_secret,
           moderation_status, moderation_reason, date_created)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(r.id)
    .bind(r.post_id)
    .bind(r.parent_item_id)
    .bind(r.depth)
    .bind(r.author_id)
    .bind(r.author_name)
    .bind(r.text)
    .bind(r.is_secret)
    .bind(r.status)
    .bind(r.reason)
    .bind(r.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// What `update_reply` writes.
#[derive(Debug, Clone)]
pub(crate) struct ReplyEdit<'a> {
    pub id: &'a str,
    pub author_id: &'a str,
    pub text: &'a str,
    pub is_secret: bool,
    pub status: &'a str,
    pub reason: Option<&'a str>,
    pub at: DateTime<Utc>,
}

/// Rewrites a reply, only for its writer and only while it is not blinded or deleted.
pub(crate) async fn edit_reply(tx: &mut TxCx, e: &ReplyEdit<'_>) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE community.comment SET text = $4, is_secret = $5, moderation_status = $6,
           moderation_reason = $7, date_modified = $8, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND author_id = $3
           AND moderation_status IN ('published', 'held_for_review')",
    )
    .bind(tx.tenant.as_str())
    .bind(e.id)
    .bind(e.author_id)
    .bind(e.text)
    .bind(e.is_secret)
    .bind(e.status)
    .bind(e.reason)
    .bind(e.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

/// Moves a reply from one moderation state to another; `0` when it was not in `from` any more.
pub(crate) async fn move_reply(
    tx: &mut TxCx,
    id: &str,
    from: &str,
    to: &str,
    reason: Option<&str>,
) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE community.comment SET moderation_status = $4, moderation_reason = $5,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND moderation_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(from)
    .bind(to)
    .bind(reason)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

// ── report ─────────────────────────────────────────────────────────────────────────────────

/// The report a shopper already made on a post, if any.
pub(crate) async fn report_by(
    tx: &mut TxCx,
    post_id: &str,
    customer_id: &str,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM community.content_report
         WHERE tenant_id = $1 AND discussion_forum_posting_id = $2 AND customer_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(post_id)
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A new report (`report_post`).
#[derive(Debug, Clone)]
pub(crate) struct NewReport<'a> {
    pub id: &'a str,
    pub post_id: &'a str,
    pub customer_id: &'a str,
    pub reason: &'a str,
    pub description: Option<&'a str>,
    pub at: DateTime<Utc>,
}

/// Inserts the report unless this shopper already reported the post; `true` when written.
pub(crate) async fn insert_report(tx: &mut TxCx, r: &NewReport<'_>) -> Result<bool, ToolError> {
    let done = sqlx::query(
        "INSERT INTO community.content_report (tenant_id, id, discussion_forum_posting_id,
           customer_id, report_reason, description, start_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(r.id)
    .bind(r.post_id)
    .bind(r.customer_id)
    .bind(r.reason)
    .bind(r.description)
    .bind(r.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(done.rows_affected() == 1)
}

/// Different shoppers who reported a post.
pub(crate) async fn reporters(tx: &mut TxCx, post_id: &str) -> Result<i64, ToolError> {
    sqlx::query_scalar(
        "SELECT count(DISTINCT customer_id) FROM community.content_report
         WHERE tenant_id = $1 AND discussion_forum_posting_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(post_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// `%keyword%` for ILIKE, with the pattern characters in the keyword taken literally.
pub(crate) fn like_pattern(keyword: &str) -> String {
    let escaped = keyword
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_match_literally() {
        assert_eq!(like_pattern("50% off_now"), "%50\\% off\\_now%");
    }
}
